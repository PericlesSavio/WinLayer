//! Windows 3.x KERNEL.EXE: task startup, global/local memory, modules and files.

use super::api::{far, Call, Impl};
use super::ldt::{self, Kind};
use super::trans::map_rwx;
use std::collections::BTreeMap;
use std::ffi::c_void;
use std::sync::Mutex;

extern "C" {
    fn munmap(addr: *mut c_void, len: usize) -> i32;
}

/// Data of the running Win16 process.
pub struct Process {
    pub dgroup_sel: u16,
    pub dgroup_base: u32,
    pub psp_sel: u16,
    pub exe_name: String,
    pub stack_size: u16,
    pub heap_size: u16,
    pub env_sel: u16,
}

static PROCESS: Mutex<Option<Process>> = Mutex::new(None);

pub fn set_process(p: Process) {
    *PROCESS.lock().unwrap() = Some(p);
}

pub fn with_process<R>(f: impl FnOnce(&Process) -> R) -> R {
    f(PROCESS.lock().unwrap().as_ref().expect("Win16 process not initialized"))
}

// ---- global memory -----------------------------------------------------------------------

struct Block {
    base: u32,
    size: u32,
    chunks: u32,
}

static BLOCKS: Mutex<BTreeMap<u16, Block>> = Mutex::new(BTreeMap::new());

/// Allocates a zeroed global block; the "handle" is the selector (blocks > 64 KiB use a run of selectors).
pub fn global_alloc(size: u32) -> Option<u16> {
    let size = size.max(1);
    let mem = map_rwx((size as usize).div_ceil(4096) * 4096);
    if mem as isize == -1 {
        return None;
    }
    let base = mem as usize as u32;
    // Windows 3.1 rounded global blocks up to 16-byte paragraphs: the selector limit includes the slack
    // (programs read one byte past the end, e.g. sum/copy loops).
    let limit_size = size.checked_add(15).map_or(size, |v| v & !15);
    let sel = ldt::alloc_run(base, limit_size, Kind::Data16).ok()?;
    BLOCKS.lock().unwrap().insert(sel, Block { base, size, chunks: limit_size.div_ceil(0x1_0000) });
    Some(sel)
}

pub fn global_info(sel: u16) -> Option<(u32, u32)> {
    BLOCKS.lock().unwrap().get(&sel).map(|b| (b.base, b.size))
}

/// Frees a global block (for use by other parts of the Win16 layer).
pub fn global_free_pub(sel: u16) -> bool {
    global_free(sel)
}

fn global_free(sel: u16) -> bool {
    let Some(b) = BLOCKS.lock().unwrap().remove(&sel) else { return false };
    ldt::release_run(sel, b.chunks);
    if std::env::var_os("WINLAYER_NO_MUNMAP").is_some() {
        return true;
    }
    unsafe { munmap(b.base as usize as *mut c_void, (b.size as usize).div_ceil(4096) * 4096) };
    true
}

// ---- local heap (inside DGROUP) ----------------------------------------------------------

struct LocalHeap {
    base: u32,
    /// (offset, size, in use)
    blocks: Vec<(u16, u16, bool)>,
    start: u16,
    end: u16,
}

static LOCAL: Mutex<Option<LocalHeap>> = Mutex::new(None);

fn local_alloc(bytes: u16, zero: bool) -> u16 {
    let mut g = LOCAL.lock().unwrap();
    let Some(h) = g.as_mut() else { return 0 };
    let need = (bytes.max(1) + 3) & !3;
    // first free block that fits
    if let Some(i) = h.blocks.iter().position(|b| !b.2 && b.1 >= need) {
        let (off, size, _) = h.blocks[i];
        if size > need + 8 {
            h.blocks[i] = (off, need, true);
            h.blocks.insert(i + 1, (off + need, size - need, false));
        } else {
            h.blocks[i].2 = true;
        }
        if zero {
            unsafe { std::ptr::write_bytes((h.base + off as u32) as usize as *mut u8, 0, need as usize) };
        }
        return off;
    }
    0
}

fn local_free(off: u16) -> bool {
    let mut g = LOCAL.lock().unwrap();
    let Some(h) = g.as_mut() else { return false };
    let Some(i) = h.blocks.iter().position(|b| b.0 == off && b.2) else { return false };
    h.blocks[i].2 = false;
    // merges free neighbors
    let mut k = 0;
    while k + 1 < h.blocks.len() {
        if !h.blocks[k].2 && !h.blocks[k + 1].2 {
            h.blocks[k].1 += h.blocks[k + 1].1;
            h.blocks.remove(k + 1);
        } else {
            k += 1;
        }
    }
    true
}

