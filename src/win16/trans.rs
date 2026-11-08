//! Transitions between 32-bit code (winlayer) and 16-bit code (Win16 programs).
//!
//! 32 -> 16: `call16` pushes the arguments onto the 16-bit stack, switches SS:ESP atomically (`lss`)
//!           and jumps to the procedure.
//! 16 -> 32: each API is a stub (`push id; jmp far`) in a 16-bit segment that lands in
//!           `w16_api_entry`: saves registers, returns to the host stack and calls `w16_dispatch`.

use super::ldt::{self, Kind};
use std::ffi::c_void;

extern "C" {
    fn mprotect(addr: *mut c_void, len: usize, prot: i32) -> i32;
    fn mmap(addr: *mut c_void, len: usize, prot: i32, flags: i32, fd: i32, off: i64) -> *mut c_void;
    fn w16_api_entry();
    fn w16_ret_entry();
    fn w16_call(off: u32, cs: u32, ds: u32, sp: u32, ss: u32);
}

// Globals read and written by the assembly (hence `static mut` and simple types).
#[no_mangle] static mut W16_DS32: u32 = 0;
#[no_mangle] static mut W16_CS32: u32 = 0;
#[no_mangle] static mut W16_HOST_STACK: [u32; 2] = [0; 2]; // host esp, ss (for `lss`)
#[no_mangle] static mut W16_SAVED16: [u32; 2] = [0; 2];    // sp, ss of the 16-bit stack at the moment of the API
#[no_mangle] static mut W16_NEWSTACK: [u32; 2] = [0; 2];
#[no_mangle] static mut W16_FPTR: [u32; 2] = [0; 2];       // far pointer (off32, sel16) of the target
#[no_mangle] static mut W16_ARGN: u32 = 0;                 // argument bytes to discard on return
#[no_mangle] static mut W16_RET: [u32; 2] = [0; 2];        // AX, DX returned by a 32->16 call
/// Dedicated page (write-protected after `init`): if 16-bit code writes here by
/// mistake, the SIGSEGV shows the culprit's CS:IP instead of a mysterious crash later.
#[repr(C, align(4096))]
struct DispatchPage {
    dispatch: usize,
    _pad: [u8; 4092],
}

#[no_mangle]
static mut W16_DISPATCH: DispatchPage = DispatchPage { dispatch: 0, _pad: [0; 4092] };
/// Registers handed to the called procedure: ax, bx, cx, dx, si, di, es (4 bytes each).
#[no_mangle] static mut W16_REGS: [u32; 7] = [0; 7];
/// Linear base of the 16-bit stack segment (to find the frame of an API call).
static mut STACK_BASE: u32 = 0;

core::arch::global_asm!(
    r#"
    .intel_syntax noprefix
    .text

    .globl w16_api_entry
w16_api_entry:
    cld
    pushad
    sub esp, 8
    mov ax, ds
    movzx eax, ax
    mov dword ptr ss:[esp + 4], eax
    mov ax, es
    movzx eax, ax
    mov dword ptr ss:[esp], eax
    mov ax, word ptr cs:[{ds32}]
    mov ds, ax
    mov es, ax
    movzx eax, sp
    mov dword ptr [{saved16}], eax
    mov ax, ss
    movzx eax, ax
    mov dword ptr [{saved16} + 4], eax
    lss esp, fword ptr [{host}]
    push dword ptr [{saved16}]
    call dword ptr [{dispatch}]
    add esp, 4
    lss esp, fword ptr [{saved16}]
    mov cx, word ptr ss:[esp]
    mov es, cx
    mov cx, word ptr ss:[esp + 4]
    mov ds, cx
    add esp, 8
    popad
    add esp, 2
    mov bx, word ptr cs:[{argn}]
    movzx ebx, bx
    mov ecx, dword ptr ss:[esp]
    mov dword ptr ss:[esp + ebx], ecx
    add esp, ebx
    .byte 0x66, 0xCB

    .globl w16_call
w16_call:
    push ebp
    push ebx
    push esi
    push edi
    mov eax, dword ptr [esp + 20]
    mov dword ptr [{fptr}], eax
    mov eax, dword ptr [esp + 24]
    mov dword ptr [{fptr} + 4], eax
    mov eax, dword ptr [esp + 32]
    mov dword ptr [{newstack}], eax
    mov eax, dword ptr [esp + 36]
    mov dword ptr [{newstack} + 4], eax
    mov ebx, dword ptr [esp + 28]
    mov dword ptr [{host}], esp
    mov ax, ss
    movzx eax, ax
    mov dword ptr [{host} + 4], eax
    mov ax, bx
    lss esp, fword ptr [{newstack}]
    mov ds, bx
    mov es, bx
    mov ax, word ptr cs:[{regs}]
    mov bx, word ptr cs:[{regs} + 4]
    mov cx, word ptr cs:[{regs} + 8]
    mov dx, word ptr cs:[{regs} + 12]
    mov si, word ptr cs:[{regs} + 16]
    mov di, word ptr cs:[{regs} + 20]
    mov es, word ptr cs:[{regs} + 24]
    jmp fword ptr cs:[{fptr}]

    .globl w16_ret_entry
w16_ret_entry:
    cld
    mov bx, word ptr cs:[{ds32}]
    mov ds, bx
    mov es, bx
    movzx eax, ax
    movzx edx, dx
    mov dword ptr [{ret}], eax
    mov dword ptr [{ret} + 4], edx
    lss esp, fword ptr [{host}]
    pop edi
    pop esi
    pop ebx
    pop ebp
    ret
    "#,
    ds32 = sym W16_DS32,
    saved16 = sym W16_SAVED16,
    host = sym W16_HOST_STACK,
    dispatch = sym W16_DISPATCH,
    argn = sym W16_ARGN,
    fptr = sym W16_FPTR,
    newstack = sym W16_NEWSTACK,
    ret = sym W16_RET,
    regs = sym W16_REGS,
);

