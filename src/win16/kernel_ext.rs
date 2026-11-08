//! KERNEL: recursos NE, arquivos DOS (_lopen...), módulos e outras APIs menores.

use super::api::{Call, Impl};
use super::kernel::{global_alloc, global_info};
use super::ne::{ResKey, ResType};
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::Mutex;

fn pack(ax: u16, dx: u16) -> u32 {
    ax as u32 | (dx as u32) << 16
}

// ---- recursos -----------------------------------------------------------------------------

static RES: Mutex<Option<(Vec<ResType>, Vec<u8>)>> = Mutex::new(None);
/// Recursos localizados por FindResource: handle = índice + 1 -> (offset, tamanho).
static FOUND: Mutex<Vec<(u32, u32)>> = Mutex::new(Vec::new());
/// Handle de FindResource -> bloco global carregado por LoadResource.
static LOADED: Mutex<BTreeMap<u16, u16>> = Mutex::new(BTreeMap::new());

pub fn set_resources(types: Vec<ResType>, data: Vec<u8>) {
    *RES.lock().unwrap() = Some((types, data));
}

/// `seg:off` de um parâmetro LPCSTR -> chave de recurso (inteiro se o seletor é 0; "#n" vale inteiro).
pub fn key_of_far(raw: u32) -> ResKey {
    let (off, sel) = (raw as u16, (raw >> 16) as u16);
    if sel == 0 {
        return ResKey::Id(off);
    }
    let lin = super::api::far_to_linear(raw);
    if lin == 0 {
        return ResKey::Id(0);
    }
    let s = unsafe { std::ffi::CStr::from_ptr(lin as usize as *const i8) }.to_string_lossy().to_uppercase();
    match s.strip_prefix('#').and_then(|n| n.parse().ok()) {
        Some(n) => ResKey::Id(n),
        None => ResKey::Name(s),
    }
}

fn key_eq(a: &ResKey, b: &ResKey) -> bool {
    match (a, b) {
        (ResKey::Id(x), ResKey::Id(y)) => x == y,
        (ResKey::Name(x), ResKey::Name(y)) => x.eq_ignore_ascii_case(y),
        _ => false,
    }
}

pub fn find_bytes(ty: &ResKey, name: &ResKey) -> Option<(u32, u32)> {
    let g = RES.lock().unwrap();
    let (types, _) = g.as_ref()?;
    let t = types.iter().find(|t| key_eq(&t.key, ty))?;
    t.entries.iter().find(|e| key_eq(&e.key, name)).map(|e| (e.off, e.len))
}

/// Bytes de um recurso NE.
pub fn resource_bytes(ty: &ResKey, name: &ResKey) -> Option<Vec<u8>> {
    let (off, len) = find_bytes(ty, name)?;
    let g = RES.lock().unwrap();
    let (_, data) = g.as_ref()?;
    data.get(off as usize..(off + len) as usize).map(|b| b.to_vec())
}

fn find_resource(c: &mut Call) -> u32 {
    let (name, ty) = (super::kernel_ext::key_of_far(c.l(1)), super::kernel_ext::key_of_far(c.l(2)));
    match find_bytes(&ty, &name) {
        Some(r) => {
            let mut f = FOUND.lock().unwrap();
            f.push(r);
            f.len() as u32
        }
        None => 0,
    }
}

fn load_resource(c: &mut Call) -> u32 {
    let h = c.w(1);
    if let Some(&sel) = LOADED.lock().unwrap().get(&h) {
        return sel as u32;
    }
    let Some((off, len)) = FOUND.lock().unwrap().get(h as usize - 1).copied() else { return 0 };
    let Some(sel) = global_alloc(len) else { return 0 };
    let (base, _) = global_info(sel).unwrap();
    let g = RES.lock().unwrap();
    if let Some((_, data)) = g.as_ref() {
        let src = &data[off as usize..(off + len) as usize];
        unsafe { std::ptr::copy_nonoverlapping(src.as_ptr(), base as usize as *mut u8, src.len()) };
    }
    drop(g);
    LOADED.lock().unwrap().insert(h, sel);
    sel as u32
}

