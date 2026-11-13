//! kernel32: heaps, locale/strings, version, time and legacy I/O (`_l*`) used by the C runtime.

use std::ffi::c_void;

pub fn resolve(name: &str) -> Option<usize> {
    exports!(
        name,
        OpenProcess, VirtualQueryEx, GetSystemInfo,
        HeapCreate, HeapDestroy, HeapAlloc, HeapFree, HeapReAlloc, HeapSize, HeapValidate, GetProcessHeap,
        SetLastError, GetCommandLineW, GetEnvironmentStringsW, GetStringTypeA, GetStringTypeW,
        LCMapStringA, LCMapStringW, CompareStringA, CompareStringW, GetVersionExA, GetSystemDirectoryA,
        GetSystemTime, VirtualQuery, LoadLibraryExA, LoadLibraryW, VirtualProtect, MulDiv, _lread, _lwrite, _llseek, _lclose,
        _hwrite, _lcreat, GlobalCompact, GetComputerNameA,
    )
    .or_else(|| super::stubs::resolve_kernel32(name))
}

extern "C" {
    fn calloc(n: usize, size: usize) -> *mut c_void;
    fn free(p: *mut c_void);
    fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
    fn malloc_usable_size(p: *mut c_void) -> usize;
}

// ---- heaps (all share the process malloc) --------------------------------------------------

const HEAP_HANDLE: u32 = 0x6000_0004;

#[allow(non_snake_case)]
extern "stdcall" fn HeapCreate(_opts: u32, _init: u32, _max: u32) -> u32 {
    HEAP_HANDLE
}

#[allow(non_snake_case)]
extern "stdcall" fn GetProcessHeap() -> u32 {
    HEAP_HANDLE
}

#[allow(non_snake_case)]
extern "stdcall" fn HeapDestroy(_h: u32) -> i32 {
    1
}

#[allow(non_snake_case)]
extern "stdcall" fn HeapAlloc(_h: u32, _flags: u32, size: usize) -> *mut c_void {
    unsafe { calloc(1, size.max(1)) }
}

#[allow(non_snake_case)]
extern "stdcall" fn HeapFree(_h: u32, _flags: u32, p: *mut c_void) -> i32 {
    unsafe { free(p) };
    1
}

#[allow(non_snake_case)]
extern "stdcall" fn HeapReAlloc(_h: u32, _flags: u32, p: *mut c_void, size: usize) -> *mut c_void {
    unsafe { realloc(p, size.max(1)) }
}

#[allow(non_snake_case)]
extern "stdcall" fn HeapSize(_h: u32, _flags: u32, p: *mut c_void) -> usize {
    unsafe { malloc_usable_size(p) }
}

#[allow(non_snake_case)]
extern "stdcall" fn HeapValidate(_h: u32, _flags: u32, _p: u32) -> i32 {
    1
}

#[allow(non_snake_case)]
extern "stdcall" fn SetLastError(e: u32) {
    super::kernel32::set_last_error(e);
}

#[allow(non_snake_case)]
extern "stdcall" fn MulDiv(a: i32, b: i32, c: i32) -> i32 {
    if c == 0 { -1 } else { ((a as i64 * b as i64 + c as i64 / 2) / c as i64) as i32 }
}

// ---- UTF-16 command line / environment ----------------------------------------------------

fn leak_wide(bytes: &[u8]) -> u32 {
    let mut v: Vec<u16> = bytes.iter().map(|&b| b as u16).collect();
    v.push(0);
    v.push(0);
    Box::leak(v.into_boxed_slice()).as_ptr() as u32
}

#[allow(non_snake_case)]
extern "stdcall" fn GetCommandLineW() -> u32 {
    let a = super::kernel32::command_line();
    leak_wide(a.to_bytes())
}

#[allow(non_snake_case)]
extern "stdcall" fn GetEnvironmentStringsW() -> u32 {
    leak_wide(b"")
}

// ---- string classification, mapping and comparison -----------------------------------------