fn local_size(off: u16) -> u16 {
    LOCAL.lock().unwrap().as_ref().and_then(|h| h.blocks.iter().find(|b| b.0 == off && b.2).map(|b| b.1)).unwrap_or(0)
}

// ---- handlers -----------------------------------------------------------------------------

const GMEM_ZEROINIT: u16 = 0x40;
const LMEM_ZEROINIT: u16 = 0x40;

fn pack(ax: u16, dx: u16) -> u32 {
    ax as u32 | (dx as u32) << 16
}

/// INITTASK: returns ES:BX = command line (in the PSP), CX = stack limit, DX = nCmdShow,
/// SI = hPrevInstance, DI = hInstance; AX != 0 indicates success.
fn init_task(c: &mut Call) -> u32 {
    with_process(|p| {
        c.set_es(p.psp_sel);
        c.set_ebx(0x81);
        c.set_ecx(p.stack_size);
        c.set_esi(0);
        c.set_edi(p.dgroup_sel);
    });
    pack(1, 1) // AX = 1, DX = SW_SHOWNORMAL
}

fn wait_event(_c: &mut Call) -> u32 {
    0
}

fn get_version(_c: &mut Call) -> u32 {
    pack(0x0A03, 0x0616) // Windows 3.10; DOS 6.22
}

fn local_init(c: &mut Call) -> u32 {
    let (start, end) = (c.w(1), c.w(2));
    let base = with_process(|p| p.dgroup_base);
    let end = if end == 0 { 0xFFFE } else { end };
    *LOCAL.lock().unwrap() = Some(LocalHeap { base, blocks: vec![(start, end.saturating_sub(start), false)], start, end });
    1
}

fn local_alloc_h(c: &mut Call) -> u32 {
    local_alloc(c.w(1), c.w(0) & LMEM_ZEROINIT != 0) as u32
}

fn local_realloc_h(c: &mut Call) -> u32 {
    let (h, bytes) = (c.w(0), c.w(1));
    if h == 0 {
        return local_alloc(bytes, true) as u32;
    }
    let old = local_size(h);
    if bytes <= old {
        return h as u32;
    }
    let n = local_alloc(bytes, true);
    if n != 0 {
        let base = with_process(|p| p.dgroup_base);
        unsafe { std::ptr::copy_nonoverlapping((base + h as u32) as usize as *const u8, (base + n as u32) as usize as *mut u8, old as usize) };
        local_free(h);
    }
    n as u32
}

fn local_free_h(c: &mut Call) -> u32 {
    (!local_free(c.w(0))) as u32 // 0 = success
}

fn local_lock(c: &mut Call) -> u32 {
    c.w(0) as u32
}

fn local_size_h(c: &mut Call) -> u32 {
    local_size(c.w(0)) as u32
}

fn local_compact(_c: &mut Call) -> u32 {
    LOCAL.lock().unwrap().as_ref().map_or(0, |h| h.blocks.iter().filter(|b| !b.2).map(|b| b.1).max().unwrap_or(0) as u32)
}

fn zero(_c: &mut Call) -> u32 {
    0
}

fn global_alloc_h(c: &mut Call) -> u32 {
    let (_flags, size) = (c.w(0), c.l(1));
    global_alloc(size).map_or(0, |s| s as u32)
}

fn global_realloc_h(c: &mut Call) -> u32 {
    let (h, size, flags) = (c.w(0), c.l(1), c.w(2));
    let Some((base, old)) = global_info(h) else { return 0 };
    let (rsize, rold) = ((size + 15) & !15, (old + 15) & !15); // limits rounded to paragraphs
    if rsize.div_ceil(0x1_0000) != rold.div_ceil(0x1_0000) {
        // Changes the number of selectors: returns a new block (the handle changes).
        let Some(n) = global_alloc(size) else { return 0 };
        let (nb, _) = global_info(n).unwrap();
        unsafe { std::ptr::copy_nonoverlapping(base as usize as *const u8, nb as usize as *mut u8, old.min(size) as usize) };
        global_free(h);
        eprintln!("[win16] warning: GlobalReAlloc changed selector ({h:#x} -> {n:#x})");
        return n as u32;
    }
    let mem = map_rwx((size.max(1) as usize).div_ceil(4096) * 4096);
    unsafe { std::ptr::copy_nonoverlapping(base as usize as *const u8, mem, old.min(size) as usize) };
    let nbase = mem as usize as u32;
    for i in 0..rold.div_ceil(0x1_0000).max(1) {
        let len = if (i + 1) * 0x1_0000 >= rsize { rsize - i * 0x1_0000 } else { 0x1_0000 };
        let _ = ldt::update(h.wrapping_add((i * 8) as u16), nbase + i * 0x1_0000, len.max(1) - 1);
    }
    unsafe { munmap(base as usize as *mut c_void, (old as usize).div_ceil(4096) * 4096) };
    BLOCKS.lock().unwrap().insert(h, Block { base: nbase, size, chunks: rsize.div_ceil(0x1_0000) });
    let _ = (flags, GMEM_ZEROINIT);
    h as u32
}