fn lock_resource(c: &mut Call) -> u32 {
    pack(0, c.w(0)) // seletor:0
}

fn sizeof_resource(c: &mut Call) -> u32 {
    FOUND.lock().unwrap().get((c.w(1) as usize).wrapping_sub(1)).map_or(0, |r| r.1)
}

fn free_resource(_c: &mut Call) -> u32 {
    0
}

// ---- arquivos DOS -------------------------------------------------------------------------

static FILES: Mutex<BTreeMap<u16, File>> = Mutex::new(BTreeMap::new());
const HFILE_ERROR: u32 = 0xFFFF;

pub(super) fn open_file(path: &str, create: bool, mode: u16) -> u32 {
    let p = crate::win32::files::host_path(path);
    if std::env::var_os("WINLAYER_WIN16_TRACE").is_some() {
        eprintln!("[win16] abrir {path:?} -> {p:?} (criar={create})");
    }
    let mut o = OpenOptions::new();
    if create {
        o.read(true).write(true).create(true).truncate(true);
    } else {
        match mode & 3 {
            0 => o.read(true),
            1 => o.write(true),
            _ => o.read(true).write(true),
        };
    }
    match o.open(&p) {
        Ok(f) => {
            let mut t = FILES.lock().unwrap();
            let h = (5..u16::MAX).find(|h| !t.contains_key(h)).unwrap_or(5);
            t.insert(h, f);
            h as u32
        }
        Err(_) => HFILE_ERROR,
    }
}

fn path_arg(c: &Call, i: usize) -> String {
    let p = c.ptr(i);
    if p == 0 { String::new() } else { unsafe { std::ffi::CStr::from_ptr(p as usize as *const i8) }.to_string_lossy().into_owned() }
}

fn lopen(c: &mut Call) -> u32 {
    open_file(&path_arg(c, 0), false, c.w(1))
}

fn lcreat(c: &mut Call) -> u32 {
    open_file(&path_arg(c, 0), true, 0)
}

/// INT 21h: fecha/posiciona arquivos pelos mesmos handles de `_lopen`.
pub(super) fn dos_close(h: u16) -> bool {
    FILES.lock().unwrap().remove(&h).is_some()
}

pub(super) fn dos_seek(h: u16, off: i32, origin: u8) -> Option<u32> {
    let mut t = FILES.lock().unwrap();
    let f = t.get_mut(&h)?;
    let from = match origin {
        0 => SeekFrom::Start(off.max(0) as u64),
        1 => SeekFrom::Current(off as i64),
        _ => SeekFrom::End(off as i64),
    };
    f.seek(from).ok().map(|p| p as u32)
}

pub(super) fn dos_truncate_here(h: u16) -> bool {
    let mut t = FILES.lock().unwrap();
    match t.get_mut(&h) {
        Some(f) => f.stream_position().and_then(|p| f.set_len(p)).is_ok(),
        None => false,
    }
}

fn lclose(c: &mut Call) -> u32 {
    FILES.lock().unwrap().remove(&c.w(0));
    0
}

pub(super) fn read_into(h: u16, ptr: u32, n: u32) -> u32 {
    let mut t = FILES.lock().unwrap();
    let Some(f) = t.get_mut(&h) else { return HFILE_ERROR };
    let buf = unsafe { std::slice::from_raw_parts_mut(ptr as usize as *mut u8, n as usize) };
    let mut total = 0;
    while total < buf.len() {
        match f.read(&mut buf[total..]) {
            Ok(0) | Err(_) => break,
            Ok(k) => total += k,
        }
    }
    total as u32
}

fn lread(c: &mut Call) -> u32 {
    read_into(c.w(0), c.ptr(1), c.w(2) as u32)
}

fn hread(c: &mut Call) -> u32 {
    read_into(c.w(0), c.ptr(1), c.l(2))
}

