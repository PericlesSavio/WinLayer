//! kernel32 file functions with Windows -> Linux path translation.
//!
//! `C:\` (or any leading `X:\` and `\`) is the .exe's folder; relative paths start from the
//! current directory (which starts at the .exe's folder). Names are resolved case-insensitively,
//! as on Windows.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

pub const INVALID_HANDLE: u32 = u32::MAX;
const ERROR_FILE_NOT_FOUND: u32 = 2;
const ERROR_ALREADY_EXISTS: u32 = 183;
const ERROR_INVALID_HANDLE: u32 = 6;

static ROOT: OnceLock<PathBuf> = OnceLock::new();
static TABLE: Mutex<BTreeMap<u32, File>> = Mutex::new(BTreeMap::new());
static NEXT: AtomicU32 = AtomicU32::new(0x4000);

pub fn root() -> Option<&'static PathBuf> {
    ROOT.get()
}

pub fn set_root(dir: PathBuf) {
    let _ = ROOT.set(dir);
}

pub fn resolve(name: &str) -> Option<usize> {
    exports!(
        name,
        CreateFileA, ReadFile, SetFilePointer, GetFileSize, GetFileAttributesA, GetFullPathNameA,
        GetCurrentDirectoryA, SetCurrentDirectoryA, CreateDirectoryA, GetTempPathA,
        CreateFileMappingA, MapViewOfFile, UnmapViewOfFile, FindFirstFileA, FindNextFileA, FindClose, GetLogicalDrives, GetDriveTypeA, FileTimeToSystemTime,
        FileTimeToLocalFileTime, SetEndOfFile, SetErrorMode, GlobalAddAtomA, GlobalDeleteAtom, lstrlenA,
        lstrcpyA, lstrcatA, lstrcmpA, lstrcmpiA, GetPrivateProfileIntA, GetPrivateProfileStringA, WritePrivateProfileStringA,
    )
}

/// Converts a Windows path to the real host path.
pub fn host_path(win: &str) -> PathBuf {
    let mut s = win.replace('\\', "/");
    let mut rooted = false;
    if s.len() >= 2 && s.as_bytes()[1] == b':' {
        s = s[2..].to_string();
        rooted = true;
    }
    if s.starts_with('/') {
        rooted = true;
    }
    let mut cur = if rooted {
        ROOT.get().cloned().unwrap_or_default()
    } else {
        std::env::current_dir().unwrap_or_default()
    };
    for comp in s.split('/').filter(|c| !c.is_empty() && *c != ".") {
        if comp == ".." {
            cur.pop();
            continue;
        }
        let exact = cur.join(comp);
        if exact.exists() {
            cur = exact;
            continue;
        }
        let found = std::fs::read_dir(&cur).ok().and_then(|rd| {
            rd.flatten().find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(comp)).map(|e| e.path())
        });
        cur = found.unwrap_or(exact);
    }
    cur
}

unsafe fn path_arg(p: *const i8) -> PathBuf {
    host_path(&std::ffi::CStr::from_ptr(p).to_string_lossy())
}

fn set_error(e: u32) {
    super::kernel32::set_last_error(e);
}

pub fn file_info(h: u32) -> Option<(u64, bool)> {
    TABLE.lock().unwrap().get(&h).and_then(|f| f.metadata().ok()).map(|m| (m.len(), m.is_dir()))
}

/// Reads up to `n` bytes from a file handle; returns false if the handle is invalid.
pub unsafe fn read_file(h: u32, buf: *mut u8, n: u32, got: &mut u32) -> bool {
    ReadFile(h, buf, n, got, 0) != 0
}

pub fn is_file(h: u32) -> bool {
    TABLE.lock().unwrap().contains_key(&h)
}

pub fn close(h: u32) {
    TABLE.lock().unwrap().remove(&h);
    MAPPINGS.lock().unwrap().remove(&h);
}

