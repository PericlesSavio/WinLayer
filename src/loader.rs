//! Mapeia imagens PE32 (exe e DLLs) na memória do processo.

use crate::pe::{ImportName, Pe32, IMAGE_FILE_DLL};
use crate::win32::{self, modules};
use std::ffi::c_void;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};

const PROT_READ: i32 = 1;
const PROT_WRITE: i32 = 2;
const PROT_EXEC: i32 = 4;
const MAP_PRIVATE: i32 = 0x02;
const MAP_ANONYMOUS: i32 = 0x20;
const MAP_FIXED_NOREPLACE: i32 = 0x10_0000;

extern "C" {
    fn mmap(addr: *mut c_void, len: usize, prot: i32, flags: i32, fd: i32, off: i64) -> *mut c_void;
}

/// Com `--lazy`, imports não implementados viram armadilhas em vez de impedir a execução.
static LAZY: AtomicBool = AtomicBool::new(false);

pub fn set_lazy(v: bool) {
    LAZY.store(v, Ordering::Relaxed);
}

#[derive(Debug)]
pub enum LoadError {
    IsDll,
    NotDll,
    Map(&'static str),
    NeedsRelocation,
    Missing(Vec<String>),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::IsDll => write!(f, "DLL não pode ser executada diretamente"),
            LoadError::NotDll => write!(f, "não é uma DLL"),
            LoadError::Map(s) => write!(f, "falha ao mapear a imagem: {s}"),
            LoadError::NeedsRelocation => write!(f, "imagem sem relocações não coube no endereço base"),
            LoadError::Missing(v) => write!(f, "imports não implementados (use --lazy para ignorar):\n  {}", v.join("\n  ")),
        }
    }
}

pub struct Loaded {
    entry: usize,
}

impl Loaded {
    /// # Safety
    /// Executa código arbitrário do `.exe`.
    pub unsafe fn run(&self) -> u32 {
        // Entry point de um .exe Win32: stdcall, sem argumentos, retorna o exit code.
        let f: extern "stdcall" fn() -> u32 = std::mem::transmute(self.entry);
        let code = f();
        win32::kernel32::exit_process(code)
    }
}

/// Mapeia a imagem no endereço preferido ou, se ocupado e houver relocações, em outro lugar.
/// Copia cabeçalhos e seções e aplica as relocações. Devolve o endereço real.
pub fn map(pe: &Pe32, file: &[u8]) -> Result<u32, LoadError> {
    // Mapeia em RWX por simplicidade; endurecer por seção depois.
    let prot = PROT_READ | PROT_WRITE | PROT_EXEC;
    let flags = MAP_PRIVATE | MAP_ANONYMOUS;
    let size = pe.size_of_image as usize;
    let mut base = unsafe { mmap(pe.image_base as usize as *mut c_void, size, prot, flags | MAP_FIXED_NOREPLACE, -1, 0) };
    if base as usize != pe.image_base as usize {
        if pe.reloc_dir.0 == 0 {
            return Err(LoadError::NeedsRelocation);
        }
        base = unsafe { mmap(std::ptr::null_mut(), size, prot, flags, -1, 0) };
    }
    if base as isize == -1 {
        return Err(LoadError::Map("mmap falhou"));
    }
    let base = base as *mut u8;

    let hdr = (pe.size_of_headers as usize).min(file.len());
    unsafe { std::ptr::copy_nonoverlapping(file.as_ptr(), base, hdr) };
    for s in &pe.sections {
        let n = (s.raw_size as usize).min(file.len().saturating_sub(s.raw_offset as usize));
        unsafe {
            std::ptr::copy_nonoverlapping(file.as_ptr().add(s.raw_offset as usize), base.add(s.virtual_address as usize), n);
        }
    }

    let delta = (base as usize as u32).wrapping_sub(pe.image_base);
    if delta != 0 {
        apply_relocations(pe, base, delta);
    }
    Ok(base as usize as u32)
}

