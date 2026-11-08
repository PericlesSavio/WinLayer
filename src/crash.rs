//! Diagnostics: prints registers when the .exe's code raises SIGSEGV/SIGILL.

use std::ffi::c_void;

/// musl's `struct sigaction` on i386.
#[repr(C)]
struct SigAction {
    handler: extern "C" fn(i32, *const u8, *const u32),
    mask: [u32; 32],
    flags: i32,
    restorer: usize,
}

#[repr(C)]
struct StackT {
    sp: *mut c_void,
    flags: i32,
    size: usize,
}

extern "C" {
    fn sigaction(sig: i32, act: *const SigAction, old: *mut c_void) -> i32;
    fn sigaltstack(ss: *const StackT, old: *mut StackT) -> i32;
    fn mmap(addr: *mut c_void, len: usize, prot: i32, flags: i32, fd: i32, off: i64) -> *mut c_void;
}

const SA_ONSTACK: i32 = 0x0800_0000;

const SA_SIGINFO: i32 = 4;

extern "C" fn on_fault(sig: i32, info: *const u8, uctx: *const u32) {
    crate::dbgalloc::protect(false); // the handler itself allocates
    if false {
        crate::win16::ldt::audit("on crash");
        crate::win16::ldt::overlaps();
    }
    // INT n in Win16 code (LDT selector) -> SIGSEGV: emulates the service and resumes after the instruction.
    {
        let g = unsafe { std::slice::from_raw_parts_mut(uctx.add(5) as *mut u32, 19) };
        let (cs, ip) = (g[15] as u16, g[14] as u16);
        if sig == 11 && cs & 4 != 0 {
            if let Some(lin) = crate::win16::ldt::linear(cs, ip) {
                let (op, n) = unsafe { (*(lin as usize as *const u8), *((lin + 1) as usize as *const u8)) };
                if op == 0xCD {
                    crate::win16::dosint::handle(n, &mut crate::win16::dosint::Regs(g));
                    g[14] = g[14].wrapping_add(2);
                    crate::dbgalloc::protect(true); // protects the host heap again while the guest runs
                    return;
                }
            }
        }
    }
    // i386 ucontext: uc_flags, uc_link, uc_stack (3 words), then gregs[19]:
    // GS FS ES DS EDI ESI EBP ESP EBX EDX ECX EAX TRAPNO ERR EIP CS EFL UESP SS
    let (fault, g) = unsafe { (*(info.add(12) as *const u32), std::slice::from_raw_parts(uctx.add(5), 19)) };
    eprintln!("\nwinlayer: signal {sig} at CS:EIP={:04x}:{:#010x}, address={:#010x}, SS={:04x}", g[15], g[14], fault, g[18]);
    eprintln!(
        "  EAX={:08x} EBX={:08x} ECX={:08x} EDX={:08x}\n  ESI={:08x} EDI={:08x} EBP={:08x} ESP={:08x}\n  FS={:04x} GS={:04x} DS={:04x} ES={:04x}",
        g[11], g[8], g[10], g[9], g[5], g[4], g[6], g[7], g[1], g[0], g[3], g[2]
    );
    let (cs, ip, ss, sp) = (g[15] as u16, g[14], g[18] as u16, g[7]);
    if cs & 4 != 0 {
        // 16-bit code (LDT selector): shows the bytes at CS:IP and the top of the 16-bit stack
        if let Some(lin) = crate::win16::ldt::linear(cs, ip as u16) {
            let b: Vec<String> = (0..8).map(|i| format!("{:02x}", unsafe { *((lin + i) as usize as *const u8) })).collect();
            eprintln!("  bytes at CS:IP (16-bit): {}", b.join(" "));
        }
        if let Some(lin) = crate::win16::ldt::linear(ss, sp as u16) {
            let w: Vec<String> = (0..8).map(|i| format!("{:04x}", unsafe { std::ptr::read_unaligned((lin + i * 2) as usize as *const u16) })).collect();
            eprintln!("  16-bit stack: {}", w.join(" "));
        }
        if let Some(lin) = crate::win16::ldt::linear(ss, (g[6] as u16).wrapping_sub(0x40)) {
            let w: Vec<String> = (0..48).map(|i| format!("{:02x}", unsafe { *((lin + i) as usize as *const u8) })).collect();
            eprintln!("  [bp-0x40..]: {}", w.join(" "));
        }
        if let Some(lin) = crate::win16::ldt::linear(ss, g[6] as u16) {
            let w: Vec<String> = (0..6).map(|i| format!("{:04x}", unsafe { std::ptr::read_unaligned((lin + i * 2) as usize as *const u16) })).collect();
            eprintln!("  [bp]: prev-bp ret-ip ret-cs arg1.. = {}", w.join(" "));
            // caller's bytes, ending at the return address
            let (rip, rcs) = unsafe { (std::ptr::read_unaligned((lin + 2) as usize as *const u16), std::ptr::read_unaligned((lin + 4) as usize as *const u16)) };
            if let Some(l2) = crate::win16::ldt::linear(rcs, rip.wrapping_sub(40)) {
                let w: Vec<String> = (0..40).map(|i| format!("{:02x}", unsafe { *((l2 + i) as usize as *const u8) })).collect();
                eprintln!("  caller (40 bytes before {rcs:04x}:{rip:04x}): {}", w.join(" "));
            }
        }
    } else if g[18] as u16 == 0x2b || g[18] as u16 == 0x7b || true {
        // host code: dumps the top of the stack (to locate the caller with `nm`)
        let sp = g[17] as usize;
        if sp > 0x10000 {
            let w: Vec<String> = (0..48).map(|i| format!("{:08x}", unsafe { std::ptr::read_unaligned((sp + i * 4) as *const u32) })).collect();
            eprintln!("  host stack: {}", w.join(" "));
        }
        if (0x400000..0x1000000).contains(&g[14]) {
            let p = g[14] as *const u8;
            let bytes: Vec<String> = (0..8).map(|i| format!("{:02x}", unsafe { *p.add(i) })).collect();
            eprintln!("  bytes at EIP: {}", bytes.join(" "));
        }
    } else if (0x400000..0x1000000).contains(&g[14]) {
        let p = g[14] as *const u8;
        let bytes: Vec<String> = (0..8).map(|i| format!("{:02x}", unsafe { *p.add(i) })).collect();
        eprintln!("  bytes at EIP: {}", bytes.join(" "));
    }
    // memory pointed to by EAX/ECX/EDX (useful for seeing the corrupted structure)
    for (nome, v) in [("EAX", g[11]), ("ECX", g[10]), ("EDX", g[9])] {
        let b = crate::loader::peek(v & !0xF, 64);
        if b.len() == 64 {
            let hex: Vec<String> = b.iter().map(|x| format!("{x:02x}")).collect();
            let txt: String = b.iter().map(|&c| if (32..127).contains(&c) { c as char } else { '.' }).collect();
            eprintln!("  [{nome}={:#x}] {} |{txt}|", v & !0xF, hex.join(""));
        }
    }
    std::process::exit(139);
}

pub fn install() {
    // Alternate stack: Win16 code runs with a 16-bit SS, where the kernel cannot build the
    // signal frame on the normal stack.
    // mmap (outside the program's heap): the heap may become read-only in debug mode
    let size = 256 * 1024;
    let sp = unsafe { mmap(std::ptr::null_mut(), size, 3, 0x22, -1, 0) };
    let st = StackT { sp, flags: 0, size };
    unsafe { sigaltstack(&st, std::ptr::null_mut()) };
    let act = SigAction { handler: on_fault, mask: [0; 32], flags: SA_SIGINFO | SA_ONSTACK, restorer: 0 };
    for sig in [11, 4, 7, 8] {
        // SIGSEGV, SIGILL, SIGBUS, SIGFPE
        unsafe { sigaction(sig, &act, std::ptr::null_mut()) };
    }
}
