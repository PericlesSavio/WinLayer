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
    fn fread(p: *mut c_void, size: usize, n: usize, f: *mut c_void) -> usize;
    fn fwrite(p: *const c_void, size: usize, n: usize, f: *mut c_void) -> usize;
    fn fseek(f: *mut c_void, off: i32, whence: i32) -> i32;
    fn ftell(f: *mut c_void) -> i32;
    fn fgets(s: *mut i8, n: i32, f: *mut c_void) -> *mut i8;
    fn vfprintf(f: *mut c_void, fmt: *const i8, args: *mut c_void) -> i32;
    fn vfscanf(f: *mut c_void, fmt: *const i8, args: *mut c_void) -> i32;
    fn putc(c: i32, f: *mut c_void) -> i32;
    fn feof(f: *mut c_void) -> i32;
    fn ferror(f: *mut c_void) -> i32;
    fn strncmp(a: *const i8, b: *const i8, n: usize) -> i32;
    fn strncpy(d: *mut i8, s: *const i8, n: usize) -> *mut i8;
    fn vsprintf(buf: *mut i8, fmt: *const i8, args: *mut c_void) -> i32;
    fn strftime(buf: *mut i8, max: usize, fmt: *const i8, tm: *const MuslTm) -> usize;
    fn winlayer_ftol();
    fn winlayer_ciacos();
    fn winlayer_cipow();
    fn winlayer_fprintf();
    fn winlayer_fscanf();
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
    ".globl winlayer_cipow",
    "winlayer_cipow:", // pow(st1, st0)
    "sub esp, 16",
    "fstp qword ptr [esp + 8]",
    "fstp qword ptr [esp]",
    "call {pow}",
    "add esp, 16",
    "ret",
    // fprintf/fscanf(f, fmt, ...): the variadic arguments are already a va_list on the stack.
    ".globl winlayer_fprintf",
    "winlayer_fprintf:",
    "lea eax, [esp + 12]",
    "push eax",
    "push dword ptr [esp + 12]",
    "push dword ptr [esp + 12]",
    "call {vfprintf}",
    "add esp, 12",
    "ret",
    ".globl winlayer_fscanf",
    "winlayer_fscanf:",
    "lea eax, [esp + 12]",
    "push eax",
    "push dword ptr [esp + 12]",
    "push dword ptr [esp + 12]",
    "call {vfscanf}",
    "add esp, 12",
    "ret",
    pow = sym pow_,
    vfprintf = sym ms_vfprintf,
    vfscanf = sym ms_vfscanf,
);

extern "C" fn pow_(x: f64, y: f64) -> f64 {
    x.powf(y)
}

static COMMODE: AtomicU32 = AtomicU32::new(0);
static FMODE: AtomicU32 = AtomicU32::new(0);
static ADJUST_FDIV: AtomicU32 = AtomicU32::new(0);
static ACMDLN: AtomicU32 = AtomicU32::new(0);
static RAND: AtomicU32 = AtomicU32::new(1);

/// Imports that are variables, not functions: the API trace must not wrap them.
pub fn is_data(label: &str) -> bool {
    label.ends_with("!_acmdln") || label.ends_with("!_adjust_fdiv")
}

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
        "sprintf" => sprintf as *const () as usize,
        "sscanf" => sscanf as *const () as usize,
        "strncmp" => strncmp as *const () as usize,
        "strncpy" => strncpy as *const () as usize,
        "vsprintf" => vsprintf as *const () as usize,
        "_CIpow" => winlayer_cipow as *const () as usize,
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
        "fclose" => Some(ms_fclose as *const () as usize),
        "fread" => Some(ms_fread as *const () as usize),
        "fwrite" => Some(ms_fwrite as *const () as usize),
        "fseek" => Some(ms_fseek as *const () as usize),
        "ftell" => Some(ms_ftell as *const () as usize),
        "fgets" => Some(ms_fgets as *const () as usize),
        "putc" | "fputc" => Some(ms_putc as *const () as usize),
        "feof" => Some(ms_feof as *const () as usize),
        "ferror" => Some(ms_ferror as *const () as usize),
        "fprintf" => Some(winlayer_fprintf as *const () as usize),
        "fscanf" => Some(winlayer_fscanf as *const () as usize),
        "rand" => Some(rand as *const () as usize),
        "strftime" => Some(strftime_ as *const () as usize),
        "_purecall" => Some(purecall as *const () as usize),
        _ => exports!(
            name, __set_app_type, __setusermatherr, __p__commode, __p__fmode, __getmainargs, _controlfp, _except_handler3,
            _XcptFilter, _initterm, exit, srand, clock, time, localtime, strftime_, _strlwr, __CxxFrameHandler,
            _beginthread, _endthread, _exit, _cexit, _c_exit, _itoa, _ltoa, wcscpy, wcslen,
        ),
    }
}