pub(super) fn write_from(h: u16, ptr: u32, n: u32) -> u32 {
    let mut t = FILES.lock().unwrap();
    let Some(f) = t.get_mut(&h) else { return HFILE_ERROR };
    let buf = unsafe { std::slice::from_raw_parts(ptr as usize as *const u8, n as usize) };
    if f.write_all(buf).is_ok() { n } else { HFILE_ERROR }
}

fn lwrite(c: &mut Call) -> u32 {
    write_from(c.w(0), c.ptr(1), c.w(2) as u32)
}

fn hwrite(c: &mut Call) -> u32 {
    write_from(c.w(0), c.ptr(1), c.l(2))
}

fn llseek(c: &mut Call) -> u32 {
    let (h, off, origin) = (c.w(0), c.l(1) as i32, c.w(2));
    let mut t = FILES.lock().unwrap();
    let Some(f) = t.get_mut(&h) else { return u32::MAX };
    let from = match origin {
        0 => SeekFrom::Start(off.max(0) as u64),
        1 => SeekFrom::Current(off as i64),
        _ => SeekFrom::End(off as i64),
    };
    f.seek(from).map_or(u32::MAX, |p| p as u32) // DX:AX = nova posição
}

// ---- módulos ------------------------------------------------------------------------------

fn load_library(c: &mut Call) -> u32 {
    let name = path_arg(c, 0).to_uppercase();
    let stem = name.rsplit(['\\', '/']).next().unwrap_or("").trim_end_matches(".DLL").to_string();
    match stem.as_str() {
        "COMMDLG" | "SHELL" | "VER" | "MMSYSTEM" | "TOOLHELP" | "LZEXPAND" | "KEYBOARD" | "SOUND" => 0x0100 + (stem.len() as u32), // handle simbólico (>= 32)
        _ => 2, // ERROR_FILE_NOT_FOUND
    }
}

// ---- MMSYSTEM / COMMDLG / WIN87EM ---------------------------------------------------------

/// sndPlaySound(lpszSound, wFlags): o som sai pelo visualizador (ver `audio.rs`). Chamadas síncronas
/// (SND_SYNC) não bloqueiam; o jogo continua enquanto o navegador toca.
fn snd_play_sound(c: &mut Call) -> u32 {
    let (name, flags) = (c.ptr(0), c.w(1) as u32);
    unsafe { crate::win32::winmm::play_sound(name, flags) }
}

/// OPENFILENAME de 16 bits: lStructSize(4) hwndOwner(2) hInstance(2) lpstrFilter(4) lpstrCustomFilter(4)
/// nMaxCustFilter(4) nFilterIndex(4) lpstrFile(4) nMaxFile(4) lpstrFileTitle(4) nMaxFileTitle(4)
/// lpstrInitialDir(4) lpstrTitle(4) Flags(4) nFileOffset(2) nFileExtension(2) lpstrDefExt(4)
fn get_file_name(c: &mut Call, save: bool) -> u32 {
    let ofn = c.ptr(0);
    if ofn == 0 {
        return 0;
    }
    unsafe {
        let rd32 = |o: u32| std::ptr::read_unaligned((ofn + o) as usize as *const u32);
        let far = |o: u32| super::api::far_to_linear(rd32(o));
        let cstr = |p: u32| -> String {
            if p == 0 { String::new() } else { std::ffi::CStr::from_ptr(p as usize as *const i8).to_string_lossy().into_owned() }
        };
        let (file, max_file) = (far(24), rd32(28) as usize);
        if file == 0 || max_file < 2 {
            return 0;
        }
        // extensão do filtro escolhido: pares "descrição\0padrão\0" terminados por \0\0
        let mut ext = String::new();
        let filter = far(8);
        if filter != 0 {
            let mut p = filter;
            let mut parts = Vec::new();
            loop {
                let s = cstr(p);
                if s.is_empty() {
                    break;
                }
                p += s.len() as u32 + 1;
                parts.push(s);
            }
            let idx = (rd32(20) as usize).max(1);
            if let Some(pat) = parts.get(idx * 2 - 1) {
                ext = pat.clone();
            }
        }
        let title = { let t = cstr(far(44)); if t.is_empty() { (if save { "Save As" } else { "Open" }).to_string() } else { t } };
        let default_name = cstr(file);
        let Some(mut name) = crate::win32::user32::file_dialog(c.w(0) as u32 & 0, &title, &cstr(far(40)), &ext, &default_name, save) else { return 0 };
        let def_ext = cstr(far(56));
        if !name.contains('.') && !def_ext.is_empty() {
            name = format!("{name}.{def_ext}");
        }
        let full = if name.contains(':') || name.contains('\\') { name.clone() } else { format!("C:\\{name}") };
        let n = full.len().min(max_file - 1);
        std::ptr::copy_nonoverlapping(full.as_ptr(), file as usize as *mut u8, n);
        *((file + n as u32) as usize as *mut u8) = 0;
        let off = full.rfind('\\').map_or(0, |i| i + 1);
        let ext_off = full.rfind('.').map_or(0, |i| i + 1);
        std::ptr::write_unaligned((ofn + 52) as usize as *mut u16, off as u16);
        std::ptr::write_unaligned((ofn + 54) as usize as *mut u16, ext_off as u16);
        let (title_buf, title_max) = (far(32), rd32(36) as usize);
        if title_buf != 0 && title_max > 1 {
            let fname = &full[off..];
            let m = fname.len().min(title_max - 1);
            std::ptr::copy_nonoverlapping(fname.as_ptr(), title_buf as usize as *mut u8, m);
            *((title_buf + m as u32) as usize as *mut u8) = 0;
        }
    }
    1
}