fn global_free_h(c: &mut Call) -> u32 {
    (!global_free(c.w(0))) as u32 // 0 = success
}

fn global_lock(c: &mut Call) -> u32 {
    let h = c.w(0);
    if global_info(h).is_some() { pack(0, h) } else { 0 } // DX:AX = selector:0
}

fn global_size(c: &mut Call) -> u32 {
    global_info(c.w(0)).map_or(0, |(_, s)| s)
}

fn global_handle(c: &mut Call) -> u32 {
    let sel = c.w(0);
    pack(sel, sel) // LOWORD = handle, HIWORD = selector
}

fn global_compact(_c: &mut Call) -> u32 {
    0x0080_0000 // 8 MiB free (the value is only informative)
}

fn get_free_space(_c: &mut Call) -> u32 {
    0x0080_0000
}

fn lock_segment(_c: &mut Call) -> u32 {
    with_process(|p| p.dgroup_sel as u32)
}

fn get_win_flags(_c: &mut Call) -> u32 {
    0x0429
}

fn get_module_file_name(c: &mut Call) -> u32 {
    let (buf, size) = (c.ptr(1), c.w(2) as usize);
    let name = with_process(|p| format!("C:\\{}", p.exe_name));
    let n = name.len().min(size.saturating_sub(1));
    if buf != 0 {
        unsafe {
            std::ptr::copy_nonoverlapping(name.as_ptr(), buf as usize as *mut u8, n);
            *((buf as usize + n) as *mut u8) = 0;
        }
    }
    n as u32
}

fn make_proc_instance(c: &mut Call) -> u32 {
    c.l(0) // the procedure is called with the correct DS, so the far ptr itself serves as the "instance"
}

fn free_proc_instance(_c: &mut Call) -> u32 {
    0
}

fn get_dos_environment(_c: &mut Call) -> u32 {
    with_process(|p| pack(0, p.env_sel))
}

fn fatal_exit(c: &mut Call) -> u32 {
    eprintln!("[win16] FatalExit({:#x})", c.w(0));
    crate::win32::kernel32::exit_process(1)
}

fn get_profile_int(c: &mut Call) -> u32 {
    c.w(2) as u32 // default value: WIN.INI is not read yet
}

fn lstrlen(c: &mut Call) -> u32 {
    let p = c.ptr(0);
    if p == 0 { 0 } else { unsafe { std::ffi::CStr::from_ptr(p as usize as *const i8).to_bytes().len() as u32 } }
}

fn lstrcpy(c: &mut Call) -> u32 {
    let (dst, src) = (c.ptr(0), c.ptr(1));
    if dst != 0 && src != 0 {
        unsafe {
            let n = std::ffi::CStr::from_ptr(src as usize as *const i8).to_bytes().len();
            std::ptr::copy(src as usize as *const u8, dst as usize as *mut u8, n + 1);
        }
    }
    c.l(0) // returns the destination pointer (far)
}

fn hmemcpy(c: &mut Call) -> u32 {
    let (dst, src, n) = (c.ptr(0), c.ptr(1), c.l(2));
    if dst != 0 && src != 0 {
        unsafe { std::ptr::copy(src as usize as *const u8, dst as usize as *mut u8, n as usize) };
    }
    0
}

