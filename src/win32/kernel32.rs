//! kernel32.dll — what the MSVC CRT startup and simple programs need.
//! On i686, `extern "stdcall"` means callee cleanup: the callee cleans the stack.

use std::collections::BTreeMap;
use std::ffi::{c_void, CString};
use std::io::Write;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

// ---- global state of the "Windows process" -----------------------------------------------

static IMAGE_BASE: AtomicU32 = AtomicU32::new(0);
static LAST_ERROR: AtomicU32 = AtomicU32::new(0);
static MODULE_PATH: OnceLock<CString> = OnceLock::new();
static COMMAND_LINE: OnceLock<CString> = OnceLock::new();
/// Regions handed out by VirtualAlloc: start -> size.
static REGIONS: Mutex<BTreeMap<usize, usize>> = Mutex::new(BTreeMap::new());

pub fn image_base() -> u32 {
    IMAGE_BASE.load(Ordering::Relaxed)
}

pub fn set_last_error(e: u32) {
    LAST_ERROR.store(e, Ordering::Relaxed);
}

pub fn command_line() -> &'static std::ffi::CStr {
    COMMAND_LINE.get().map_or(c"", |c| c.as_c_str())
}

/// Region handed out by VirtualAlloc that contains `addr`: (start, size).
pub fn region_of(addr: usize) -> Option<(usize, usize)> {
    let r = REGIONS.lock().unwrap();
    r.range(..=addr).next_back().filter(|(&s, &l)| addr < s + l).map(|(&s, &l)| (s, l))
}

pub fn set_image_base(base: u32) {
    IMAGE_BASE.store(base, Ordering::Relaxed);
}

pub fn init(path: &str, args: &[String]) {
    let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.into());
    let dir = abs.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let name = abs.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    super::files::set_root(dir.clone());
    let _ = std::env::set_current_dir(&dir);
    // As seen by the program, the .exe's directory is C:\
    let win_path = format!("C:\\{name}");
    let _ = MODULE_PATH.set(CString::new(win_path.clone()).unwrap_or_default());
    let mut cmd = format!("\"{win_path}\"");
    for a in args {
        cmd.push(' ');
        cmd.push_str(a);
    }
    let _ = COMMAND_LINE.set(CString::new(cmd).unwrap_or_default());
}

fn set_error(e: u32) {
    LAST_ERROR.store(e, Ordering::Relaxed);
}

// ---- constants ----------------------------------------------------------------------------

const STD_INPUT_HANDLE: u32 = -10i32 as u32;
const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
const STD_ERROR_HANDLE: u32 = -12i32 as u32;

// Fake HANDLEs; only valid for the standard handles for now.
const H_STDIN: usize = 0x10;
const H_STDOUT: usize = 0x11;
const H_STDERR: usize = 0x12;
const INVALID_HANDLE_VALUE: usize = usize::MAX;

const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_NOT_ENOUGH_MEMORY: u32 = 8;
const FILE_TYPE_UNKNOWN: u32 = 0;
const FILE_TYPE_CHAR: u32 = 2;
const MEM_RELEASE: u32 = 0x8000;

/// GetVersion: Windows 95 (4.0, build 950). Bit 31 = plataforma Win9x.
const WIN95_VERSION: u32 = 0x8000_0000 | (950 << 16) | 4;

const PAGE: usize = 4096;

extern "C" {
    fn mmap(addr: *mut c_void, len: usize, prot: i32, flags: i32, fd: i32, off: i64) -> *mut c_void;
    fn munmap(addr: *mut c_void, len: usize) -> i32;
}

// ---- export table -------------------------------------------------------------------------

pub fn resolve(name: &str) -> Option<usize> {
    exports!(
        name,
        GetStdHandle, SetStdHandle, WriteFile, ExitProcess, CloseHandle, FlushFileBuffers,
        GetFileType, GetLastError, GetVersion, GetModuleHandleA,
        GetModuleFileNameA, GetCommandLineA, GetStartupInfoA, GetEnvironmentStrings,
        GetACP, GetOEMCP, GetCPInfo, GetWindowsDirectoryA, WinExec, VirtualAlloc,
        VirtualFree, WideCharToMultiByte, RtlUnwind, UnhandledExceptionFilter, GetProcAddress, LoadLibraryA,
        LocalAlloc, DeleteFileA, FindResourceA, LoadResource, LockResource, SizeofResource,
        GlobalAlloc, GlobalLock, GlobalUnlock, GlobalFree, GlobalReAlloc, LocalFree, FreeResource,
        FreeLibrary, RaiseException, InitializeCriticalSection, EnterCriticalSection,
        LeaveCriticalSection, DeleteCriticalSection, SetEnvironmentVariableA, GetLocalTime, GetTimeZoneInformation, MultiByteToWideChar,
    )
}

