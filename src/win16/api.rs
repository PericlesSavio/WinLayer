//! Registration and dispatch of the Windows 3.x APIs called by 16-bit code.
//!
//! Each imported function gets an id and a stub in the thunk segment (see `trans`). When
//! called, `dispatch` finds the handler, reads the arguments (PASCAL convention: the last argument
//! is at the lowest address) and returns the result in AX (or DX:AX).

use super::{ldt, ordinals, trans};
use std::sync::Mutex;

/// API handler: reads arguments via `Call` and returns AX (low) / DX (high).
pub type Handler = fn(&mut Call) -> u32;

pub struct Impl {
    pub module: &'static str,
    pub name: &'static str,
    /// Argument signature, left to right: `w` word, `l` dword, `p` far
    /// pointer (4 bytes; `Call::ptr` converts to linear).
    pub sig: &'static str,
    pub f: Handler,
}

struct Slot {
    module: String,
    name: String,
    ordinal: u16,
    sig: &'static str,
    f: Option<Handler>,
}

static SLOTS: Mutex<Vec<Slot>> = Mutex::new(Vec::new());

fn find(module: &str, name: &str) -> Option<&'static Impl> {
    [super::kernel::IMPLS, super::kernel_ext::IMPLS, super::user::IMPLS, super::gdi::IMPLS].iter().flat_map(|t| t.iter()).find(|i| i.module == module && i.name == name)
}

/// Absolute constants exported by ordinal (they apply as the fixup offset, selector 0).
pub fn absolute(module: &str, ordinal: u16) -> Option<u16> {
    match (module, ordinal) {
        ("KERNEL", 113) => Some(3),      // __AHSHIFT
        ("KERNEL", 114) => Some(8),      // __AHINCR
        ("KERNEL", 178) => Some(0x0429), // __WINFLAGS
        _ => None,
    }
}

/// Creates (or reuses) the stub for `module.ordinal` and returns the thunk's (selector, offset).
pub fn resolve_import(thunks: &mut trans::Thunks, cache: &mut Vec<((String, u16), u16)>, module: &str, ordinal: u16) -> (u16, u16) {
    if let Some(v) = absolute(module, ordinal) {
        return (0, v);
    }
    let key = (module.to_string(), ordinal);
    if let Some((_, off)) = cache.iter().find(|(k, _)| *k == key) {
        return (thunks.sel, *off);
    }
    let fallback = format!("ORD{ordinal}");
    let name = ordinals::name_of(module, ordinal).unwrap_or(&fallback);
    let imp = find(module, name);
    let mut slots = SLOTS.lock().unwrap();
    let id = slots.len() as u16;
    slots.push(Slot { module: module.to_string(), name: name.to_string(), ordinal, sig: imp.map_or("", |i| i.sig), f: imp.map(|i| i.f) });
    drop(slots);
    let off = thunks.add_api(id);
    cache.push((key, off));
    (thunks.sel, off)
}

/// List of (module.name, implemented?) for everything the program imports — diagnostics.
pub fn report() -> Vec<(String, bool)> {
    SLOTS.lock().unwrap().iter().map(|s| (format!("{}.{}({})", s.module, s.name, s.ordinal), s.f.is_some())).collect()
}

/// Arguments and registers of a call in progress.
pub struct Call {
    frame: *mut u8,
    sig: &'static str,
}

fn size_of(c: char) -> usize {
    if c == 'w' { 2 } else { 4 }
}

impl Call {
    fn rd16(&self, o: usize) -> u16 {
        unsafe { std::ptr::read_unaligned(self.frame.add(o) as *const u16) }
    }
    fn rd32(&self, o: usize) -> u32 {
        unsafe { std::ptr::read_unaligned(self.frame.add(o) as *const u32) }
    }
    fn wr32(&mut self, o: usize, v: u32) {
        unsafe { std::ptr::write_unaligned(self.frame.add(o) as *mut u32, v) }
    }

    /// Offset (in the frame) of argument `i` (0 = leftmost).
    fn arg_off(&self, i: usize) -> usize {
        trans::FRAME_ARGS + self.sig.chars().skip(i + 1).map(size_of).sum::<usize>()
    }

    /// Argument bytes (what `retf N` discards).
    pub fn argn(&self) -> u32 {
        self.sig.chars().map(size_of).sum::<usize>() as u32
    }

    /// 16-bit argument.
    pub fn w(&self, i: usize) -> u16 {
        self.rd16(self.arg_off(i))
    }

    /// Signed 16-bit argument.
    pub fn s(&self, i: usize) -> i16 {
        self.w(i) as i16
    }