fn apply_relocations(pe: &Pe32, base: *mut u8, delta: u32) {
    let (mut rva, size) = pe.reloc_dir;
    let end = rva + size;
    unsafe {
        while rva + 8 <= end {
            let page = *(base.add(rva as usize) as *const u32);
            let block = *(base.add(rva as usize + 4) as *const u32);
            if block < 8 { break }
            for i in 0..(block - 8) / 2 {
                let e = *(base.add((rva + 8 + i * 2) as usize) as *const u16);
                if e >> 12 == 3 {
                    // IMAGE_REL_BASED_HIGHLOW
                    let p = base.add((page + (e & 0xFFF) as u32) as usize) as *mut u32;
                    *p = (*p).wrapping_add(delta);
                }
            }
            rva += block;
        }
    }
}

// ---- imports ------------------------------------------------------------------------------

/// Nomes das funções que faltam, na ordem em que foram criadas as armadilhas.
static TRAP_NAMES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
static TRAP_PAGE: std::sync::Mutex<(usize, usize)> = std::sync::Mutex::new((0, 0)); // (endereço, usados)

static API_NAMES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

#[repr(C)]
struct IoVec {
    base: *mut c_void,
    len: usize,
}

extern "C" {
    fn process_vm_readv(pid: i32, local: *const IoVec, lcnt: u64, remote: *const IoVec, rcnt: u64, flags: u64) -> isize;
}

/// Lê até `n` bytes de um endereço qualquer sem arriscar SIGSEGV (via process_vm_readv no próprio processo).
pub(crate) fn peek(addr: u32, n: usize) -> Vec<u8> {
    if addr < 0x10000 {
        return Vec::new();
    }
    let mut buf = vec![0u8; n];
    let local = IoVec { base: buf.as_mut_ptr() as *mut c_void, len: n };
    let remote = IoVec { base: addr as usize as *mut c_void, len: n };
    let got = unsafe { process_vm_readv(std::process::id() as i32, &local, 1, &remote, 1, 0) };
    buf.truncate(got.max(0) as usize);
    buf
}

extern "C" fn log_api(idx: u32, args: *const u32) {
    let name = API_NAMES.lock().unwrap().get(idx as usize).cloned().unwrap_or_default();
    let mut line = format!("[api] {name}(");
    for i in 0..4 {
        let a = unsafe { *args.add(i) };
        if i > 0 {
            line.push_str(", ");
        }
        line.push_str(&format!("{a:#x}"));
        let bytes = peek(a, 40);
        let text: String = bytes.iter().take_while(|&&c| (32..127).contains(&c)).map(|&c| c as char).collect();
        if text.len() >= 3 {
            line.push_str(&format!(" \"{text}\""));
        }
    }
    line.push(')');
    eprintln!("{line}");
}

/// Stub de rastreio: preserva registradores e flags, loga o nome e salta para a função real.
fn make_trace_thunk(label: String, target: u32) -> u32 {
    let mut names = API_NAMES.lock().unwrap();
    let idx = names.len() as u32;
    names.push(label);
    drop(names);
    let mut page = TRACE_PAGE.lock().unwrap();
    if page.0 == 0 || page.1 + 48 > 4096 {
        let p = unsafe { mmap(std::ptr::null_mut(), 4096, PROT_READ | PROT_WRITE | PROT_EXEC, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0) };
        *page = (p as usize, 0);
    }
    let at = page.0 + page.1;
    page.1 += 48;
    // pushfd; pushad; mov eax,esp; add eax,40 (args); push eax; push idx; mov eax,log; call eax;
    // add esp,8; popad; popfd; push target; ret
    let mut code = vec![0x9C, 0x60, 0x89, 0xE0, 0x83, 0xC0, 0x28, 0x50, 0x68];
    code.extend_from_slice(&idx.to_le_bytes());
    code.push(0xB8);
    code.extend_from_slice(&(log_api as usize as u32).to_le_bytes());
    code.extend_from_slice(&[0xFF, 0xD0, 0x83, 0xC4, 0x08, 0x61, 0x9D, 0x68]);
    code.extend_from_slice(&target.to_le_bytes());
    code.push(0xC3);
    unsafe { std::ptr::copy_nonoverlapping(code.as_ptr(), at as *mut u8, code.len()) };
    at as u32
}

