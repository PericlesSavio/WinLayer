//! msvcrt.dll — Microsoft C runtime. Functions with the same convention (cdecl) and the same behavior go straight
//! to musl, including `sprintf`/`sscanf`: the caller cleans the stack, so variadic arguments pass through intact.

use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};

extern "C" {
    fn malloc(n: usize) -> *mut c_void;
    fn free(p: *mut c_void);
    fn calloc(n: usize, size: usize) -> *mut c_void;
    fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
    fn memmove(d: *mut c_void, s: *const c_void, n: usize) -> *mut c_void;
    fn atoi(s: *const i8) -> i32;
    fn atol(s: *const i8) -> i32;
    fn isalnum(c: i32) -> i32;
    fn isdigit(c: i32) -> i32;
    fn isspace(c: i32) -> i32;
    fn strstr(a: *const i8, b: *const i8) -> *const i8;
    fn strncasecmp(a: *const i8, b: *const i8, n: usize) -> i32;
    fn floor(x: f64) -> f64;
    fn fopen(path: *const i8, mode: *const i8) -> *mut c_void;
    fn fclose(f: *mut c_void) -> i32;
    fn sprintf(buf: *mut i8, fmt: *const i8, ...) -> i32;
    fn sscanf(s: *const i8, fmt: *const i8, ...) -> i32;
    fn winlayer_ftol();
    fn winlayer_ciacos();
}

// Microsoft compiler helpers that take their argument on the x87 stack (st0), not on the normal stack.
core::arch::global_asm!(
    ".globl winlayer_ftol",
    "winlayer_ftol:",
    "sub esp, 12",
    "fnstcw word ptr [esp + 8]",
    "mov ax, word ptr [esp + 8]",
    "or ax, 0xC00", // round by truncating
    "mov word ptr [esp + 10], ax",
    "fldcw word ptr [esp + 10]",
    "fistp qword ptr [esp]",
    "fldcw word ptr [esp + 8]",
    "mov eax, dword ptr [esp]",
    "mov edx, dword ptr [esp + 4]",
    "add esp, 12",
    "ret",
    ".globl winlayer_ciacos",
    "winlayer_ciacos:", // acos(x) = atan2(sqrt(1 - x*x), x)
    "fld st(0)",
    "fmul st(0), st(0)",
    "fld1",
    "fsubrp st(1), st(0)",
    "fsqrt",
    "fxch st(1)",
    "fpatan",
    "ret",
);

static COMMODE: AtomicU32 = AtomicU32::new(0);
static FMODE: AtomicU32 = AtomicU32::new(0);
static ADJUST_FDIV: AtomicU32 = AtomicU32::new(0);
static ACMDLN: AtomicU32 = AtomicU32::new(0);
static RAND: AtomicU32 = AtomicU32::new(1);

pub fn resolve(name: &str) -> Option<usize> {
    let direct = match name {
        "malloc" | "??2@YAPAXI@Z" => malloc as *const () as usize, // operator new
        "free" | "??3@YAXPAX@Z" => free as *const () as usize,     // operator delete
        "calloc" => calloc as *const () as usize,
        "realloc" => realloc as *const () as usize,
        "memmove" => memmove as *const () as usize,
        "atoi" => atoi as *const () as usize,
        "atol" => atol as *const () as usize,
        "isalnum" => isalnum as *const () as usize,
        "isdigit" => isdigit as *const () as usize,
        "isspace" => isspace as *const () as usize,
        "strstr" => strstr as *const () as usize,
        "_strnicmp" => strncasecmp as *const () as usize,
        "floor" => floor as *const () as usize,
        "fclose" => fclose as *const () as usize,
        "sprintf" => sprintf as *const () as usize,
        "sscanf" => sscanf as *const () as usize,
        "_ftol" => winlayer_ftol as *const () as usize,
        "_CIacos" => winlayer_ciacos as *const () as usize,
        // data imports: the address is that of the variable itself
        "_adjust_fdiv" => ADJUST_FDIV.as_ptr() as usize,
        "_acmdln" => {
            ACMDLN.store(super::kernel32::GetCommandLineA() as u32, Ordering::Relaxed);
            ACMDLN.as_ptr() as usize
        }
        _ => 0,
    };
    if direct != 0 {
        return Some(direct);
    }
    match name {
        "fopen" => Some(fopen_ as *const () as usize),
        "rand" => Some(rand as *const () as usize),
        "_purecall" => Some(purecall as *const () as usize),
        _ => exports!(
            name, __set_app_type, __setusermatherr, __p__commode, __p__fmode, __getmainargs, _controlfp, _except_handler3,
            _XcptFilter, _initterm, exit, _exit, _cexit, _c_exit, _itoa, _ltoa, wcscpy, wcslen,
        ),
    }
}