/// The Microsoft `FILE` (`_iobuf`): programs read `_flag` directly, e.g. the `feof` macro tests
/// `_IOEOF`. The musl stream goes after the 32 bytes the program knows about.
#[repr(C)]
struct MsFile {
    ptr: u32,
    cnt: i32,
    base: u32,
    flag: u32,
    file: i32,
    charbuf: i32,
    bufsiz: i32,
    tmpfname: u32,
    host: *mut c_void,
}

const IOREAD: u32 = 0x1;
const IOWRT: u32 = 0x2;
const IOEOF: u32 = 0x10;
const IOERR: u32 = 0x20;
const IORW: u32 = 0x80;

/// Copies the musl end-of-file and error state into `_flag`.
unsafe fn sync(f: *mut MsFile) {
    let mut flag = (*f).flag & !(IOEOF | IOERR);
    if feof((*f).host) != 0 {
        flag |= IOEOF;
    }
    if ferror((*f).host) != 0 {
        flag |= IOERR;
    }
    (*f).flag = flag;
}

/// The program's paths are those of the virtual Windows: they go through the same mapping as CreateFile.
unsafe extern "C" fn fopen_(path: *const i8, mode: *const i8) -> *mut MsFile {
    let win = std::ffi::CStr::from_ptr(path).to_string_lossy().into_owned();
    let Ok(host) = std::ffi::CString::new(super::files::host_path(&win).to_string_lossy().into_owned()) else {
        return std::ptr::null_mut();
    };
    let stream = fopen(host.as_ptr(), mode);
    if stream.is_null() {
        crate::diag::note(crate::diag::Kind::FileNotFound, format!("{win} -> {} (fopen)", host.to_string_lossy()));
        return std::ptr::null_mut();
    }
    let m = std::ffi::CStr::from_ptr(mode).to_bytes();
    let flag = if m.contains(&b'+') { IORW } else if m.first() == Some(&b'r') { IOREAD } else { IOWRT };
    Box::into_raw(Box::new(MsFile { ptr: 0, cnt: 0, base: 0, flag, file: 3, charbuf: 0, bufsiz: 0, tmpfname: 0, host: stream }))
}

unsafe extern "C" fn ms_fclose(f: *mut MsFile) -> i32 {
    if f.is_null() {
        return -1;
    }
    let r = fclose((*f).host);
    drop(Box::from_raw(f));
    r
}

unsafe extern "C" fn ms_fread(p: *mut c_void, size: usize, n: usize, f: *mut MsFile) -> usize {
    let r = fread(p, size, n, (*f).host);
    sync(f);
    r
}

unsafe extern "C" fn ms_fwrite(p: *const c_void, size: usize, n: usize, f: *mut MsFile) -> usize {
    let r = fwrite(p, size, n, (*f).host);
    sync(f);
    r
}

unsafe extern "C" fn ms_fseek(f: *mut MsFile, off: i32, whence: i32) -> i32 {
    let r = fseek((*f).host, off, whence);
    sync(f); // a successful seek clears end-of-file
    r
}

unsafe extern "C" fn ms_ftell(f: *mut MsFile) -> i32 {
    ftell((*f).host)
}

unsafe extern "C" fn ms_fgets(s: *mut i8, n: i32, f: *mut MsFile) -> *mut i8 {
    let r = fgets(s, n, (*f).host);
    sync(f);
    r
}

unsafe extern "C" fn ms_putc(c: i32, f: *mut MsFile) -> i32 {
    let r = putc(c, (*f).host);
    sync(f);
    r
}

unsafe extern "C" fn ms_feof(f: *mut MsFile) -> i32 {
    ((*f).flag & IOEOF) as i32
}

unsafe extern "C" fn ms_ferror(f: *mut MsFile) -> i32 {
    ((*f).flag & IOERR) as i32
}

unsafe extern "C" fn ms_vfprintf(f: *mut MsFile, fmt: *const i8, args: *mut c_void) -> i32 {
    let r = vfprintf((*f).host, fmt, args);
    sync(f);
    r
}