pub fn exit_process(code: u32) -> ! {
    crate::diag::set_end(format!("ExitProcess({code})"));
    let _ = std::io::stdout().flush();
    std::process::exit(code as i32)
}

/// Copies `s` + NUL to `buf` (up to `size` bytes); returns how many bytes excluding the NUL.
unsafe fn write_cstr(buf: *mut u8, size: u32, s: &[u8]) -> u32 {
    if buf.is_null() || size == 0 {
        return 0;
    }
    let n = s.len().min(size as usize - 1);
    std::ptr::copy_nonoverlapping(s.as_ptr(), buf, n);
    *buf.add(n) = 0;
    n as u32
}

// ---- process / environment ----------------------------------------------------------------

#[allow(non_snake_case)]
extern "stdcall" fn ExitProcess(code: u32) -> ! {
    exit_process(code)
}

#[allow(non_snake_case)]
extern "stdcall" fn GetVersion() -> u32 {
    WIN95_VERSION
}

#[allow(non_snake_case)]
extern "stdcall" fn GetLastError() -> u32 {
    LAST_ERROR.load(Ordering::Relaxed)
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn GetModuleHandleA(name: *const i8) -> u32 {
    // NULL = the executable itself.
    if name.is_null() {
        return IMAGE_BASE.load(Ordering::Relaxed);
    }
    super::modules::module_handle(&std::ffi::CStr::from_ptr(name).to_string_lossy()).unwrap_or(0)
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn GetModuleFileNameA(module: u32, buf: *mut u8, size: u32) -> u32 {
    get_module_file_name(module, buf, size)
}

pub unsafe fn get_module_file_name(module: u32, buf: *mut u8, size: u32) -> u32 {
    if module == 0 || module == IMAGE_BASE.load(Ordering::Relaxed) {
        // TODO: convert to a DOS-style path (C:\...) once there is drive mapping.
        return write_cstr(buf, size, MODULE_PATH.get().map_or(&[][..], |c| c.to_bytes()));
    }
    match super::modules::module_file_name(module) {
        Some(p) => write_cstr(buf, size, p.as_bytes()),
        None => 0,
    }
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetCommandLineA() -> *const u8 {
    COMMAND_LINE.get().map_or(b"\0".as_ptr(), |c| c.as_ptr() as *const u8)
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn GetStartupInfoA(si: *mut u8) {
    // STARTUPINFOA is 68 bytes; only `cb` is filled in.
    std::ptr::write_bytes(si, 0, 68);
    *(si as *mut u32) = 68;
}

#[allow(non_snake_case)]
extern "stdcall" fn GetEnvironmentStrings() -> *const u8 {
    b"\0\0".as_ptr() // empty environment block
}

#[allow(non_snake_case)]
extern "stdcall" fn GetACP() -> u32 {
    1252
}

#[allow(non_snake_case)]
extern "stdcall" fn GetOEMCP() -> u32 {
    437
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn GetCPInfo(_cp: u32, info: *mut u8) -> i32 {
    // CPINFO: MaxCharSize(4) DefaultChar[2] LeadByte[12]. Single-byte code page.
    std::ptr::write_bytes(info, 0, 18);
    *(info as *mut u32) = 1;
    *info.add(4) = b'?';
    1
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn GetWindowsDirectoryA(buf: *mut u8, size: u32) -> u32 {
    write_cstr(buf, size, b"C:\\WINDOWS")
}

#[allow(non_snake_case)]
extern "stdcall" fn WinExec(_cmd: *const i8, _show: u32) -> u32 {
    2 // <= 31 indicates an error (ERROR_FILE_NOT_FOUND)
}

#[allow(non_snake_case)]
extern "stdcall" fn RtlUnwind(_target: usize, _ip: usize, _record: usize, _ret: u32) {}

#[allow(non_snake_case)]
extern "stdcall" fn UnhandledExceptionFilter(_info: usize) -> i32 {
    1 // EXCEPTION_EXECUTE_HANDLER
}

// ---- handles and files --------------------------------------------------------------------

#[allow(non_snake_case)]
extern "stdcall" fn GetStdHandle(n: u32) -> usize {
    match n {
        STD_INPUT_HANDLE => H_STDIN,
        STD_OUTPUT_HANDLE => H_STDOUT,
        STD_ERROR_HANDLE => H_STDERR,
        _ => INVALID_HANDLE_VALUE,
    }
}

#[allow(non_snake_case)]
extern "stdcall" fn SetStdHandle(_n: u32, _h: usize) -> i32 {
    1
}

#[allow(non_snake_case)]
extern "stdcall" fn CloseHandle(h: usize) -> i32 {
    super::sync::close(h as u32);
    super::files::close(h as u32);
    1
}

#[allow(non_snake_case)]
extern "stdcall" fn FlushFileBuffers(_h: usize) -> i32 {
    1
}

#[allow(non_snake_case)]
extern "stdcall" fn GetFileType(h: usize) -> u32 {
    match h {
        H_STDIN | H_STDOUT | H_STDERR => FILE_TYPE_CHAR,
        h if super::files::is_file(h as u32) => 1, // FILE_TYPE_DISK
        _ => FILE_TYPE_UNKNOWN,
    }
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn WriteFile(
    handle: usize,
    buf: *const u8,
    len: u32,
    written: *mut u32,
    _overlapped: *mut u8,
) -> i32 {
    write_file_raw(handle, buf, len, written)
}

pub unsafe fn write_file_raw(handle: usize, buf: *const u8, len: u32, written: *mut u32) -> i32 {
    let data = std::slice::from_raw_parts(buf, len as usize);
    let ok = match handle {
        H_STDOUT => std::io::stdout().write_all(data).is_ok(),
        H_STDERR => std::io::stderr().write_all(data).is_ok(),
        h => match super::files::write(h as u32, data) {
            Some(n) => {
                if !written.is_null() { *written = n }
                return 1;
            }
            None => false,
        },
    };
    if ok && !written.is_null() {
        *written = len;
    }
    ok as i32
}

// ---- memory -------------------------------------------------------------------------------

#[allow(non_snake_case)]
extern "stdcall" fn VirtualAlloc(addr: usize, size: usize, _kind: u32, _protect: u32) -> usize {
    const PROT_RWX: i32 = 7;
    const MAP_PRIVATE: i32 = 0x02;
    const MAP_ANONYMOUS: i32 = 0x20;
    const MAP_FIXED_NOREPLACE: i32 = 0x10_0000;

    let mut regions = REGIONS.lock().unwrap();
    // Commit inside a region we already reserved: the anonymous mmap is already usable.
    if addr != 0 {
        if let Some((&start, &len)) = regions.range(..=addr).next_back() {
            if addr >= start && addr + size <= start + len {
                return addr;
            }
        }
    }
    let len = (size.max(1) + PAGE - 1) & !(PAGE - 1);
    if addr == 0 {
        // Windows hands out regions on 64 KB boundaries (the allocation granularity), and some allocators
        // rely on it: they find a block's header by rounding a pointer down to 64 KB. Maps 64 KB more and
        // trims both ends.
        const GRAIN: usize = 0x10000;
        let p = unsafe { mmap(std::ptr::null_mut(), len + GRAIN, PROT_RWX, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0) };
        if p as isize == -1 {
            set_error(ERROR_NOT_ENOUGH_MEMORY);
            return 0;
        }
        let raw = p as usize;
        let start = (raw + GRAIN - 1) & !(GRAIN - 1);
        unsafe {
            if start > raw {
                munmap(raw as *mut c_void, start - raw);
            }
            let tail = raw + len + GRAIN - (start + len);
            if tail > 0 {
                munmap((start + len) as *mut c_void, tail);
            }
        }
        regions.insert(start, len);
        return start;
    }
    let p = unsafe { mmap(addr as *mut c_void, len, PROT_RWX, MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED_NOREPLACE, -1, 0) };
    if p as isize == -1 || p as usize != addr {
        set_error(ERROR_NOT_ENOUGH_MEMORY);
        return 0;
    }
    regions.insert(p as usize, len);
    p as usize
}

#[allow(non_snake_case)]
extern "stdcall" fn VirtualFree(addr: usize, _size: usize, kind: u32) -> i32 {
    if kind & MEM_RELEASE != 0 {
        if let Some(len) = REGIONS.lock().unwrap().remove(&addr) {
            unsafe { munmap(addr as *mut c_void, len) };
        }
    }
    1 // MEM_DECOMMIT: no effect for now
}

// ---- text ---------------------------------------------------------------------------------

#[allow(non_snake_case)]
unsafe extern "stdcall" fn WideCharToMultiByte(
    _cp: u32,
    _flags: u32,
    wide: *const u16,
    wlen: i32,
    out: *mut u8,
    outlen: i32,
    _default: *const i8,
    _used_default: *mut i32,
) -> i32 {
    // Simplified conversion: each UTF-16 unit becomes one byte (Latin-1; '?' otherwise).
    let n = if wlen < 0 {
        let mut i = 0;
        while *wide.add(i) != 0 { i += 1; }
        i + 1 // includes the NUL
    } else {
        wlen as usize
    };
    if outlen == 0 {
        return n as i32;
    }
    if (outlen as usize) < n {
        return 0; // ERROR_INSUFFICIENT_BUFFER
    }
    for i in 0..n {
        let c = *wide.add(i);
        *out.add(i) = if c < 0x100 { c as u8 } else { b'?' };
    }
    n as i32
}

// ---- modules, local memory, files ---------------------------------------------------------

const ERROR_MOD_NOT_FOUND: u32 = 126;
const ERROR_PROC_NOT_FOUND: u32 = 127;

extern "C" {
    fn calloc(n: usize, size: usize) -> *mut c_void;
    fn unlink(path: *const i8) -> i32;
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn LoadLibraryA(name: *const i8) -> u32 {
    let dll = std::ffi::CStr::from_ptr(name).to_string_lossy();
    match super::modules::load_library(&dll) {
        Some(h) => h,
        None => {
            crate::diag::note(crate::diag::Kind::DllNotFound, dll.into_owned());
            set_error(ERROR_MOD_NOT_FOUND);
            0
        }
    }
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn GetProcAddress(module: u32, name: u32) -> u32 {
    let r = get_proc_address(module, name);
    if r == 0 && crate::diag::enabled() {
        let dll = super::modules::module_file_name(module).unwrap_or_else(|| format!("{module:#x}"));
        let f = if name < 0x10000 { format!("#{name}") } else { std::ffi::CStr::from_ptr(name as usize as *const i8).to_string_lossy().into_owned() };
        crate::diag::note(crate::diag::Kind::ProcNotFound, format!("{dll}!{f}"));
    }
    r
}

unsafe fn get_proc_address(module: u32, name: u32) -> u32 {
    use super::modules::{get_proc, Proc};
    let p = if name < 0x10000 {
        Proc::Ordinal(name)
    } else {
        // `to_string_lossy` borrows: we keep the String alive during the call.
        let s = std::ffi::CStr::from_ptr(name as usize as *const i8).to_string_lossy().into_owned();
        return get_proc(module, Proc::Name(&s)).unwrap_or_else(|| {
            set_error(ERROR_PROC_NOT_FOUND);
            0
        });
    };
    get_proc(module, p).unwrap_or_else(|| {
        set_error(ERROR_PROC_NOT_FOUND);
        0
    })
}

#[allow(non_snake_case)]
extern "stdcall" fn LocalAlloc(_flags: u32, bytes: usize) -> *mut c_void {
    // Memory is always zeroed (superset of LMEM_ZEROINIT).
    unsafe { calloc(1, bytes.max(1)) }
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn DeleteFileA(path: *const i8) -> i32 {
    let p: Vec<u8> = std::ffi::CStr::from_ptr(path).to_bytes().iter().map(|&b| if b == b'\\' { b'/' } else { b }).collect();
    let c = CString::new(p).unwrap_or_default();
    (unlink(c.as_ptr()) == 0) as i32
}

// ---- resources ----------------------------------------------------------------------------

#[allow(non_snake_case)]
unsafe extern "stdcall" fn FindResourceA(module: u32, name: u32, ty: u32) -> u32 {
    use super::resources::{find, res_id};
    let base = if module == 0 { IMAGE_BASE.load(Ordering::Relaxed) } else { module };
    find(base, &res_id(ty), &res_id(name)).map_or(0, |(entry, _, _)| entry)
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn LoadResource(module: u32, info: u32) -> u32 {
    let base = if module == 0 { IMAGE_BASE.load(Ordering::Relaxed) } else { module };
    if info == 0 { 0 } else { base + *(info as usize as *const u32) }
}

#[allow(non_snake_case)]
extern "stdcall" fn LockResource(h: u32) -> u32 {
    h
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn SizeofResource(_module: u32, info: u32) -> u32 {
    if info == 0 { 0 } else { *((info + 4) as usize as *const u32) }
}

// ---- environment, time, text --------------------------------------------------------------

#[allow(non_snake_case)]
extern "stdcall" fn SetEnvironmentVariableA(_name: u32, _value: u32) -> i32 {
    1
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn GetLocalTime(st: *mut u16) {
    get_local_time(st)
}

pub unsafe fn get_local_time(st: *mut u16) {
    // SYSTEMTIME: year, month, dow, day, hour, minute, second, ms (UTC for now).
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
    let (days, rem) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    // days -> civil date (Howard Hinnant's algorithm)
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + (month <= 2) as i64;
    let v = [year, month, (days + 4).rem_euclid(7), day, rem / 3600, rem % 3600 / 60, rem % 60, 0];
    for (i, x) in v.iter().enumerate() {
        *st.add(i) = *x as u16;
    }
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn GetTimeZoneInformation(tz: *mut u8) -> u32 {
    std::ptr::write_bytes(tz, 0, 172);
    0 // TIME_ZONE_ID_UNKNOWN
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn MultiByteToWideChar(_cp: u32, _flags: u32, src: *const u8, n: i32, dst: *mut u16, max: i32) -> i32 {
    let len = if n < 0 {
        let mut i = 0;
        while *src.add(i) != 0 { i += 1 }
        i + 1
    } else {
        n as usize
    };
    if max == 0 {
        return len as i32;
    }
    if (max as usize) < len {
        return 0;
    }
    for i in 0..len {
        *dst.add(i) = *src.add(i) as u16; // Latin-1
    }
    len as i32
}

// ---- global memory, critical sections, miscellaneous --------------------------------------

#[allow(non_snake_case)]
extern "stdcall" fn GlobalAlloc(_flags: u32, bytes: usize) -> *mut c_void {
    unsafe { calloc(1, bytes.max(1)) } // the "handle" is the pointer itself (no movable memory)
}

#[allow(non_snake_case)]
extern "stdcall" fn GlobalLock(h: u32) -> u32 {
    h
}

#[allow(non_snake_case)]
extern "stdcall" fn GlobalUnlock(_h: u32) -> i32 {
    1
}

extern "C" {
    fn free(p: *mut c_void);
    fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
}

#[allow(non_snake_case)]
extern "stdcall" fn GlobalFree(h: *mut c_void) -> u32 {
    unsafe { free(h) };
    0
}

#[allow(non_snake_case)]
extern "stdcall" fn LocalFree(h: *mut c_void) -> u32 {
    GlobalFree(h)
}

#[allow(non_snake_case)]
extern "stdcall" fn GlobalReAlloc(h: *mut c_void, bytes: usize, _flags: u32) -> *mut c_void {
    unsafe { realloc(h, bytes.max(1)) }
}

#[allow(non_snake_case)]
extern "stdcall" fn FreeResource(_h: u32) -> i32 {
    0
}

#[allow(non_snake_case)]
extern "stdcall" fn FreeLibrary(_m: u32) -> i32 { // DLLs stay loaded until the end of the process
    
    1
}

#[allow(non_snake_case)]
extern "stdcall" fn RaiseException(code: u32, _flags: u32, _n: u32, _args: u32) -> ! {
    eprintln!("[winlayer] RaiseException({code:#x})");
    let what = if code == 0xE06D_7363 { " (C++ throw; exceptions are not supported)" } else { "" };
    if crate::diag::enabled() {
        crate::diag::note(crate::diag::Kind::Exception, format!("RaiseException({code:#x}){what}{}", crate::diag::callers()));
    }
    crate::diag::set_end(format!("unhandled exception {code:#x}"));
    exit_process(code)
}

// CRITICAL_SECTION (24 bytes, in program memory): [4]=LockCount, [8]=RecursionCount,
// [12]=OwningThread. Implemented with the owner in [12] and a short busy wait.
#[allow(non_snake_case)]
unsafe extern "stdcall" fn InitializeCriticalSection(cs: *mut u32) {
    std::ptr::write_bytes(cs as *mut u8, 0, 24);
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn EnterCriticalSection(cs: *mut u32) {
    use std::sync::atomic::AtomicU32;
    let me = super::sync::current_tid();
    let owner = &*(cs.add(3) as *const AtomicU32);
    loop {
        match owner.compare_exchange(0, me, Ordering::Acquire, Ordering::Relaxed) {
            Ok(_) => break,
            Err(cur) if cur == me => break,
            Err(_) => std::thread::yield_now(),
        }
    }
    *cs.add(2) += 1;
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn LeaveCriticalSection(cs: *mut u32) {
    use std::sync::atomic::AtomicU32;
    *cs.add(2) -= 1;
    if *cs.add(2) == 0 {
        (*(cs.add(3) as *const AtomicU32)).store(0, Ordering::Release);
    }
}

#[allow(non_snake_case)]
extern "stdcall" fn DeleteCriticalSection(_cs: u32) {}
