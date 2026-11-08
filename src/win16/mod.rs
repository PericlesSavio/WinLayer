//! Camada Win16: carrega executáveis NE e executa o código de 16 bits diretamente no processador,
//! usando segmentos LDT. As chamadas à API do Windows 3.x passam por stubs (ver `trans`).

pub mod api;
pub mod dosint;
pub mod gdi;
pub mod kernel;
pub mod kernel_ext;
pub mod ldt;
pub mod ne;
pub mod ordinals;
pub mod trans;
pub mod user;

use trans::{Regs, Stack16, Thunks};

/// Estado do runtime Win16 (um programa por processo; o código de 16 bits é de thread única).
pub struct Runtime {
    pub thunks: Thunks,
    pub stack: Stack16,
}

static mut RUNTIME: Option<Runtime> = None;

/// Acesso ao runtime. Seguro na prática: só a thread principal executa código Win16.
#[allow(static_mut_refs)]
pub fn rt() -> &'static mut Runtime {
    unsafe { RUNTIME.as_mut().expect("runtime Win16 não inicializado") }
}

fn ne_resources_holder(ne: &ne::Ne) -> Vec<ne::ResType> {
    ne.resources.iter().map(|t| ne::ResType { key: t.key.clone(), entries: t.entries.iter().map(|e| ne::ResEntry { key: e.key.clone(), off: e.off, len: e.len }).collect() }).collect()
}

/// Carrega e executa um programa NE. Só retorna se o programa terminar sem chamar `exit`.
pub fn run(path: &str, data: &[u8], args: &[String]) -> Result<i32, String> {
    trans::init(api::dispatch);
    crate::win32::text::WIN16.store(true, std::sync::atomic::Ordering::Relaxed);
    let mut thunks = Thunks::new()?;
    let mut cache = Vec::new();
    let ne = ne::load(data, &mut |module, ordinal| api::resolve_import(&mut thunks, &mut cache, module, ordinal))?;

    if std::env::var_os("WINLAYER_WIN16_REPORT").is_some() {
        let (ok, missing): (Vec<_>, Vec<_>) = api::report().into_iter().partition(|(_, ok)| *ok);
        eprintln!("[win16] {} APIs implementadas, {} faltando:", ok.len(), missing.len());
        for (n, _) in missing {
            eprintln!("  falta {n}");
        }
    }

    kernel_ext::set_resources(std::mem::take(&mut ne_resources_holder(&ne)), data.to_vec());
    user::install_hooks();
    let dg = ne.dgroup.ok_or("programa sem segmento de dados automático (DGROUP)")?;
    let dseg = &ne.segs[dg];
    let (sp_decl, ss_seg) = (ne.stack_start.1, ne.stack_start.0);
    let _ = ss_seg;
    // SP = 0 no cabeçalho: a pilha fica logo depois dos dados do DGROUP
    let sp = if sp_decl == 0 { (dseg.size.min(0xFFFE) as u16).saturating_sub(ne.heap) } else { sp_decl };
    let sp = sp & !1;
    trans::set_stack_base(dseg.base);
    let stack = Stack16 { ss: dseg.sel, base: dseg.base, top: sp as u32, cur_sp: sp as u32 };

    let exe_name = std::path::Path::new(path).file_name().map_or("PROGRAM.EXE".to_string(), |n| n.to_string_lossy().to_uppercase());
    kernel::setup(dseg.sel, dseg.base, &exe_name, &args.join(" "), ne.stack, ne.heap)?;

    let (cs_seg, ip) = ne.entry;
    let cs = ne.segs.get(cs_seg as usize - 1).ok_or("segmento de entrada inválido")?.sel;
    let psp = kernel::with_process(|p| p.psp_sel);
    let ds = dseg.sel;
    unsafe { RUNTIME = Some(Runtime { thunks, stack }) };
    let r = rt();
    // Registradores de partida de uma tarefa Win16: BX = pilha, CX = heap, DI = hInstance, ES = PSP.
    let regs = Regs { ax: 0, bx: ne.stack, cx: ne.heap, dx: 0, si: 0, di: ds, es: psp };
    let (ax, _) = trans::call16_regs(&r.thunks, &mut r.stack, cs, ip, ds, &[], regs);
    Ok(ax as i32)
}

/// Teste de viabilidade: executa um trecho de código de 16 bits à mão que soma 3+4 chamando uma
/// "API" (id 0) por um stub, e confere AX = 7 na volta.
pub fn selftest() -> Result<(), String> {
    trans::init(selftest_dispatch);
    let mut thunks = Thunks::new()?;
    let api_off = thunks.add_api(0);

    // segmento de código: push 3; push 4; call far thunk:api; retf
    let code = trans::map_rwx(4096);
    let mut c: Vec<u8> = vec![0x6A, 0x03, 0x6A, 0x04, 0x9A];
    c.extend_from_slice(&api_off.to_le_bytes());
    c.extend_from_slice(&thunks.sel.to_le_bytes());
    c.push(0xCB); // retf
    unsafe { std::ptr::copy_nonoverlapping(c.as_ptr(), code, c.len()) };
    let code_sel = ldt::alloc(code as usize as u32, 4095, ldt::Kind::Code16)?;

    // segmento de dados/pilha de 64 KiB
    let data = trans::map_rwx(0x1_0000);
    let data_sel = ldt::alloc(data as usize as u32, 0xFFFF, ldt::Kind::Data16)?;
    trans::set_stack_base(data as usize as u32);
    let mut stack = Stack16 { ss: data_sel, base: data as usize as u32, top: 0xFFF0, cur_sp: 0xFFF0 };

    let (ax, _dx) = trans::call16(&thunks, &mut stack, code_sel, 0, data_sel, &[]);
    if ax == 7 { Ok(()) } else { Err(format!("esperava AX=7, veio {ax}")) }
}

/// Despachante do selftest: API 0 = soma de duas palavras (PASCAL: a, b empilhados nessa ordem,
/// então `b` está no endereço mais baixo).
extern "C" fn selftest_dispatch(sp16: u32) {
    unsafe {
        let f = trans::frame_ptr(sp16);
        let rd = |o: usize| std::ptr::read_unaligned(f.add(o) as *const u16);
        let (id, b, a) = (rd(trans::FRAME_ID), rd(trans::FRAME_ARGS), rd(trans::FRAME_ARGS + 2));
        let r = if id == 0 { a.wrapping_add(b) } else { 0xFFFF };
        std::ptr::write_unaligned(f.add(trans::FRAME_EAX) as *mut u32, r as u32);
        trans::set_argn(4);
    }
}