/// Size of the thunk segment (one 64 KiB segment).
const THUNK_SIZE: usize = 0x1_0000;
/// Each API stub takes 16 bytes; the last stretch holds the 16 -> 32 return stub.
const STUB: usize = 16;
const RET_STUB: usize = THUNK_SIZE - 32;

pub struct Thunks {
    pub sel: u16,
    base: *mut u8,
    next: usize,
}

unsafe impl Send for Thunks {}

/// Global state of the 16-bit stack (a single Win16 "process").
pub struct Stack16 {
    pub ss: u16,
    pub base: u32,
    pub top: u32,
    /// Lowest SP in use (grows downward); nested calls stay below it.
    pub cur_sp: u32,
}

/// Maps `size` bytes (RWX, zeroed) followed by an inaccessible guard page: a host write
/// past the end of a segment becomes a SIGSEGV at the exact spot, instead of corrupting the neighbor.
pub fn map_rwx(size: usize) -> *mut u8 {
    let size = size.div_ceil(4096) * 4096;
    unsafe {
        // PROT_READ|WRITE|EXEC, MAP_PRIVATE|MAP_ANONYMOUS
        let p = mmap(std::ptr::null_mut(), size + 4096, 7, 0x22, -1, 0);
        if p as isize != -1 {
            mprotect((p as usize + size) as *mut c_void, 4096, 0); // PROT_NONE
        }
        p as *mut u8
    }
}

/// Initializes the assembly globals (the host's flat selectors and the dispatcher).
pub fn init(dispatch: extern "C" fn(u32)) {
    unsafe {
        let (ds, cs): (u32, u32);
        core::arch::asm!("mov {0:e}, ds", "mov {1:e}, cs", out(reg) ds, out(reg) cs);
        W16_DS32 = ds & 0xFFFF;
        W16_CS32 = cs & 0xFFFF;
        W16_DISPATCH.dispatch = dispatch as usize;
        mprotect(std::ptr::addr_of_mut!(W16_DISPATCH) as *mut c_void, 4096, 1); // PROT_READ
    }
}

impl Thunks {
    /// Creates the thunk segment (16-bit code) with the return stub.
    pub fn new() -> Result<Thunks, String> {
        let base = map_rwx(THUNK_SIZE);
        if base as isize == -1 {
            return Err("thunk segment mmap failed".into());
        }
        let sel = ldt::alloc(base as usize as u32, (THUNK_SIZE - 1) as u32, Kind::Code16)?;
        let mut t = Thunks { sel, base, next: 0 };
        // return stub: jmp far cs32:w16_ret_entry (66 EA off32 sel16)
        t.write_far_jmp(RET_STUB, w16_ret_entry as usize as u32);
        Ok(t)
    }

    fn write_far_jmp(&mut self, at: usize, target: u32) {
        let cs = unsafe { W16_CS32 } as u16;
        let mut code = vec![0x66, 0xEA];
        code.extend_from_slice(&target.to_le_bytes());
        code.extend_from_slice(&cs.to_le_bytes());
        unsafe { std::ptr::copy_nonoverlapping(code.as_ptr(), self.base.add(at), code.len()) };
    }

    /// Reserves a stub for API `id` and returns its offset in the thunk segment.
    pub fn add_api(&mut self, id: u16) -> u16 {
        let at = self.next;
        assert!(at + STUB <= RET_STUB, "thunk segment full");
        self.next += STUB;
        // push imm16 id (16-bit): 68 lo hi
        let push = [0x68, id as u8, (id >> 8) as u8];
        unsafe { std::ptr::copy_nonoverlapping(push.as_ptr(), self.base.add(at), 3) };
        self.write_far_jmp(at + 3, w16_api_entry as usize as u32);
        at as u16
    }

