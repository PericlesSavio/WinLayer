//! kernel32: variantes wide (W), console e miscelânea exigidas pelo runtime C (msvcrt).

use super::files;
use std::ffi::CString;

pub fn resolve(name: &str) -> Option<usize> {
    exports!(
        name,
        CreateProcessA, CreateProcessW, CreatePipe, GetCurrentProcess, DuplicateHandle, GetExitCodeProcess,
        CreateFileW, DeleteFileW, GetFileAttributesW, FindFirstFileW, FindNextFileW, GetFullPathNameW,
        GetCurrentDirectoryW, SetCurrentDirectoryW, CreateDirectoryW, RemoveDirectoryA, RemoveDirectoryW,
        MoveFileA, MoveFileW, SetFileAttributesA, SetFileAttributesW, SetEnvironmentVariableW,
        GetDriveTypeW, GetModuleFileNameW, GetUserDefaultLCID, LockFile, UnlockFile, SetLocalTime,
        GetDiskFreeSpaceA, GetNumberOfConsoleInputEvents, PeekConsoleInputA, ReadConsoleInputA, ReadConsoleA,
        WriteConsoleA, GetConsoleMode, SetConsoleMode, SetConsoleCtrlHandler, SystemTimeToFileTime,
        LocalFileTimeToFileTime, IsValidCodePage, GetFileInformationByHandle, Beep, Sleep, IsBadCodePtr,
        IsBadWritePtr, IsBadReadPtr, SetUnhandledExceptionFilter, SetFileTime,
    )
}

fn set_error(e: u32) {
    super::kernel32::set_last_error(e);
}

/// UTF-16 terminado em NUL -> CString (Latin-1: unidades > 0xFF viram '?').
unsafe fn narrow(p: *const u16) -> CString {
    let mut v = Vec::new();
    let mut i = 0;
    while *p.add(i) != 0 {
        let c = *p.add(i);
        v.push(if c < 0x100 { c as u8 } else { b'?' });
        i += 1;
    }
    CString::new(v).unwrap_or_default()
}

unsafe fn widen_into(src: &[u8], dst: *mut u16, max: u32) -> u32 {
    if dst.is_null() || max == 0 {
        return src.len() as u32 + 1;
    }
    let n = src.len().min(max as usize - 1);
    for i in 0..n {
        *dst.add(i) = src[i] as u16;
    }
    *dst.add(n) = 0;
    n as u32
}

// ---- processos e handles ------------------------------------------------------------------

#[allow(non_snake_case)]
extern "stdcall" fn CreateProcessA(_a: u32, _b: u32, _c: u32, _d: u32, _e: i32, _f: u32, _g: u32, _h: u32, _i: u32, _j: u32) -> i32 {
    set_error(2);
    0
}

#[allow(non_snake_case)]
extern "stdcall" fn CreateProcessW(a: u32, b: u32, c: u32, d: u32, e: i32, f: u32, g: u32, h: u32, i: u32, j: u32) -> i32 {
    CreateProcessA(a, b, c, d, e, f, g, h, i, j)
}

#[allow(non_snake_case)]
extern "stdcall" fn CreatePipe(_r: u32, _w: u32, _attr: u32, _size: u32) -> i32 {
    0
}

