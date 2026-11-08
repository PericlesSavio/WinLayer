//! Registro e despacho das APIs do Windows 3.x chamadas pelo código de 16 bits.
//!
//! Cada função importada ganha um id e um stub no segmento de thunks (ver `trans`). Ao ser
//! chamada, `dispatch` acha o tratador, lê os argumentos (convenção PASCAL: o último argumento
//! está no endereço mais baixo) e devolve o resultado em AX (ou DX:AX).

use super::{ldt, ordinals, trans};
use std::sync::Mutex;

/// Tratador de uma API: lê argumentos via `Call` e devolve AX (baixo) / DX (alto).
pub type Handler = fn(&mut Call) -> u32;

pub struct Impl {
    pub module: &'static str,
    pub name: &'static str,
    /// Assinatura dos argumentos, da esquerda para a direita: `w` palavra, `l` dword, `p` ponteiro
    /// far (4 bytes; `Call::ptr` converte para linear).
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

/// Constantes absolutas exportadas por ordinal (valem como offset do fixup, seletor 0).
pub fn absolute(module: &str, ordinal: u16) -> Option<u16> {
    match (module, ordinal) {
        ("KERNEL", 113) => Some(3),      // __AHSHIFT
        ("KERNEL", 114) => Some(8),      // __AHINCR
        ("KERNEL", 178) => Some(0x0429), // __WINFLAGS
        _ => None,
    }
}

/// Cria (ou reaproveita) o stub de `module.ordinal` e devolve (seletor, offset) do thunk.
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

/// Lista (módulo.nome, implementada?) de tudo o que o programa importa — diagnóstico.
pub fn report() -> Vec<(String, bool)> {
    SLOTS.lock().unwrap().iter().map(|s| (format!("{}.{}({})", s.module, s.name, s.ordinal), s.f.is_some())).collect()
}

/// Argumentos e registradores de uma chamada em andamento.
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

    /// Offset (no quadro) do argumento `i` (0 = o mais à esquerda).
    fn arg_off(&self, i: usize) -> usize {
        trans::FRAME_ARGS + self.sig.chars().skip(i + 1).map(size_of).sum::<usize>()
    }

    /// Bytes de argumentos (o que `retf N` descarta).
    pub fn argn(&self) -> u32 {
        self.sig.chars().map(size_of).sum::<usize>() as u32
    }

    /// Argumento de 16 bits.
    pub fn w(&self, i: usize) -> u16 {
        self.rd16(self.arg_off(i))
    }

    /// Argumento de 16 bits com sinal.
    pub fn s(&self, i: usize) -> i16 {
        self.w(i) as i16
    }

    /// Argumento de 32 bits cru (dword ou ponteiro far como `seg:off`).
    pub fn l(&self, i: usize) -> u32 {
        self.rd32(self.arg_off(i))
    }

    /// Ponteiro far -> endereço linear (0 se nulo ou seletor inválido).
    pub fn ptr(&self, i: usize) -> u32 {
        far_to_linear(self.l(i))
    }

    /// Valor de um registrador salvo (ex.: BX numa API que recebe argumentos em registradores).
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

/// `seg:off` (dword com o offset no baixo) -> linear; 0 se nulo ou seletor desconhecido.
pub fn far_to_linear(p: u32) -> u32 {
    let (off, sel) = (p as u16, (p >> 16) as u16);
    if sel == 0 {
        return 0;
    }
    match ldt::lookup(sel) {
        Some(seg) if (off as u32) <= seg.limit => seg.base.wrapping_add(off as u32),
        Some(seg) => {
            eprintln!("[win16] AVISO: ponteiro far {sel:04x}:{off:04x} além do limite do segmento ({:#x})", seg.limit);
            0
        }
        None => {
            eprintln!("[win16] AVISO: ponteiro far {sel:04x}:{off:04x} com seletor desconhecido");
            0
        }
    }
}

/// Empacota seletor e offset num ponteiro far.
pub fn far(sel: u16, off: u16) -> u32 {
    ((sel as u32) << 16) | off as u32
}

fn trace_on() -> bool {
    std::env::var_os("WINLAYER_WIN16_TRACE").is_some()
}

/// Chamada pelo assembly (`w16_api_entry`) quando o código de 16 bits invoca uma API.
pub extern "C" fn dispatch(sp16: u32) {
    crate::dbgalloc::protect(false);
    struct Reprotect;
    impl Drop for Reprotect {
        fn drop(&mut self) {
            crate::dbgalloc::protect(true);
        }
    }
    let _reprotect = Reprotect;
    // Chamadas aninhadas (callbacks para 16 bits) devem continuar abaixo do quadro desta chamada.
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
        eprintln!("[win16] {module}.{name}({ordinal}) id/ip/cs=[{}] args(pilha)=[{}] sp16={sp16:#x}", head.join(" "), words.join(" "));
    }
    let Some(f) = f else {
        eprintln!("\n[win16] API NÃO IMPLEMENTADA: {module}.{name} (ordinal {ordinal})");
        crate::win32::kernel32::exit_process(3);
    };
    let r = f(&mut call);
    if crate::dbgalloc::check_all(&format!("depois de {module}.{name}")) > 0 {
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