pub static IMPLS: &[Impl] = &[
    Impl { module: "KERNEL", name: "INITTASK", sig: "", f: init_task },
    Impl { module: "KERNEL", name: "WAITEVENT", sig: "w", f: wait_event },
    Impl { module: "KERNEL", name: "GETVERSION", sig: "", f: get_version },
    Impl { module: "KERNEL", name: "LOCALINIT", sig: "www", f: local_init },
    Impl { module: "KERNEL", name: "LOCALALLOC", sig: "ww", f: local_alloc_h },
    Impl { module: "KERNEL", name: "LOCALREALLOC", sig: "www", f: local_realloc_h },
    Impl { module: "KERNEL", name: "LOCALFREE", sig: "w", f: local_free_h },
    Impl { module: "KERNEL", name: "LOCALLOCK", sig: "w", f: local_lock },
    Impl { module: "KERNEL", name: "LOCALUNLOCK", sig: "w", f: zero },
    Impl { module: "KERNEL", name: "LOCALSIZE", sig: "w", f: local_size_h },
    Impl { module: "KERNEL", name: "LOCALHANDLE", sig: "w", f: local_lock },
    Impl { module: "KERNEL", name: "LOCALFLAGS", sig: "w", f: zero },
    Impl { module: "KERNEL", name: "LOCALCOMPACT", sig: "w", f: local_compact },
    Impl { module: "KERNEL", name: "GLOBALALLOC", sig: "wl", f: global_alloc_h },
    Impl { module: "KERNEL", name: "GLOBALREALLOC", sig: "wlw", f: global_realloc_h },
    Impl { module: "KERNEL", name: "GLOBALFREE", sig: "w", f: global_free_h },
    Impl { module: "KERNEL", name: "GLOBALLOCK", sig: "w", f: global_lock },
    Impl { module: "KERNEL", name: "GLOBALUNLOCK", sig: "w", f: zero },
    Impl { module: "KERNEL", name: "GLOBALSIZE", sig: "w", f: global_size },
    Impl { module: "KERNEL", name: "GLOBALHANDLE", sig: "w", f: global_handle },
    Impl { module: "KERNEL", name: "GLOBALFLAGS", sig: "w", f: zero },
    Impl { module: "KERNEL", name: "GLOBALCOMPACT", sig: "l", f: global_compact },
    Impl { module: "KERNEL", name: "GETFREESPACE", sig: "w", f: get_free_space },
    Impl { module: "KERNEL", name: "LOCKSEGMENT", sig: "w", f: lock_segment },
    Impl { module: "KERNEL", name: "UNLOCKSEGMENT", sig: "w", f: zero },
    Impl { module: "KERNEL", name: "GETWINFLAGS", sig: "", f: get_win_flags },
    Impl { module: "KERNEL", name: "GETMODULEFILENAME", sig: "wpw", f: get_module_file_name },
    Impl { module: "KERNEL", name: "MAKEPROCINSTANCE", sig: "lw", f: make_proc_instance },
    Impl { module: "KERNEL", name: "FREEPROCINSTANCE", sig: "l", f: free_proc_instance },
    Impl { module: "KERNEL", name: "GETDOSENVIRONMENT", sig: "", f: get_dos_environment },
    Impl { module: "KERNEL", name: "FATALEXIT", sig: "w", f: fatal_exit },
    Impl { module: "KERNEL", name: "GETPROFILEINT", sig: "ppw", f: get_profile_int },
    Impl { module: "KERNEL", name: "LSTRLEN", sig: "p", f: lstrlen },
    Impl { module: "KERNEL", name: "LSTRCPY", sig: "pp", f: lstrcpy },
    Impl { module: "KERNEL", name: "HMEMCPY", sig: "ppl", f: hmemcpy },
];

/// Creates the process's PSP and DOS environment and registers the process data.
pub fn setup(dgroup_sel: u16, dgroup_base: u32, exe_name: &str, cmdline: &str, stack: u16, heap: u16) -> Result<(), String> {
    let psp = global_alloc(256).ok_or("out of memory for the PSP")?;
    let (pbase, _) = global_info(psp).unwrap();
    let cmd = cmdline.as_bytes();
    unsafe {
        let p = pbase as usize as *mut u8;
        *p = 0xCD; // INT 20h
        *p.add(1) = 0x20;
        let n = cmd.len().min(126);
        *p.add(0x80) = n as u8;
        std::ptr::copy_nonoverlapping(cmd.as_ptr(), p.add(0x81), n);
        *p.add(0x81 + n) = 0x0D;
    }
    let env = global_alloc(256).ok_or("out of memory for the environment")?;
    let (ebase, _) = global_info(env).unwrap();
    unsafe {
        let e = ebase as usize as *mut u8;
        // empty environment block + count + program name
        *e.add(2) = 1;
        let full = format!("C:\\{exe_name}");
        std::ptr::copy_nonoverlapping(full.as_ptr(), e.add(4), full.len());
    }
    set_process(Process { dgroup_sel, dgroup_base, psp_sel: psp, exe_name: exe_name.to_string(), stack_size: stack, heap_size: heap, env_sel: env });
    let _ = far;
    Ok(())
}

/// Debugging: walks the internal tables (a corrupted BTreeMap breaks here, right after the guilty API).
pub fn validate() -> usize {
    let n = {
        let b = BLOCKS.lock().unwrap();
        b.values().map(|b| b.size as usize & 0).sum::<usize>() + b.len()
    };
    n + super::ldt::count()
}