    /// Far pointer (selector, offset) of the return stub.
    pub fn ret_addr(&self) -> (u16, u16) {
        (self.sel, RET_STUB as u16)
    }
}

/// Initial registers of a 32 -> 16 call (what the procedure sees on start).
#[derive(Clone, Copy, Default)]
pub struct Regs {
    pub ax: u16,
    pub bx: u16,
    pub cx: u16,
    pub dx: u16,
    pub si: u16,
    pub di: u16,
    pub es: u16,
}

/// Calls a 16-bit procedure (PASCAL convention: arguments pushed left to
/// right, the callee cleans the stack). `args` are 16-bit words already in call order;
/// `(cs, ip)` is the target, `ds` the program's data selector. AX and ES start equal to `ds`
/// (the prologue of exported procedures expects DS in AX). Returns (AX, DX).
pub fn call16(thunks: &Thunks, stack: &mut Stack16, cs: u16, ip: u16, ds: u16, args: &[u16]) -> (u16, u16) {
    call16_regs(thunks, stack, cs, ip, ds, args, Regs { ax: ds, es: ds, ..Default::default() })
}

pub fn call16_regs(thunks: &Thunks, stack: &mut Stack16, cs: u16, ip: u16, ds: u16, args: &[u16], regs: Regs) -> (u16, u16) {
    unsafe {
        W16_REGS = [regs.ax as u32, regs.bx as u32, regs.cx as u32, regs.dx as u32, regs.si as u32, regs.di as u32, regs.es as u32];
        // Saves the global state that nested calls overwrite.
        let saved_host = W16_HOST_STACK;
        let saved_frame = W16_SAVED16;
        let saved_argn = W16_ARGN;
        let saved_cur = stack.cur_sp;

        // Builds the stack: [args...][ret ip][ret cs], with the first argument at the highest address.
        let mut sp = stack.cur_sp as u16;
        let mut push = |v: u16, sp: &mut u16| {
            *sp = sp.wrapping_sub(2);
            *((stack.base + *sp as u32) as usize as *mut u16) = v;
        };
        for &a in args {
            push(a, &mut sp);
        }
        let (rcs, rip) = thunks.ret_addr();
        push(rcs, &mut sp);
        push(rip, &mut sp);

        stack.cur_sp = sp as u32;
        crate::dbgalloc::protect(true);
        w16_call(ip as u32, cs as u32, ds as u32, sp as u32, stack.ss as u32);
        crate::dbgalloc::protect(false);
        let ret = (W16_RET[0] as u16, W16_RET[1] as u16);

        W16_HOST_STACK = saved_host;
        W16_SAVED16 = saved_frame;
        W16_ARGN = saved_argn;
        stack.cur_sp = saved_cur;
        ret
    }
}

// ---- access to the frame of an API call (used by the dispatcher) ------------------------------

/// Layout of the frame saved by `w16_api_entry`, starting at SP:
/// es ds (4 bytes each) | edi esi ebp esp ebx edx ecx eax | id u16 | ip u16 | cs u16 | arguments...
pub const FRAME_ES: usize = 0;
pub const FRAME_DS: usize = 4;
pub const FRAME_EDI: usize = 8;
pub const FRAME_ESI: usize = 12;
pub const FRAME_EBX: usize = 24;
pub const FRAME_EDX: usize = 28;
pub const FRAME_ECX: usize = 32;
pub const FRAME_EAX: usize = 36;
pub const FRAME_ID: usize = 40;
pub const FRAME_ARGS: usize = 46;

/// Records the linear base of the 16-bit stack.
pub fn set_stack_base(base: u32) {
    unsafe { STACK_BASE = base }
}

/// Pointer to the saved frame of an API call (SP at the moment of `w16_api_entry`).
pub fn frame_ptr(sp16: u32) -> *mut u8 {
    unsafe { (STACK_BASE + sp16) as usize as *mut u8 }
}

pub fn set_argn(n: u32) {
    unsafe { W16_ARGN = n }
}

pub fn saved_sp() -> u32 {
    unsafe { W16_SAVED16[0] }
}

/// Debugging: state of the globals used by the assembly.
pub fn debug_state() -> String {
    unsafe {
        format!("HOST_STACK={:x?} SAVED16={:x?} NEWSTACK={:x?} FPTR={:x?} ARGN={:#x} DS32={:#x} CS32={:#x} DISPATCH={:#x}",
            W16_HOST_STACK, W16_SAVED16, W16_NEWSTACK, W16_FPTR, W16_ARGN, W16_DS32, W16_CS32, W16_DISPATCH.dispatch)
    }
}