    /// Raw 32-bit argument (dword or far pointer as `seg:off`).
    pub fn l(&self, i: usize) -> u32 {
        self.rd32(self.arg_off(i))
    }

    /// Far pointer -> linear address (0 if null or invalid selector).
    pub fn ptr(&self, i: usize) -> u32 {
        far_to_linear(self.l(i))
    }

    /// Value of a saved register (e.g. BX in an API that takes arguments in registers).
    pub fn set_ecx(&mut self, v: u16) {
        self.wr32(trans::FRAME_ECX, v as u32)
    }
    pub fn set_ebx(&mut self, v: u16) {
        self.wr32(trans::FRAME_EBX, v as u32)
    }
    pub fn set_esi(&mut self, v: u16) {
        self.wr32(trans::FRAME_ESI, v as u32)
    }
    pub fn set_edi(&mut self, v: u16) {
        self.wr32(trans::FRAME_EDI, v as u32)
    }
    pub fn set_es(&mut self, v: u16) {
        self.wr32(trans::FRAME_ES, v as u32)
    }
}

/// `seg:off` (dword with the offset in the low part) -> linear; 0 if null or unknown selector.
pub fn far_to_linear(p: u32) -> u32 {
    let (off, sel) = (p as u16, (p >> 16) as u16);
    if sel == 0 {
        return 0;
    }
    match ldt::lookup(sel) {
        Some(seg) if (off as u32) <= seg.limit => seg.base.wrapping_add(off as u32),
        Some(seg) => {
            eprintln!("[win16] WARNING: far pointer {sel:04x}:{off:04x} beyond the segment limit ({:#x})", seg.limit);
            0
        }
        None => {
            eprintln!("[win16] WARNING: far pointer {sel:04x}:{off:04x} with unknown selector");
            0
        }
    }
}

/// Packs selector and offset into a far pointer.
pub fn far(sel: u16, off: u16) -> u32 {
    ((sel as u32) << 16) | off as u32
}

fn trace_on() -> bool {
    std::env::var_os("WINLAYER_WIN16_TRACE").is_some()
}

/// Called by the assembly (`w16_api_entry`) when 16-bit code invokes an API.
pub extern "C" fn dispatch(sp16: u32) {
    crate::dbgalloc::protect(false);
    struct Reprotect;
    impl Drop for Reprotect {
        fn drop(&mut self) {
            crate::dbgalloc::protect(true);
        }
    }
    let _reprotect = Reprotect;
    // Nested calls (callbacks to 16-bit code) must stay below this call's frame.
    super::rt().stack.cur_sp = (sp16 - 16) & !1;
    let frame = trans::frame_ptr(sp16);
    let id = unsafe { std::ptr::read_unaligned(frame.add(trans::FRAME_ID) as *const u16) } as usize;
    let (module, name, ordinal, sig, f) = {
        let s = SLOTS.lock().unwrap();
        let sl = &s[id];
        (sl.module.clone(), sl.name.clone(), sl.ordinal, sl.sig, sl.f)
    };
    let mut call = Call { frame, sig };
    if trace_on() {
        let words: Vec<String> = (0..sig.len().max(1) * 2).map(|k| format!("{:04x}", call.rd16(trans::FRAME_ARGS + k * 2))).collect();
        let head: Vec<String> = (0..4).map(|k| format!("{:04x}", call.rd16(trans::FRAME_ID + k * 2))).collect();
        eprintln!("[win16] {module}.{name}({ordinal}) id/ip/cs=[{}] args(stack)=[{}] sp16={sp16:#x}", head.join(" "), words.join(" "));
    }
    let Some(f) = f else {
        eprintln!("\n[win16] UNIMPLEMENTED API: {module}.{name} (ordinal {ordinal})");
        crate::win32::kernel32::exit_process(3);
    };
    let r = f(&mut call);
    if crate::dbgalloc::check_all(&format!("after {module}.{name}")) > 0 {
        crate::win32::kernel32::exit_process(9);
    }
    if trace_on() {
        eprintln!("[win16]   -> dx:ax = {:04x}:{:04x}", r >> 16, r as u16);
    }
    if std::env::var_os("WINLAYER_VALIDATE").is_some() {
        super::kernel::validate();
    }
    unsafe {
        std::ptr::write_unaligned(frame.add(trans::FRAME_EAX) as *mut u16, r as u16);
        std::ptr::write_unaligned(frame.add(trans::FRAME_EDX) as *mut u16, (r >> 16) as u16);
    }
    trans::set_argn(call.argn());
}