static TRACE_PAGE: std::sync::Mutex<(usize, usize)> = std::sync::Mutex::new((0, 0));

extern "C" fn missing_import(idx: u32) -> ! {
    let name = TRAP_NAMES.lock().unwrap().get(idx as usize).cloned().unwrap_or_default();
    eprintln!("\n[winlayer] import NÃO IMPLEMENTADO chamado: {name}");
    win32::kernel32::exit_process(3)
}

/// Gera um stub `push idx; mov eax, handler; call eax` que reporta a função chamada.
fn make_trap(name: String) -> u32 {
    let mut names = TRAP_NAMES.lock().unwrap();
    let idx = names.len() as u32;
    names.push(name);
    drop(names);
    let mut page = TRAP_PAGE.lock().unwrap();
    if page.0 == 0 || page.1 + 16 > 4096 {
        let p = unsafe { mmap(std::ptr::null_mut(), 4096, PROT_READ | PROT_WRITE | PROT_EXEC, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0) };
        *page = (p as usize, 0);
    }
    let at = page.0 + page.1;
    page.1 += 16;
    let mut code = vec![0x68];
    code.extend_from_slice(&idx.to_le_bytes());
    code.push(0xB8);
    code.extend_from_slice(&(missing_import as usize as u32).to_le_bytes());
    code.extend_from_slice(&[0xFF, 0xD0]);
    unsafe { std::ptr::copy_nonoverlapping(code.as_ptr(), at as *mut u8, code.len()) };
    at as u32
}

/// Escreve na IAT o endereço de cada import. Devolve os que não foram resolvidos
/// (em modo lazy, já com armadilha instalada).
pub fn fill_imports(pe: &Pe32, base: u32) -> Vec<String> {
    let lazy = LAZY.load(Ordering::Relaxed);
    let trace = std::env::var_os("WINLAYER_API_TRACE").is_some();
    let mut missing = Vec::new();
    for imp in &pe.imports {
        for (i, name) in imp.names.iter().enumerate() {
            let label = match name {
                ImportName::Name(n) => format!("{}!{}", imp.dll, n),
                ImportName::Ordinal(o) => format!("{}!#{}", imp.dll, o),
            };
            let addr = modules::resolve_import(&imp.dll, name).or_else(|| {
                if std::env::var_os("WINLAYER_TRACE").is_some() { eprintln!("[loader] falta {label}") }
                missing.push(label.clone());
                lazy.then(|| make_trap(label.clone()))
            });
            let addr = match addr {
                Some(a) if trace && !label.starts_with("missing") => Some(make_trace_thunk(label.clone(), a)),
                other => other,
            };
            if let Some(a) = addr {
                unsafe { *((base + imp.iat_rva + i as u32 * 4) as usize as *mut u32) = a };
            }
        }
    }
    missing
}

// ---- executável principal -----------------------------------------------------------------

pub fn load(pe: &Pe32, file: &[u8]) -> Result<Loaded, LoadError> {
    if pe.characteristics & IMAGE_FILE_DLL != 0 {
        return Err(LoadError::IsDll);
    }
    let base = map(pe, file)?;
    win32::kernel32::set_image_base(base);
    // O TEB precisa existir antes: DLLs carregadas pelos imports executam DllMain (usa fs:).
    if let Err(e) = win32::teb::install(base) {
        eprintln!("winlayer: TEB: {e}");
        return Err(LoadError::Map("falha ao criar o TEB"));
    }
    let missing = fill_imports(pe, base);
    if !missing.is_empty() {
        if LAZY.load(Ordering::Relaxed) {
            eprintln!("[winlayer] {} imports não implementados (armadilhas instaladas)", missing.len());
        } else {
            return Err(LoadError::Missing(missing));
        }
    }
    Ok(Loaded { entry: base as usize + pe.entry_rva as usize })
}
