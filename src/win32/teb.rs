//! TEB/PEB falsos. Código gerado pelo MSVC lê `fs:[0]` (cadeia de SEH) e
//! `fs:[0x18]` (ponteiro do próprio TEB) já no startup, então `fs` precisa
//! apontar para uma estrutura válida. Usamos uma entrada de TLS na GDT.

use std::ffi::{c_long, c_void};

const SYS_SET_THREAD_AREA: c_long = 243; // i386

extern "C" {
    fn syscall(num: c_long, ...) -> c_long;
}

/// Layout de `struct user_desc` do Linux.
#[repr(C)]
struct UserDesc {
    entry_number: u32,
    base_addr: u32,
    limit: u32,
    flags: u32,
}

// Offsets do TEB (layout NT, o mesmo que o código das aplicações espera em fs:).
const TEB_SEH_CHAIN: usize = 0x00;
const TEB_STACK_BASE: usize = 0x04;
const TEB_STACK_LIMIT: usize = 0x08;
const TEB_SELF: usize = 0x18;
const TEB_PID: usize = 0x20;
const TEB_TID: usize = 0x24;
const TEB_PEB: usize = 0x30;
const PEB_IMAGE_BASE: usize = 0x08;

fn put(buf: &mut [u8], off: usize, v: u32) {
    buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

/// Cria TEB+PEB e aponta `fs` para o TEB.
pub fn install(image_base: u32) -> Result<(), &'static str> {
    // Vazam de propósito: vivem até o fim do processo.
    let peb: &'static mut [u8] = Box::leak(vec![0u8; 0x400].into_boxed_slice());
    let teb: &'static mut [u8] = Box::leak(vec![0u8; 0x1000].into_boxed_slice());

    put(peb, PEB_IMAGE_BASE, image_base);

    let here = 0u8;
    let stack_top = &here as *const u8 as usize as u32;
    put(teb, TEB_SEH_CHAIN, 0xFFFF_FFFF); // fim da cadeia
    put(teb, TEB_STACK_BASE, stack_top);
    put(teb, TEB_STACK_LIMIT, stack_top.saturating_sub(8 << 20)); // 8 MiB, padrão do Linux
    put(teb, TEB_SELF, teb.as_ptr() as usize as u32);
    put(teb, TEB_PID, std::process::id());
    put(teb, TEB_TID, std::process::id());
    put(teb, TEB_PEB, peb.as_ptr() as usize as u32);

    let mut desc = UserDesc {
        entry_number: u32::MAX, // o kernel escolhe uma entrada livre
        base_addr: teb.as_ptr() as usize as u32,
        limit: 0xFFF,
        // seg_32bit(1) | useable(1<<6); dados, leitura/escrita, limite em bytes
        flags: 1 | (1 << 6),
    };
    // SAFETY: `desc` tem o layout esperado pelo kernel.
    if unsafe { syscall(SYS_SET_THREAD_AREA, &mut desc as *mut UserDesc as *mut c_void) } != 0 {
        return Err("set_thread_area falhou");
    }
    let selector = (desc.entry_number << 3) | 3; // RPL 3, tabela GDT
    // SAFETY: o seletor acabou de ser criado e é válido para dados.
    unsafe { core::arch::asm!("mov fs, {0:x}", in(reg) selector, options(nostack, preserves_flags)) };
    Ok(())
}