#[allow(non_snake_case)]
extern "stdcall" fn GetCurrentProcess() -> u32 {
    u32::MAX // pseudo-handle
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn DuplicateHandle(_sp: u32, h: u32, _tp: u32, out: *mut u32, _acc: u32, _inh: i32, _opt: u32) -> i32 {
    if !out.is_null() {
        *out = h;
    }
    1
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn GetExitCodeProcess(_h: u32, code: *mut u32) -> i32 {
    *code = 0;
    1
}

// ---- arquivos (W) -------------------------------------------------------------------------

#[allow(non_snake_case)]
unsafe extern "stdcall" fn CreateFileW(name: *const u16, access: u32, share: u32, sec: u32, disp: u32, flags: u32, tmpl: u32) -> u32 {
    files::CreateFileA(narrow(name).as_ptr(), access, share, sec, disp, flags, tmpl)
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn DeleteFileW(name: *const u16) -> i32 {
    std::fs::remove_file(files::host_path(&narrow(name).to_string_lossy())).is_ok() as i32
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn GetFileAttributesW(name: *const u16) -> u32 {
    files::GetFileAttributesA(narrow(name).as_ptr())
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn FindFirstFileW(pattern: *const u16, out: *mut u16) -> u32 {
    // WIN32_FIND_DATAW: 44 bytes de cabeçalho iguais; cFileName[260] em UTF-16 a partir de 44.
    let mut tmp = [0u8; 320];
    let h = files::FindFirstFileA(narrow(pattern).as_ptr(), tmp.as_mut_ptr());
    if h != files::INVALID_HANDLE {
        convert_find(&tmp, out);
    }
    h
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn FindNextFileW(h: u32, out: *mut u16) -> i32 {
    let mut tmp = [0u8; 320];
    let ok = files::FindNextFileA(h, tmp.as_mut_ptr());
    if ok != 0 {
        convert_find(&tmp, out);
    }
    ok
}

unsafe fn convert_find(a: &[u8; 320], w: *mut u16) {
    std::ptr::copy_nonoverlapping(a.as_ptr(), w as *mut u8, 44);
    let name = &a[44..304];
    let n = name.iter().position(|&b| b == 0).unwrap_or(260);
    for i in 0..260 {
        *(w as *mut u8).add(44 + i * 2).cast::<u16>() = if i < n { name[i] as u16 } else { 0 };
    }
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn GetFullPathNameW(name: *const u16, max: u32, buf: *mut u16, filepart: *mut u32) -> u32 {
    let s = narrow(name);
    let n = widen_into(s.to_bytes(), buf, max);
    if !filepart.is_null() && !buf.is_null() {
        let off = s.to_bytes().iter().rposition(|&c| c == b'\\' || c == b'/').map_or(0, |i| i + 1);
        *filepart = buf as u32 + off as u32 * 2;
    }
    n
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn GetCurrentDirectoryW(max: u32, buf: *mut u16) -> u32 {
    let mut tmp = [0u8; 512];
    let n = files::GetCurrentDirectoryA(512, tmp.as_mut_ptr());
    widen_into(&tmp[..n as usize], buf, max)
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn SetCurrentDirectoryW(name: *const u16) -> i32 {
    files::SetCurrentDirectoryA(narrow(name).as_ptr())
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn CreateDirectoryW(name: *const u16, sec: u32) -> i32 {
    files::CreateDirectoryA(narrow(name).as_ptr(), sec)
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn RemoveDirectoryA(name: *const i8) -> i32 {
    std::fs::remove_dir(files::host_path(&std::ffi::CStr::from_ptr(name).to_string_lossy())).is_ok() as i32
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn RemoveDirectoryW(name: *const u16) -> i32 {
    RemoveDirectoryA(narrow(name).as_ptr())
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn MoveFileA(from: *const i8, to: *const i8) -> i32 {
    let f = |p: *const i8| files::host_path(&std::ffi::CStr::from_ptr(p).to_string_lossy());
    std::fs::rename(f(from), f(to)).is_ok() as i32
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn MoveFileW(from: *const u16, to: *const u16) -> i32 {
    MoveFileA(narrow(from).as_ptr(), narrow(to).as_ptr())
}

#[allow(non_snake_case)]
extern "stdcall" fn SetFileAttributesA(_name: u32, _attr: u32) -> i32 {
    1
}

#[allow(non_snake_case)]
extern "stdcall" fn SetFileAttributesW(_name: u32, _attr: u32) -> i32 {
    1
}

#[allow(non_snake_case)]
extern "stdcall" fn SetEnvironmentVariableW(_n: u32, _v: u32) -> i32 {
    1
}

#[allow(non_snake_case)]
extern "stdcall" fn GetDriveTypeW(_root: u32) -> u32 {
    3
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn GetModuleFileNameW(module: u32, buf: *mut u16, max: u32) -> u32 {
    let mut tmp = [0u8; 260];
    let n = super::kernel32::get_module_file_name(module, tmp.as_mut_ptr(), 260);
    widen_into(&tmp[..n as usize], buf, max)
}

#[allow(non_snake_case)]
extern "stdcall" fn GetUserDefaultLCID() -> u32 {
    0x0409
}

#[allow(non_snake_case)]
extern "stdcall" fn LockFile(_h: u32, _a: u32, _b: u32, _c: u32, _d: u32) -> i32 {
    1
}

#[allow(non_snake_case)]
extern "stdcall" fn UnlockFile(_h: u32, _a: u32, _b: u32, _c: u32, _d: u32) -> i32 {
    1
}

#[allow(non_snake_case)]
extern "stdcall" fn SetLocalTime(_st: u32) -> i32 {
    1
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn GetDiskFreeSpaceA(_root: u32, spc: *mut u32, bps: *mut u32, free: *mut u32, total: *mut u32) -> i32 {
    *spc = 8;
    *bps = 512;
    *free = 100_000;
    *total = 200_000;
    1
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn GetFileInformationByHandle(h: u32, info: *mut u32) -> i32 {
    let Some((len, dir)) = files::file_info(h) else { return 0 };
    // BY_HANDLE_FILE_INFORMATION (52 bytes)
    std::ptr::write_bytes(info as *mut u8, 0, 52);
    *info = if dir { 0x10 } else { 0x80 };
    *info.add(8) = (len >> 32) as u32; // nFileSizeHigh
    *info.add(9) = len as u32;         // nFileSizeLow
    *info.add(11) = 1;                 // nNumberOfLinks
    1
}

#[allow(non_snake_case)]
extern "stdcall" fn SetFileTime(_h: u32, _a: u32, _b: u32, _c: u32) -> i32 {
    1
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn SystemTimeToFileTime(st: *const u16, ft: *mut u64) -> i32 {
    let (y, m, d) = (*st as i64, *st.add(1) as i64, *st.add(3) as i64);
    let (h, mi, s, ms) = (*st.add(4) as i64, *st.add(5) as i64, *st.add(6) as i64, *st.add(7) as i64);
    // data civil -> dias desde 1970 (Howard Hinnant)
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2.rem_euclid(400);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    let secs = days * 86400 + h * 3600 + mi * 60 + s;
    *ft = ((secs + 11_644_473_600) as u64) * 10_000_000 + ms as u64 * 10_000;
    1
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn LocalFileTimeToFileTime(src: *const u64, dst: *mut u64) -> i32 {
    *dst = *src;
    1
}

// ---- console (não há console: tudo falha ou é ignorado) -----------------------------------

#[allow(non_snake_case)]
extern "stdcall" fn GetNumberOfConsoleInputEvents(_h: u32, _n: u32) -> i32 { 0 }
#[allow(non_snake_case)]
extern "stdcall" fn PeekConsoleInputA(_h: u32, _b: u32, _n: u32, _r: u32) -> i32 { 0 }
#[allow(non_snake_case)]
extern "stdcall" fn ReadConsoleInputA(_h: u32, _b: u32, _n: u32, _r: u32) -> i32 { 0 }
#[allow(non_snake_case)]
extern "stdcall" fn ReadConsoleA(_h: u32, _b: u32, _n: u32, _r: u32, _x: u32) -> i32 { 0 }
#[allow(non_snake_case)]
extern "stdcall" fn GetConsoleMode(_h: u32, _m: u32) -> i32 { 0 }
#[allow(non_snake_case)]
extern "stdcall" fn SetConsoleMode(_h: u32, _m: u32) -> i32 { 0 }
#[allow(non_snake_case)]
extern "stdcall" fn SetConsoleCtrlHandler(_p: u32, _a: i32) -> i32 { 1 }

#[allow(non_snake_case)]
unsafe extern "stdcall" fn WriteConsoleA(h: u32, buf: *const u8, n: u32, written: *mut u32, _r: u32) -> i32 {
    super::kernel32::write_file_raw(h as usize, buf, n, written)
}

// ---- miscelânea ---------------------------------------------------------------------------

#[allow(non_snake_case)]
extern "stdcall" fn IsValidCodePage(_cp: u32) -> i32 { 1 }
#[allow(non_snake_case)]
extern "stdcall" fn Beep(_f: u32, _d: u32) -> i32 { 1 }
#[allow(non_snake_case)]
extern "stdcall" fn IsBadCodePtr(_p: u32) -> i32 { 0 }
#[allow(non_snake_case)]
extern "stdcall" fn IsBadWritePtr(_p: u32, _n: u32) -> i32 { 0 }
#[allow(non_snake_case)]
extern "stdcall" fn IsBadReadPtr(_p: u32, _n: u32) -> i32 { 0 }
#[allow(non_snake_case)]
extern "stdcall" fn SetUnhandledExceptionFilter(_f: u32) -> u32 { 0 }

#[allow(non_snake_case)]
extern "stdcall" fn Sleep(ms: u32) {
    // No modo captura (sem tela) não faz sentido esperar de verdade.
    let ms = if std::env::var_os("WINLAYER_SHOT").is_some() { ms.min(5) } else { ms };
    std::thread::sleep(std::time::Duration::from_millis(ms as u64));
}