pub fn write(h: u32, data: &[u8]) -> Option<u32> {
    let mut t = TABLE.lock().unwrap();
    let f = t.get_mut(&h)?;
    f.write_all(data).ok().map(|_| data.len() as u32)
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn CreateFileA(name: *const i8, access: u32, _share: u32, _sec: u32, disp: u32, _flags: u32, _tmpl: u32) -> u32 {
    let path = path_arg(name);
    let (read, write) = (access & 0x8000_0000 != 0, access & 0x4000_0000 != 0);
    let mut o = OpenOptions::new();
    o.read(read || !write).write(write);
    match disp {
        1 => { o.create_new(true); }                     // CREATE_NEW
        2 => { o.create(true).truncate(true); }          // CREATE_ALWAYS
        4 => { o.create(true); }                         // OPEN_ALWAYS
        5 => { o.truncate(true); }                       // TRUNCATE_EXISTING
        _ => {}                                          // OPEN_EXISTING
    }
    match o.open(&path) {
        Ok(f) => {
            let h = NEXT.fetch_add(4, Ordering::Relaxed);
            TABLE.lock().unwrap().insert(h, f);
            h
        }
        Err(e) => {
            set_error(if e.kind() == std::io::ErrorKind::AlreadyExists { ERROR_ALREADY_EXISTS } else { ERROR_FILE_NOT_FOUND });
            INVALID_HANDLE
        }
    }
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn ReadFile(h: u32, buf: *mut u8, n: u32, read: *mut u32, _ovl: u32) -> i32 {
    let mut t = TABLE.lock().unwrap();
    let Some(f) = t.get_mut(&h) else {
        set_error(ERROR_INVALID_HANDLE);
        return 0;
    };
    let dst = std::slice::from_raw_parts_mut(buf, n as usize);
    let mut total = 0;
    while total < dst.len() {
        match f.read(&mut dst[total..]) {
            Ok(0) | Err(_) => break,
            Ok(k) => total += k,
        }
    }
    if !read.is_null() {
        *read = total as u32;
    }
    1
}

const ERROR_NEGATIVE_SEEK: u32 = 131;

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn SetFilePointer(h: u32, lo: i32, hi: *mut i32, method: u32) -> u32 {
    let mut t = TABLE.lock().unwrap();
    let Some(f) = t.get_mut(&h) else {
        set_error(ERROR_INVALID_HANDLE);
        return u32::MAX;
    };
    let off = if hi.is_null() { lo as i64 } else { ((*hi as i64) << 32) | (lo as u32 as i64) };
    // Target position computed first: before the start of the file is ERROR_NEGATIVE_SEEK (the MSVC
    // runtime relies on this code when opening an empty file in text/append mode).
    let base = match method {
        0 => 0,
        1 => f.stream_position().map(|p| p as i64).unwrap_or(0),
        _ => f.metadata().map(|m| m.len() as i64).unwrap_or(0),
    };
    if base + off < 0 {
        set_error(ERROR_NEGATIVE_SEEK);
        return u32::MAX;
    }
    match f.seek(SeekFrom::Start((base + off) as u64)) {
        Ok(p) => {
            if !hi.is_null() {
                *hi = (p >> 32) as i32;
            }
            set_error(0);
            p as u32
        }
        Err(_) => {
            set_error(ERROR_NEGATIVE_SEEK);
            u32::MAX
        }
    }
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn GetFileSize(h: u32, hi: *mut u32) -> u32 {
    let t = TABLE.lock().unwrap();
    match t.get(&h).and_then(|f| f.metadata().ok()) {
        Some(m) => {
            if !hi.is_null() {
                *hi = (m.len() >> 32) as u32;
            }
            m.len() as u32
        }
        None => u32::MAX,
    }
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn GetFileAttributesA(name: *const i8) -> u32 {
    match std::fs::metadata(path_arg(name)) {
        Ok(m) if m.is_dir() => 0x10,
        Ok(_) => 0x80,
        Err(_) => {
            set_error(ERROR_FILE_NOT_FOUND);
            u32::MAX
        }
    }
}

unsafe fn put_cstr(buf: *mut u8, max: u32, s: &[u8]) -> u32 {
    if buf.is_null() || max == 0 {
        return s.len() as u32 + 1; // required size
    }
    let n = s.len().min(max as usize - 1);
    std::ptr::copy_nonoverlapping(s.as_ptr(), buf, n);
    *buf.add(n) = 0;
    n as u32
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn GetFullPathNameA(name: *const i8, max: u32, buf: *mut u8, filepart: *mut u32) -> u32 {
    let s = std::ffi::CStr::from_ptr(name).to_bytes();
    let n = put_cstr(buf, max, s);
    if !filepart.is_null() && !buf.is_null() {
        let off = s.iter().rposition(|&c| c == b'\\' || c == b'/').map_or(0, |i| i + 1);
        *filepart = buf as u32 + off as u32;
    }
    n
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn GetCurrentDirectoryA(max: u32, buf: *mut u8) -> u32 {
    let root = ROOT.get().cloned().unwrap_or_default();
    let cur = std::env::current_dir().unwrap_or_default();
    let rel = cur.strip_prefix(&root).map(|p| p.to_string_lossy().replace('/', "\\")).unwrap_or_default();
    let s = if rel.is_empty() { "C:\\".to_string() } else { format!("C:\\{rel}") };
    put_cstr(buf, max, s.as_bytes())
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn SetCurrentDirectoryA(name: *const i8) -> i32 {
    std::env::set_current_dir(path_arg(name)).is_ok() as i32
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn CreateDirectoryA(name: *const i8, _sec: u32) -> i32 {
    std::fs::create_dir(path_arg(name)).is_ok() as i32
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn GetTempPathA(max: u32, buf: *mut u8) -> u32 {
    put_cstr(buf, max, b"C:\\TEMP\\")
}

// INI: no real reading/writing yet; returns the default value.
#[allow(non_snake_case)]
unsafe extern "stdcall" fn GetPrivateProfileIntA(_sec: u32, _key: u32, default: i32, _file: u32) -> i32 {
    default
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn GetPrivateProfileStringA(_sec: u32, _key: u32, default: *const i8, buf: *mut u8, max: u32, _file: u32) -> u32 {
    let d = if default.is_null() { &[][..] } else { std::ffi::CStr::from_ptr(default).to_bytes() };
    put_cstr(buf, max, d)
}

#[allow(non_snake_case)]
extern "stdcall" fn WritePrivateProfileStringA(_sec: u32, _key: u32, _val: u32, _file: u32) -> i32 {
    1
}

// ---- file search --------------------------------------------------------------------------

struct Find {
    entries: Vec<std::fs::DirEntry>,
    next: usize,
}

static FINDS: Mutex<BTreeMap<u32, Find>> = Mutex::new(BTreeMap::new());

/// Matching with `*` and `?` wildcards, case-insensitive.
fn wild(pat: &[u8], s: &[u8]) -> bool {
    match (pat.first(), s.first()) {
        (None, None) => true,
        (Some(b'*'), _) => wild(&pat[1..], s) || (!s.is_empty() && wild(pat, &s[1..])),
        (Some(b'?'), Some(_)) => wild(&pat[1..], &s[1..]),
        (Some(a), Some(b)) if a.eq_ignore_ascii_case(b) => wild(&pat[1..], &s[1..]),
        _ => false,
    }
}

unsafe fn fill_find_data(out: *mut u8, e: &std::fs::DirEntry) {
    std::ptr::write_bytes(out, 0, 320);
    let Ok(m) = e.metadata() else { return };
    *(out as *mut u32) = if m.is_dir() { 0x10 } else { 0x80 };
    // FILETIME: 100 ns since 1601
    let ft = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| {
        d.as_secs() * 10_000_000 + 116_444_736_000_000_000
    });
    *(out.add(20) as *mut u32) = ft as u32;
    *(out.add(24) as *mut u32) = (ft >> 32) as u32;
    *(out.add(28) as *mut u32) = (m.len() >> 32) as u32;
    *(out.add(32) as *mut u32) = m.len() as u32;
    let name = e.file_name().to_string_lossy().into_owned();
    let n = name.len().min(259);
    std::ptr::copy_nonoverlapping(name.as_ptr(), out.add(44), n);
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn FindFirstFileA(pattern: *const i8, out: *mut u8) -> u32 {
    let full = std::ffi::CStr::from_ptr(pattern).to_string_lossy().replace('\\', "/");
    let (dir, pat) = match full.rfind('/') {
        Some(i) => (full[..=i].to_string(), full[i + 1..].to_string()),
        None => (String::new(), full),
    };
    let dir_path = if dir.is_empty() { std::env::current_dir().unwrap_or_default() } else { host_path(&dir) };
    let Ok(rd) = std::fs::read_dir(dir_path) else {
        set_error(3); // ERROR_PATH_NOT_FOUND
        return INVALID_HANDLE;
    };
    let mut entries: Vec<_> = rd.flatten().filter(|e| wild(pat.as_bytes(), e.file_name().to_string_lossy().as_bytes())).collect();
    entries.sort_by_key(|e| e.file_name());
    if entries.is_empty() {
        set_error(ERROR_FILE_NOT_FOUND);
        return INVALID_HANDLE;
    }
    fill_find_data(out, &entries[0]);
    let h = NEXT.fetch_add(4, Ordering::Relaxed);
    FINDS.lock().unwrap().insert(h, Find { entries, next: 1 });
    h
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn FindNextFileA(h: u32, out: *mut u8) -> i32 {
    let mut t = FINDS.lock().unwrap();
    let Some(f) = t.get_mut(&h) else { return 0 };
    match f.entries.get(f.next) {
        Some(e) => {
            fill_find_data(out, e);
            f.next += 1;
            1
        }
        None => {
            set_error(18); // ERROR_NO_MORE_FILES
            0
        }
    }
}

#[allow(non_snake_case)]
extern "stdcall" fn FindClose(h: u32) -> i32 {
    FINDS.lock().unwrap().remove(&h).is_some() as i32
}

#[allow(non_snake_case)]
extern "stdcall" fn GetLogicalDrives() -> u32 {
    0x4 // C: only
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetDriveTypeA(_root: u32) -> u32 {
    3 // DRIVE_FIXED
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn FileTimeToLocalFileTime(src: *const u64, dst: *mut u64) -> i32 {
    *dst = *src; // no time zone
    1
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn FileTimeToSystemTime(ft: *const u64, st: *mut u16) -> i32 {
    let total = *ft;
    let secs = (total / 10_000_000) as i64 - 11_644_473_600; // since 1970
    let ms = (total / 10_000 % 1000) as u16;
    let (days, rem) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + (month <= 2) as i64;
    let v = [year, month, (days + 4).rem_euclid(7), day, rem / 3600, rem % 3600 / 60, rem % 60, ms as i64];
    for (i, x) in v.iter().enumerate() {
        *st.add(i) = *x as u16;
    }
    1
}

#[allow(non_snake_case)]
extern "stdcall" fn SetEndOfFile(h: u32) -> i32 {
    let mut t = TABLE.lock().unwrap();
    let Some(f) = t.get_mut(&h) else { return 0 };
    let pos = f.stream_position().unwrap_or(0);
    f.set_len(pos).is_ok() as i32
}

#[allow(non_snake_case)]
extern "stdcall" fn SetErrorMode(_mode: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
extern "stdcall" fn GlobalAddAtomA(_name: u32) -> u32 {
    0xC001
}

#[allow(non_snake_case)]
extern "stdcall" fn GlobalDeleteAtom(_atom: u32) -> u32 {
    0
}

// ---- lstr* ---------------------------------------------------------------------------------

#[allow(non_snake_case)]
unsafe extern "stdcall" fn lstrlenA(s: *const i8) -> i32 {
    if s.is_null() { 0 } else { std::ffi::CStr::from_ptr(s).to_bytes().len() as i32 }
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn lstrcpyA(dst: *mut u8, src: *const u8) -> *mut u8 {
    let n = std::ffi::CStr::from_ptr(src as *const i8).to_bytes().len();
    std::ptr::copy(src, dst, n + 1);
    dst
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn lstrcatA(dst: *mut u8, src: *const u8) -> *mut u8 {
    let at = std::ffi::CStr::from_ptr(dst as *const i8).to_bytes().len();
    lstrcpyA(dst.add(at), src);
    dst
}

fn cmp(a: &[u8], b: &[u8], fold: bool) -> i32 {
    let (x, y): (Vec<u8>, Vec<u8>) = if fold { (a.to_ascii_lowercase(), b.to_ascii_lowercase()) } else { (a.to_vec(), b.to_vec()) };
    match x.cmp(&y) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn lstrcmpA(a: *const i8, b: *const i8) -> i32 {
    cmp(std::ffi::CStr::from_ptr(a).to_bytes(), std::ffi::CStr::from_ptr(b).to_bytes(), false)
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn lstrcmpiA(a: *const i8, b: *const i8) -> i32 {
    cmp(std::ffi::CStr::from_ptr(a).to_bytes(), std::ffi::CStr::from_ptr(b).to_bytes(), true)
}

// ---- file mapping -------------------------------------------------------------------------

/// Mapping object: contents read at creation (a copy; writes to the view do not go back to the file).
struct Mapping {
    data: Vec<u8>,
}

static MAPPINGS: Mutex<BTreeMap<u32, Mapping>> = Mutex::new(BTreeMap::new());
/// Active views: address -> size (for munmap).
static VIEWS: Mutex<BTreeMap<u32, usize>> = Mutex::new(BTreeMap::new());

extern "C" {
    fn mmap(addr: *mut std::ffi::c_void, len: usize, prot: i32, flags: i32, fd: i32, off: i64) -> *mut std::ffi::c_void;
    fn munmap(addr: *mut std::ffi::c_void, len: usize) -> i32;
}

#[allow(non_snake_case)]
extern "stdcall" fn CreateFileMappingA(file: u32, _attr: u32, _protect: u32, size_hi: u32, size_lo: u32, _name: u32) -> u32 {
    use std::os::unix::fs::FileExt;
    let want = ((size_hi as u64) << 32 | size_lo as u64) as usize;
    let data = if file == INVALID_HANDLE {
        vec![0u8; want] // anonymous memory (shared in name only)
    } else {
        let t = TABLE.lock().unwrap();
        let Some(f) = t.get(&file) else {
            set_error(ERROR_INVALID_HANDLE);
            return 0;
        };
        let len = if want != 0 { want } else { f.metadata().map_or(0, |m| m.len() as usize) };
        let mut buf = vec![0u8; len];
        let mut got = 0;
        while got < len {
            match f.read_at(&mut buf[got..], got as u64) {
                Ok(0) | Err(_) => break,
                Ok(n) => got += n,
            }
        }
        buf
    };
    let h = NEXT.fetch_add(4, Ordering::Relaxed);
    MAPPINGS.lock().unwrap().insert(h, Mapping { data });
    h
}

#[allow(non_snake_case)]
extern "stdcall" fn MapViewOfFile(h: u32, _access: u32, _off_hi: u32, off_lo: u32, bytes: usize) -> u32 {
    let maps = MAPPINGS.lock().unwrap();
    let Some(m) = maps.get(&h) else {
        set_error(ERROR_INVALID_HANDLE);
        return 0;
    };
    let off = off_lo as usize;
    let len = if bytes != 0 { bytes } else { m.data.len().saturating_sub(off) };
    let size = (len.max(1) + 4095) & !4095;
    // 3 = PROT_READ|WRITE; 0x22 = MAP_PRIVATE|MAP_ANONYMOUS
    let p = unsafe { mmap(std::ptr::null_mut(), size, 7, 0x22, -1, 0) };
    if p as isize == -1 {
        return 0;
    }
    let n = m.data.len().saturating_sub(off).min(len);
    unsafe { std::ptr::copy_nonoverlapping(m.data.as_ptr().add(off), p as *mut u8, n) };
    VIEWS.lock().unwrap().insert(p as usize as u32, size);
    p as usize as u32
}

#[allow(non_snake_case)]
extern "stdcall" fn UnmapViewOfFile(addr: u32) -> i32 {
    match VIEWS.lock().unwrap().remove(&addr) {
        Some(size) => {
            unsafe { munmap(addr as usize as *mut std::ffi::c_void, size) };
            1
        }
        None => 0,
    }
}