fn ctype1(c: u32) -> u16 {
    let ch = char::from_u32(c).unwrap_or('\0');
    let mut f = 0u16;
    if ch.is_uppercase() { f |= 0x001 | 0x100 }
    if ch.is_lowercase() { f |= 0x002 | 0x100 }
    if ch.is_ascii_digit() { f |= 0x004 }
    if ch.is_whitespace() { f |= 0x008 }
    if ch.is_ascii_punctuation() { f |= 0x010 }
    if ch.is_control() { f |= 0x020 }
    if ch == ' ' || ch == '\t' { f |= 0x040 }
    if ch.is_ascii_hexdigit() { f |= 0x080 }
    if ch.is_alphabetic() { f |= 0x100 }
    f
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn GetStringTypeA(_locale: u32, _kind: u32, src: *const u8, n: i32, out: *mut u16) -> i32 {
    let len = if n < 0 { std::ffi::CStr::from_ptr(src as *const i8).to_bytes().len() + 1 } else { n as usize };
    for i in 0..len {
        *out.add(i) = ctype1(*src.add(i) as u32);
    }
    1
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn GetStringTypeW(_kind: u32, src: *const u16, n: i32, out: *mut u16) -> i32 {
    let len = if n < 0 {
        let mut i = 0;
        while *src.add(i) != 0 { i += 1 }
        i + 1
    } else {
        n as usize
    };
    for i in 0..len {
        *out.add(i) = ctype1(*src.add(i) as u32);
    }
    1
}

const LCMAP_LOWERCASE: u32 = 0x100;
const LCMAP_UPPERCASE: u32 = 0x200;

fn map_char(c: u32, flags: u32) -> u32 {
    let Some(ch) = char::from_u32(c) else { return c };
    let m = if flags & LCMAP_LOWERCASE != 0 { ch.to_lowercase().next() } else if flags & LCMAP_UPPERCASE != 0 { ch.to_uppercase().next() } else { Some(ch) };
    m.map_or(c, |x| x as u32)
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn LCMapStringA(_l: u32, flags: u32, src: *const u8, n: i32, dst: *mut u8, max: i32) -> i32 {
    let len = if n < 0 { std::ffi::CStr::from_ptr(src as *const i8).to_bytes().len() + 1 } else { n as usize };
    if max == 0 { return len as i32 }
    if (max as usize) < len { return 0 }
    for i in 0..len {
        *dst.add(i) = map_char(*src.add(i) as u32, flags) as u8;
    }
    len as i32
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn LCMapStringW(_l: u32, flags: u32, src: *const u16, n: i32, dst: *mut u16, max: i32) -> i32 {
    let len = if n < 0 {
        let mut i = 0;
        while *src.add(i) != 0 { i += 1 }
        i + 1
    } else {
        n as usize
    };
    if max == 0 { return len as i32 }
    if (max as usize) < len { return 0 }
    for i in 0..len {
        *dst.add(i) = map_char(*src.add(i) as u32, flags) as u16;
    }
    len as i32
}

fn cmp_result(a: &[u32], b: &[u32], ignore_case: bool) -> i32 {
    let f = |v: &[u32]| -> Vec<u32> { v.iter().map(|&c| if ignore_case { map_char(c, LCMAP_LOWERCASE) } else { c }).collect() };
    match f(a).cmp(&f(b)) {
        std::cmp::Ordering::Less => 1,    // CSTR_LESS_THAN
        std::cmp::Ordering::Equal => 2,   // CSTR_EQUAL
        std::cmp::Ordering::Greater => 3, // CSTR_GREATER_THAN
    }
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn CompareStringA(_l: u32, flags: u32, a: *const u8, na: i32, b: *const u8, nb: i32) -> i32 {
    let rd = |p: *const u8, n: i32| -> Vec<u32> {
        let len = if n < 0 { std::ffi::CStr::from_ptr(p as *const i8).to_bytes().len() } else { n as usize };
        (0..len).map(|i| *p.add(i) as u32).collect()
    };
    cmp_result(&rd(a, na), &rd(b, nb), flags & 1 != 0)
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn CompareStringW(_l: u32, flags: u32, a: *const u16, na: i32, b: *const u16, nb: i32) -> i32 {
    let rd = |p: *const u16, n: i32| -> Vec<u32> {
        let len = if n < 0 {
            let mut i = 0;
            while *p.add(i) != 0 { i += 1 }
            i
        } else {
            n as usize
        };
        (0..len).map(|i| *p.add(i) as u32).collect()
    };
    cmp_result(&rd(a, na), &rd(b, nb), flags & 1 != 0)
}

// ---- system, time, modules ----------------------------------------------------------------

#[allow(non_snake_case)]
unsafe extern "stdcall" fn GetVersionExA(info: *mut u32) -> i32 {
    // OSVERSIONINFO: size, major, minor, build, platform, csd[128]. Windows 95 (4.00.950).
    if *info < 20 {
        return 0;
    }
    *info.add(1) = 4;
    *info.add(2) = 0;
    *info.add(3) = 0xC000_0000 | 950;
    *info.add(4) = 1; // VER_PLATFORM_WIN32_WINDOWS
    if *info >= 148 {
        std::ptr::write_bytes(info.add(5) as *mut u8, 0, 128);
    }
    1
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn GetSystemDirectoryA(buf: *mut u8, max: u32) -> u32 {
    let s = b"C:\\WINDOWS\\SYSTEM";
    if buf.is_null() || max as usize <= s.len() {
        return s.len() as u32 + 1;
    }
    std::ptr::copy_nonoverlapping(s.as_ptr(), buf, s.len());
    *buf.add(s.len()) = 0;
    s.len() as u32
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn GetSystemTime(st: *mut u16) {
    super::kernel32::get_local_time(st); // no time zone, local == UTC
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn LoadLibraryExA(name: *const i8, _file: u32, _flags: u32) -> u32 {
    super::modules::load_library(&std::ffi::CStr::from_ptr(name).to_string_lossy()).unwrap_or(0)
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn LoadLibraryW(name: *const u16) -> u32 {
    let mut s = String::new();
    let mut i = 0;
    while *name.add(i) != 0 {
        s.push(char::from_u32(*name.add(i) as u32).unwrap_or('?'));
        i += 1;
    }
    super::modules::load_library(&s).unwrap_or(0)
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn VirtualProtect(_addr: u32, _size: u32, _new: u32, old: *mut u32) -> i32 {
    if !old.is_null() {
        *old = 4; // PAGE_READWRITE
    }
    1 // the memory is already RWX
}

// ---- legacy I/O (_lread etc.) -------------------------------------------------------------

#[allow(non_snake_case)]
unsafe extern "stdcall" fn _lread(h: u32, buf: *mut u8, n: u32) -> u32 {
    let mut got = 0u32;
    if super::files::read_file(h, buf, n, &mut got) { got } else { u32::MAX }
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn _lwrite(h: u32, buf: *const u8, n: u32) -> u32 {
    let mut done = 0u32;
    if super::kernel32::write_file_raw(h as usize, buf, n, &mut done) != 0 { done } else { u32::MAX }
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn _hwrite(h: u32, buf: *const u8, n: u32) -> u32 {
    _lwrite(h, buf, n)
}

/// Creates or truncates the file, opened for reading and writing; the attributes are ignored.
#[allow(non_snake_case)]
unsafe extern "stdcall" fn _lcreat(name: *const i8, _attributes: u32) -> i32 {
    const GENERIC_READ_WRITE: u32 = 0xC000_0000;
    const SHARE_READ_WRITE: u32 = 3;
    const CREATE_ALWAYS: u32 = 2;
    let h = super::files::CreateFileA(name, GENERIC_READ_WRITE, SHARE_READ_WRITE, 0, CREATE_ALWAYS, 0, 0);
    if h == u32::MAX { -1 } else { h as i32 }
}

/// Memory is never moved here: reports a large free block, as a machine with plenty of RAM would.
#[allow(non_snake_case)]
extern "stdcall" fn GlobalCompact(_min_free: u32) -> u32 {
    0x0400_0000
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn GetComputerNameA(buf: *mut u8, size: *mut u32) -> i32 {
    let name = b"WINLAYER\0";
    if buf.is_null() || size.is_null() || (*size as usize) < name.len() {
        if !size.is_null() {
            *size = name.len() as u32;
        }
        return 0;
    }
    std::ptr::copy_nonoverlapping(name.as_ptr(), buf, name.len());
    *size = name.len() as u32 - 1;
    1
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn _llseek(h: u32, off: i32, origin: u32) -> u32 {
    super::files::SetFilePointer(h, off, std::ptr::null_mut(), origin)
}

#[allow(non_snake_case)]
extern "stdcall" fn _lclose(h: u32) -> u32 {
    super::files::close(h);
    0
}

/// Only the program itself can be opened: the pseudo handle of the current process.
#[allow(non_snake_case)]
extern "stdcall" fn OpenProcess(_access: u32, _inherit: i32, pid: u32) -> u32 {
    if pid == std::process::id() { 0xFFFF_FFFF } else { 0 }
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn VirtualQueryEx(_process: u32, addr: u32, info: *mut u32, len: u32) -> u32 {
    VirtualQuery(addr, info, len)
}

/// SYSTEM_INFO of a single Pentium Pro class processor, with the Windows page size and 64 KB granularity.
#[allow(non_snake_case)]
unsafe extern "stdcall" fn GetSystemInfo(info: *mut u32) {
    // dwOemId/wProcessorArchitecture, dwPageSize, lpMinimumApplicationAddress, lpMaximumApplicationAddress,
    // dwActiveProcessorMask, dwNumberOfProcessors, dwProcessorType, dwAllocationGranularity,
    // wProcessorLevel (6) + wProcessorRevision (0)
    let v = [0u32, 0x1000, 0x10000, 0x7FFE_FFFF, 1, 1, 586, 0x10000, 6];
    for (i, x) in v.iter().enumerate() {
        *info.add(i) = *x;
    }
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn VirtualQuery(addr: u32, info: *mut u32, len: u32) -> u32 {
    // MEMORY_BASIC_INFORMATION (28 bytes): BaseAddress, AllocationBase, AllocationProtect,
    // RegionSize, State, Protect, Type.
    if len < 28 {
        return 0;
    }
    let (base, size) = super::kernel32::region_of(addr as usize).unwrap_or(((addr & !0xFFF) as usize, 0x1000));
    *info = base as u32;
    *info.add(1) = base as u32;
    *info.add(2) = 4; // PAGE_READWRITE
    *info.add(3) = (size - (addr as usize - base)) as u32;
    *info.add(4) = 0x1000; // MEM_COMMIT
    *info.add(5) = 4;
    *info.add(6) = 0x20000; // MEM_PRIVATE
    28
}