fn get_open_file_name(c: &mut Call) -> u32 {
    get_file_name(c, false)
}

fn get_save_file_name(c: &mut Call) -> u32 {
    get_file_name(c, true)
}

fn zero(_c: &mut Call) -> u32 {
    0
}

fn one(_c: &mut Call) -> u32 {
    1
}

pub static IMPLS: &[Impl] = &[
    Impl { module: "KERNEL", name: "FINDRESOURCE", sig: "wll", f: find_resource },
    Impl { module: "KERNEL", name: "LOADRESOURCE", sig: "ww", f: load_resource },
    Impl { module: "KERNEL", name: "LOCKRESOURCE", sig: "w", f: lock_resource },
    Impl { module: "KERNEL", name: "SIZEOFRESOURCE", sig: "ww", f: sizeof_resource },
    Impl { module: "KERNEL", name: "FREERESOURCE", sig: "w", f: free_resource },
    Impl { module: "KERNEL", name: "_LOPEN", sig: "pw", f: lopen },
    Impl { module: "KERNEL", name: "_LCREAT", sig: "pw", f: lcreat },
    Impl { module: "KERNEL", name: "_LCLOSE", sig: "w", f: lclose },
    Impl { module: "KERNEL", name: "_LREAD", sig: "wpw", f: lread },
    Impl { module: "KERNEL", name: "_LWRITE", sig: "wpw", f: lwrite },
    Impl { module: "KERNEL", name: "_LLSEEK", sig: "wlw", f: llseek },
    Impl { module: "KERNEL", name: "_HREAD", sig: "wpl", f: hread },
    Impl { module: "KERNEL", name: "_HWRITE", sig: "wpl", f: hwrite },
    Impl { module: "KERNEL", name: "LOADLIBRARY", sig: "p", f: load_library },
    Impl { module: "MMSYSTEM", name: "SNDPLAYSOUND", sig: "pw", f: snd_play_sound },
    Impl { module: "MMSYSTEM", name: "MCISENDCOMMAND", sig: "wwll", f: zero },
    Impl { module: "COMMDLG", name: "ORD1", sig: "p", f: get_open_file_name },
    Impl { module: "COMMDLG", name: "ORD2", sig: "p", f: get_save_file_name },
    Impl { module: "COMMDLG", name: "ORD26", sig: "", f: zero },
    Impl { module: "WIN87EM", name: "ORD1", sig: "", f: one },   // __FPMATH: há coprocessador real
];