/// The program's paths are those of the virtual Windows: they go through the same mapping as CreateFile.
unsafe extern "C" fn fopen_(path: *const i8, mode: *const i8) -> *mut c_void {
    let win = std::ffi::CStr::from_ptr(path).to_string_lossy().into_owned();
    let Ok(host) = std::ffi::CString::new(super::files::host_path(&win).to_string_lossy().into_owned()) else {
        return std::ptr::null_mut();
    };
    fopen(host.as_ptr(), mode)
}

/// The same generator as the Microsoft runtime: the game may rely on the 32767 limit.
extern "C" fn rand() -> i32 {
    let next = RAND.load(Ordering::Relaxed).wrapping_mul(214_013).wrapping_add(2_531_011);
    RAND.store(next, Ordering::Relaxed);
    ((next >> 16) & 0x7FFF) as i32
}

extern "C" fn purecall() -> i32 {
    eprintln!("[winlayer] msvcrt: pure virtual function call");
    super::kernel32::exit_process(3)
}

extern "C" fn __set_app_type(_t: i32) {}

extern "C" fn __setusermatherr(_f: u32) {}

extern "C" fn __p__commode() -> *mut u32 {
    COMMODE.as_ptr()
}

extern "C" fn __p__fmode() -> *mut u32 {
    FMODE.as_ptr()
}

/// argc = 1: none of the tested programs receives arguments this way.
unsafe extern "C" fn __getmainargs(argc: *mut i32, argv: *mut u32, env: *mut u32, _wild: i32, _info: u32) -> i32 {
    let args: &'static mut [u32; 2] = Box::leak(Box::new([c"program.exe".as_ptr() as u32, 0]));
    let envp: &'static mut [u32; 1] = Box::leak(Box::new([0]));
    *argc = 1;
    *argv = args.as_ptr() as u32;
    *env = envp.as_ptr() as u32;
    0
}

/// Runtime default control word; the request is ignored.
extern "C" fn _controlfp(_new: u32, _mask: u32) -> u32 {
    0x0009_001F
}

extern "C" fn _except_handler3(_a: u32, _b: u32, _c: u32, _d: u32) -> u32 {
    1 // ExceptionContinueSearch
}

extern "C" fn _XcptFilter(_code: u32, _info: u32) -> i32 {
    0
}

unsafe extern "C" fn _initterm(mut begin: *const u32, end: *const u32) {
    while begin < end {
        if *begin != 0 {
            let f: extern "C" fn() = std::mem::transmute(*begin as usize);
            f();
        }
        begin = begin.add(1);
    }
}

extern "C" fn exit(code: u32) {
    super::kernel32::exit_process(code)
}

extern "C" fn _exit(code: u32) {
    super::kernel32::exit_process(code)
}

extern "C" fn _cexit() {}

extern "C" fn _c_exit() {}

unsafe fn format_radix(value: i32, buf: *mut u8, radix: u32) -> *mut u8 {
    let negative = radix == 10 && value < 0;
    let mut v = if negative { (value as i64).unsigned_abs() as u32 } else { value as u32 };
    let mut digits = Vec::new();
    loop {
        digits.push(char::from_digit(v % radix.clamp(2, 36), radix.clamp(2, 36)).unwrap_or('0') as u8);
        v /= radix.clamp(2, 36);
        if v == 0 {
            break;
        }
    }
    if negative {
        digits.push(b'-');
    }
    digits.reverse();
    std::ptr::copy_nonoverlapping(digits.as_ptr(), buf, digits.len());
    *buf.add(digits.len()) = 0;
    buf
}

unsafe extern "C" fn _itoa(value: i32, buf: *mut u8, radix: u32) -> *mut u8 {
    format_radix(value, buf, radix)
}

unsafe extern "C" fn _ltoa(value: i32, buf: *mut u8, radix: u32) -> *mut u8 {
    format_radix(value, buf, radix)
}

/// Windows wchar_t is 16-bit (musl's is 32): these cannot go to libc.
unsafe extern "C" fn wcslen(s: *const u16) -> usize {
    let mut n = 0;
    while *s.add(n) != 0 {
        n += 1;
    }
    n
}

unsafe extern "C" fn wcscpy(dst: *mut u16, src: *const u16) -> *mut u16 {
    std::ptr::copy(src, dst, wcslen(src) + 1);
    dst
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn itoa_signed_and_hexadecimal() {
        let mut b = [0u8; 16];
        unsafe {
            format_radix(-42, b.as_mut_ptr(), 10);
            assert_eq!(std::ffi::CStr::from_ptr(b.as_ptr() as *const i8).to_str().unwrap(), "-42");
            format_radix(255, b.as_mut_ptr(), 16);
            assert_eq!(std::ffi::CStr::from_ptr(b.as_ptr() as *const i8).to_str().unwrap(), "ff");
        }
    }

    #[test]
    fn rand_matches_microsoft_sequence() {
        RAND.store(1, Ordering::Relaxed);
        assert_eq!((rand(), rand(), rand()), (41, 18467, 6334));
    }
}
