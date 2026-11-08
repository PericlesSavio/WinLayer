//! Win16 layer: loads NE executables and runs 16-bit code directly on the processor,
//! using LDT segments. Calls to the Windows 3.x API go through stubs (see `trans`).

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

/// Win16 runtime state (one program per process; 16-bit code is single-threaded).
pub struct Runtime {
    pub thunks: Thunks,
    pub stack: Stack16,
}

static mut RUNTIME: Option<Runtime> = None;

/// Runtime access. Safe in practice: only the main thread runs Win16 code.
#[allow(static_mut_refs)]
pub fn rt() -> &'static mut Runtime {
    unsafe { RUNTIME.as_mut().expect("Win16 runtime not initialized") }
}

fn ne_resources_holder(ne: &ne::Ne) -> Vec<ne::ResType> {
    ne.resources.iter().map(|t| ne::ResType { key: t.key.clone(), entries: t.entries.iter().map(|e| ne::ResEntry { key: e.key.clone(), off: e.off, len: e.len }).collect() }).collect()
}

/// Loads and runs an NE program. Only returns if the program ends without calling `exit`.
pub fn run(path: &str, data: &[u8], args: &[String]) -> Result<i32, String> {
    trans::init(api::dispatch);
    crate::win32::text::WIN16.store(true, std::sync::atomic::Ordering::Relaxed);
    let mut thunks = Thunks::new()?;
    let mut cache = Vec::new();
    let ne = ne::load(data, &mut |module, ordinal| api::resolve_import(&mut thunks, &mut cache, module, ordinal))?;

    if std::env::var_os("WINLAYER_WIN16_REPORT").is_some() {
        let (ok, missing): (Vec<_>, Vec<_>) = api::report().into_iter().partition(|(_, ok)| *ok);
        eprintln!("[win16] {} APIs implemented, {} missing:", ok.len(), missing.len());
        for (n, _) in missing {
            eprintln!("  missing {n}");
        }
    }

    kernel_ext::set_resources(std::mem::take(&mut ne_resources_holder(&ne)), data.to_vec());
    user::install_hooks();
    let dg = ne.dgroup.ok_or("program without automatic data segment (DGROUP)")?;
    let dseg = &ne.segs[dg];
    let (sp_decl, ss_seg) = (ne.stack_start.1, ne.stack_start.0);
    let _ = ss_seg;
    // SP = 0 in the header: the stack sits right after the DGROUP data
    let sp = if sp_decl == 0 { (dseg.size.min(0xFFFE) as u16).saturating_sub(ne.heap) } else { sp_decl };
    let sp = sp & !1;
    trans::set_stack_base(dseg.base);
    let stack = Stack16 { ss: dseg.sel, base: dseg.base, top: sp as u32, cur_sp: sp as u32 };

    let exe_name = std::path::Path::new(path).file_name().map_or("PROGRAM.EXE".to_string(), |n| n.to_string_lossy().to_uppercase());
    kernel::setup(dseg.sel, dseg.base, &exe_name, &args.join(" "), ne.stack, ne.heap)?;

    let (cs_seg, ip) = ne.entry;
    let cs = ne.segs.get(cs_seg as usize - 1).ok_or("invalid entry segment")?.sel;
    let psp = kernel::with_process(|p| p.psp_sel);
    let ds = dseg.sel;
    unsafe { RUNTIME = Some(Runtime { thunks, stack }) };
    let r = rt();
    // Startup registers of a Win16 task: BX = stack, CX = heap, DI = hInstance, ES = PSP.
    let regs = Regs { ax: 0, bx: ne.stack, cx: ne.heap, dx: 0, si: 0, di: ds, es: psp };
    let (ax, _) = trans::call16_regs(&r.thunks, &mut r.stack, cs, ip, ds, &[], regs);
    Ok(ax as i32)
}

/// Feasibility test: runs a hand-written piece of 16-bit code that adds 3+4 by calling an
/// "API" (id 0) through a stub, and checks AX = 7 on return.
pub fn selftest() -> Result<(), String> {
    trans::init(selftest_dispatch);
    let mut thunks = Thunks::new()?;
    let api_off = thunks.add_api(0);

    // code segment: push 3; push 4; call far thunk:api; retf
    let code = trans::map_rwx(4096);
    let mut c: Vec<u8> = vec![0x6A, 0x03, 0x6A, 0x04, 0x9A];
    c.extend_from_slice(&api_off.to_le_bytes());
    c.extend_from_slice(&thunks.sel.to_le_bytes());
    c.push(0xCB); // retf
    unsafe { std::ptr::copy_nonoverlapping(c.as_ptr(), code, c.len()) };
    let code_sel = ldt::alloc(code as usize as u32, 4095, ldt::Kind::Code16)?;

    // 64 KiB data/stack segment
    let data = trans::map_rwx(0x1_0000);
    let data_sel = ldt::alloc(data as usize as u32, 0xFFFF, ldt::Kind::Data16)?;
    trans::set_stack_base(data as usize as u32);
    let mut stack = Stack16 { ss: data_sel, base: data as usize as u32, top: 0xFFF0, cur_sp: 0xFFF0 };

    let (ax, _dx) = trans::call16(&thunks, &mut stack, code_sel, 0, data_sel, &[]);
    if ax == 7 { Ok(()) } else { Err(format!("expected AX=7, got {ax}")) }
}

/// Selftest dispatcher: API 0 = sum of two words (PASCAL: a, b pushed in that order,
/// so `b` is at the lowest address).
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