unsafe extern "C" fn ms_vfscanf(f: *mut MsFile, fmt: *const i8, args: *mut c_void) -> i32 {
    let r = vfscanf((*f).host, fmt, args);
    sync(f);
    r
}

/// The same generator as the Microsoft runtime: the game may rely on the 32767 limit.
extern "C" fn rand() -> i32 {
    let next = RAND.load(Ordering::Relaxed).wrapping_mul(214_013).wrapping_add(2_531_011);
    RAND.store(next, Ordering::Relaxed);
    ((next >> 16) & 0x7FFF) as i32
}

extern "C" fn srand(seed: u32) {
    RAND.store(seed, Ordering::Relaxed);
}

/// Milliseconds since the program started (CLOCKS_PER_SEC is 1000 on Windows).
extern "C" fn clock() -> i32 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_millis() as i32
}

/// 32-bit time_t, as in the Microsoft runtime of the time.
unsafe extern "C" fn time(out: *mut i32) -> i32 {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i32);
    if !out.is_null() {
        *out = t;
    }
    t
}

/// The Windows `struct tm` (9 ints); musl's has two more fields, so it is converted for strftime.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Tm {
    sec: i32, min: i32, hour: i32, mday: i32, mon: i32, year: i32, wday: i32, yday: i32, isdst: i32,
}

#[repr(C)]
struct MuslTm {
    tm: Tm,
    gmtoff: i32,
    zone: *const i8,
}

/// Breaks down a time in UTC (the host's time zone is not looked up).
fn break_down(t: i64) -> Tm {
    let (days, rem) = (t.div_euclid(86_400), t.rem_euclid(86_400));
    // civil_from_days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + (m <= 2) as i64;
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    const START: [i32; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let yday = START[(m - 1) as usize] + d as i32 - 1 + (leap && m > 2) as i32;
    Tm {
        sec: (rem % 60) as i32, min: (rem / 60 % 60) as i32, hour: (rem / 3600) as i32, mday: d as i32, mon: m as i32 - 1,
        year: y as i32 - 1900, wday: (days + 4).rem_euclid(7) as i32, yday, isdst: 0,
    }
}

unsafe extern "C" fn localtime(t: *const i32) -> *mut Tm {
    static mut TM: Tm = Tm { sec: 0, min: 0, hour: 0, mday: 0, mon: 0, year: 0, wday: 0, yday: 0, isdst: 0 };
    if t.is_null() {
        return std::ptr::null_mut();
    }
    let p = &raw mut TM;
    *p = break_down(*t as i64);
    p
}

unsafe extern "C" fn strftime_(buf: *mut i8, max: usize, fmt: *const i8, tm: *const Tm) -> usize {
    let m = MuslTm { tm: *tm, gmtoff: 0, zone: c"UTC".as_ptr() };
    strftime(buf, max, fmt, &m)
}

unsafe extern "C" fn _strlwr(s: *mut u8) -> *mut u8 {
    let mut p = s;
    while *p != 0 {
        *p = (*p).to_ascii_lowercase();
        p = p.add(1);
    }
    s
}

/// C++ exceptions are not supported: the handler declines, so the search goes on.
extern "C" fn __CxxFrameHandler(_rec: u32, _frame: u32, _ctx: u32, _disp: u32) -> u32 {
    1 // ExceptionContinueSearch
}

/// `start` is cdecl; the thread entry calls it as stdcall with one argument, and since the thread
/// ends right after, the argument left on the stack does no harm.
extern "C" fn _beginthread(start: u32, stack: u32, arg: u32) -> u32 {
    let h = super::sync::create_thread(stack, start, arg);
    if h == 0 { u32::MAX } else { h }
}

extern "C" fn _endthread() {
    super::sync::exit_thread(0)
}

extern "C" fn purecall() -> i32 {
    crate::diag::note(crate::diag::Kind::Crash, "pure virtual function call");
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
    fn break_down_dates() {
        let t = break_down(0);
        assert_eq!((t.year, t.mon, t.mday, t.wday, t.yday), (70, 0, 1, 4, 0));
        let t = break_down(951_827_696); // 2000-02-29 12:34:56
        assert_eq!((t.year, t.mon, t.mday, t.hour, t.min, t.sec, t.yday, t.wday), (100, 1, 29, 12, 34, 56, 59, 2));
    }

    #[test]
    fn rand_matches_microsoft_sequence() {
        RAND.store(1, Ordering::Relaxed);
        assert_eq!((rand(), rand(), rand()), (41, 18467, 6334));
    }
}
