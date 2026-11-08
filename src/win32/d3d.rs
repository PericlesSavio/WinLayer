//! Direct3D 6 (IDirect3D3 e companhia) — por enquanto só rastreamento: os objetos existem, aceitam as
//! chamadas e registram tudo com `WINLAYER_D3D_TRACE=1`, sem desenhar. Serve para medir o que os jogos
//! usam antes de escrever o rasterizador.

use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

const OK: u32 = 0;
const E_NOINTERFACE: u32 = 0x8000_4002;

/// IID_IDirect3D3 e IID_IDirect3DTexture2 (primeiro dword).
pub const IID_D3D3: u32 = 0xBB22_3240;
pub const IID_TEXTURE2: u32 = 0x9328_1502;

fn trace() -> bool {
    std::env::var_os("WINLAYER_D3D_TRACE").is_some()
}

fn log(name: &str, this: u32, args: &[u32]) {
    if trace() {
        let a: Vec<String> = args.iter().map(|v| format!("{v:#x}")).collect();
        eprintln!("[d3d] {name} {this:#x} ({})", a.join(", "));
    }
}

macro_rules! vtable {
    ($($f:expr),* $(,)?) => {{
        static V: OnceLock<Vec<u32>> = OnceLock::new();
        V.get_or_init(|| vec![$($f as *const () as usize as u32),*]).as_slice()
    }};
}

/// Método sem efeito, registrado; a quantidade de argumentos tem de ser a real (stdcall).
macro_rules! m {
    ($($name:ident = $ret:expr, ($($a:ident),*);)*) => {
        $(extern "stdcall" fn $name(this: u32 $(, $a: u32)*) -> u32 {
            log(stringify!($name), this, &[$($a),*]);
            $ret
        })*
    };
}

fn new_obj(vtbl: &'static [u32]) -> u32 {
    Box::leak(Box::new(vtbl.as_ptr() as u32)) as *mut u32 as u32
}

unsafe fn rd(p: u32, o: u32) -> u32 {
    std::ptr::read_unaligned((p + o) as usize as *const u32)
}

unsafe fn wr(p: u32, o: u32, v: u32) {
    std::ptr::write_unaligned((p + o) as usize as *mut u32, v)
}

unsafe fn put(out: u32, v: u32) {
    if out != 0 {
        wr(out, 0, v);
    }
}

extern "stdcall" fn addref(_t: u32) -> u32 {
    1
}

extern "stdcall" fn release(_t: u32) -> u32 {
    0
}

unsafe extern "stdcall" fn qi(this: u32, riid: u32, out: *mut u32) -> u32 {
    let first = if riid == 0 { 0 } else { rd(riid, 0) };
    log("QueryInterface", this, &[first]);
    // IUnknown, IDirect3D3, Viewport/2/3, Device3, Material/2/3, Light, Texture2: as versões novas
    // estendem as antigas na mesma ordem, então cada objeto serve para todas.
    if matches!(first, 0 | IID_D3D3 | 0x4417_C146 | 0x9328_1500 | 0xB0AB_3B61 | 0xB0AB_3B60 | 0x4417_C144 | 0x9328_1503 | 0xCA9C_46F4 | 0x4417_C142 | IID_TEXTURE2) {
        *out = this;
        OK
    } else {
        *out = 0;
        E_NOINTERFACE
    }
}

// ---- IDirect3D3 -----------------------------------------------------------------------------

static D3D3: OnceLock<u32> = OnceLock::new();

pub fn direct3d3() -> u32 {
    *D3D3.get_or_init(|| new_obj(d3d3_vtbl()))
}

fn d3d3_vtbl() -> &'static [u32] {
    vtable![qi, addref, release, d3_enum_devices, d3_create_light, d3_create_material, d3_create_viewport,
        d3_find_device, d3_create_device, d3_create_vb, d3_enum_z, d3_evict]
}

m! {
    d3_find_device = 0x8000_4001, (a, b);
    d3_evict = OK, ();
}

/// D3DDEVICEDESC (DX6, 252 bytes) de um rasterizador por software.
fn device_desc(software: bool) -> [u8; 252] {
    let mut d = [0u8; 252];
    let mut w = |o: usize, v: u32| d[o..o + 4].copy_from_slice(&v.to_le_bytes());
    w(0, 252);
    if software {
        w(4, 0x7FF);
        w(8, 2); // D3DCOLOR_RGB
        w(12, 0x1 | 0x10 | 0x40 | 0x100 | 0x400); // FLOATTLVERTEX, EXECUTE/TLVERTEX/TEXTURE em memória do sistema, DRAWPRIMTLVERTEX
        for base in [44usize, 100] {
            w(base, 56);
            w(base + 8, 0x1FF); // dwRasterCaps
            w(base + 12, 0xFF); // dwZCmpCaps
            w(base + 16, 0x1FFF); // dwSrcBlendCaps
            w(base + 20, 0x1FFF); // dwDestBlendCaps
            w(base + 24, 0xFF); // dwAlphaCmpCaps
            w(base + 28, 0x1FF_FFFF); // dwShadeCaps
            w(base + 32, 0x1FF); // dwTextureCaps
            w(base + 36, 0x3F); // dwTextureFilterCaps
            w(base + 40, 0xFF); // dwTextureBlendCaps
            w(base + 44, 0x1F); // dwTextureAddressCaps
        }
        w(156, 0x400 | 0x100); // DDBD_16 | DDBD_32
        w(160, 0x400); // z-buffer de 16 bits
        w(168, 0xFFFF); // dwMaxVertexCount
        w(172, 1);
        w(176, 1);
        w(180, 1024);
        w(184, 1024);
    }
    d
}

const RGB_GUID: [u8; 16] = [0x60, 0x5C, 0x66, 0xA4, 0x73, 0x26, 0xCF, 0x11, 0xA3, 0x1A, 0x00, 0xAA, 0x00, 0xB9, 0x33, 0x56];

unsafe extern "stdcall" fn d3_enum_devices(this: u32, cb: u32, ctx: u32) -> u32 {
    log("d3_enum_devices", this, &[cb, ctx]);
    if cb != 0 {
        let (hw, hel) = (device_desc(false), device_desc(true));
        let f: extern "stdcall" fn(u32, u32, u32, u32, u32, u32) -> u32 = std::mem::transmute(cb as usize);
        f(RGB_GUID.as_ptr() as u32, c"RGB Emulation".as_ptr() as u32, c"RGB Emulation".as_ptr() as u32, hw.as_ptr() as u32, hel.as_ptr() as u32, ctx);
    }
    OK
}

unsafe extern "stdcall" fn d3_create_device(this: u32, clsid: u32, surf: u32, out: u32, outer: u32) -> u32 {
    log("d3_create_device", this, &[rd(clsid, 0), surf, out, outer]);
    put(out, new_obj(device_vtbl()));
    OK
}

unsafe extern "stdcall" fn d3_create_viewport(this: u32, out: u32, outer: u32) -> u32 {
    log("d3_create_viewport", this, &[out, outer]);
    put(out, new_obj(viewport_vtbl()));
    OK
}

unsafe extern "stdcall" fn d3_create_material(this: u32, out: u32, outer: u32) -> u32 {
    log("d3_create_material", this, &[out, outer]);
    put(out, new_obj(material_vtbl()));
    OK
}

unsafe extern "stdcall" fn d3_create_light(this: u32, out: u32, outer: u32) -> u32 {
    log("d3_create_light", this, &[out, outer]);
    put(out, new_obj(light_vtbl()));
    OK
}

/// Memória dos vertex buffers (objeto -> bloco).
static VBS: Mutex<BTreeMap<u32, Vec<u8>>> = Mutex::new(BTreeMap::new());

unsafe extern "stdcall" fn d3_create_vb(this: u32, desc: u32, out: u32, flags: u32, outer: u32) -> u32 {
    log("d3_create_vb", this, &[rd(desc, 4), rd(desc, 8), rd(desc, 12), flags, outer]);
    let obj = new_obj(vb_vtbl());
    VBS.lock().unwrap().insert(obj, vec![0; rd(desc, 12) as usize * 64]);
    put(out, obj);
    OK
}

/// Formatos de z-buffer: 16 bits.
unsafe extern "stdcall" fn d3_enum_z(this: u32, clsid: u32, cb: u32, ctx: u32) -> u32 {
    log("d3_enum_z", this, &[rd(clsid, 0), cb, ctx]);
    if cb != 0 {
        let mut pf = [0u32; 8];
        pf[0] = 32;
        pf[1] = 0x400; // DDPF_ZBUFFER
        pf[3] = 16;
        pf[5] = 0xFFFF;
        let f: extern "stdcall" fn(u32, u32) -> u32 = std::mem::transmute(cb as usize);
        f(pf.as_ptr() as u32, ctx);
    }
    OK
}

// ---- IDirect3DDevice3 -----------------------------------------------------------------------

fn device_vtbl() -> &'static [u32] {
    vtable![qi, addref, release, dv_get_caps, dv_get_stats, dv_add_viewport, dv_delete_viewport, dv_next_viewport,
        dv_enum_tex, dv_begin_scene, dv_end_scene, dv_get_d3d, dv_set_cur_vp, dv_get_cur_vp, dv_set_target, dv_get_target,
        dv_begin, dv_begin_indexed, dv_vertex, dv_index, dv_end, dv_get_rs, dv_set_rs, dv_get_ls, dv_set_ls,
        dv_set_tf, dv_get_tf, dv_mul_tf, dv_draw, dv_draw_indexed, dv_set_clip, dv_get_clip, dv_draw_strided,
        dv_draw_indexed_strided, dv_draw_vb, dv_draw_indexed_vb, dv_sphere, dv_get_tex, dv_set_tex, dv_get_tss, dv_set_tss, dv_validate]
}

m! {
    dv_get_stats = OK, (a);
    dv_add_viewport = OK, (a);
    dv_delete_viewport = OK, (a);
    dv_next_viewport = OK, (a, b, c);
    dv_begin_scene = OK, ();
    dv_end_scene = OK, ();
    dv_set_cur_vp = OK, (a);
    dv_get_cur_vp = OK, (a);
    dv_set_target = OK, (a, b);
    dv_get_target = OK, (a);
    dv_begin = OK, (a, b, c);
    dv_begin_indexed = OK, (a, b, c, d, e);
    dv_vertex = OK, (a);
    dv_index = OK, (a);
    dv_end = OK, (a);
    dv_get_rs = OK, (a, b);
    dv_set_rs = OK, (a, b);
    dv_set_tf = OK, (a, b);
    dv_get_tf = OK, (a, b);
    dv_mul_tf = OK, (a, b);
    dv_set_clip = OK, (a);
    dv_get_clip = OK, (a);
    dv_draw_strided = OK, (a, b, c, d, e);
    dv_draw_indexed_strided = OK, (a, b, c, d, e, f, g);
    dv_draw_vb = OK, (a, b, c, d, e);
    dv_draw_indexed_vb = OK, (a, b, c, d, e);
    dv_sphere = OK, (a, b, c, d, e);
    dv_get_tex = OK, (a, b);
    dv_set_tex = OK, (a, b);
    dv_get_tss = OK, (a, b, c);
    dv_set_tss = OK, (a, b, c);
    dv_validate = OK, (a);
}

unsafe extern "stdcall" fn dv_get_caps(this: u32, hw: u32, hel: u32) -> u32 {
    log("dv_get_caps", this, &[hw, hel]);
    for (p, sw) in [(hw, false), (hel, true)] {
        if p != 0 {
            let n = (rd(p, 0) as usize).clamp(4, 252);
            std::ptr::copy_nonoverlapping(device_desc(sw).as_ptr(), p as usize as *mut u8, n);
            wr(p, 0, n as u32);
        }
    }
    OK
}

unsafe extern "stdcall" fn dv_get_d3d(this: u32, out: u32) -> u32 {
    log("dv_get_d3d", this, &[out]);
    put(out, direct3d3());
    OK
}

/// Formatos de textura oferecidos: RGB 565, ARGB 1555, ARGB 4444 e ARGB 8888.
unsafe extern "stdcall" fn dv_enum_tex(this: u32, cb: u32, ctx: u32) -> u32 {
    log("dv_enum_tex", this, &[cb, ctx]);
    if cb != 0 {
        let f: extern "stdcall" fn(u32, u32) -> u32 = std::mem::transmute(cb as usize);
        for (bits, r, g, b, a) in [(16u32, 0xF800u32, 0x7E0u32, 0x1Fu32, 0u32), (16, 0x7C00, 0x3E0, 0x1F, 0x8000), (16, 0xF00, 0xF0, 0xF, 0xF000), (32, 0xFF_0000, 0xFF00, 0xFF, 0xFF00_0000)] {
            let pf = [32u32, 0x40 | if a != 0 { 1 } else { 0 }, 0, bits, r, g, b, a];
            if f(pf.as_ptr() as u32, ctx) == 0 {
                break;
            }
        }
    }
    OK
}

/// Registra um desenho com o primeiro vértice em hexadecimal (para descobrir o formato usado).
unsafe fn log_verts(name: &str, this: u32, fvf: u32, verts: u32, args: &[u32]) {
    log(name, this, args);
    if trace() && verts != 0 {
        let stride = fvf_size(fvf);
        let v: Vec<String> = (0..stride / 4).map(|i| format!("{:08x}", rd(verts, i * 4))).collect();
        eprintln!("[d3d]   vértice[0] tamanho={stride}: {}", v.join(" "));
    }
}

/// Tamanho do vértice pelo FVF (ou pelos tipos antigos D3DVT_*: 1, 2, 3).
fn fvf_size(fvf: u32) -> u32 {
    match fvf {
        1..=3 => 32,
        _ => {
            let pos = match fvf & 0xE { 2 => 12, 4 => 16, 6 => 16, 8 => 20, 0xA => 24, 0xC => 28, _ => 0 };
            let n = (fvf >> 8) & 0xF;
            pos + if fvf & 0x10 != 0 { 12 } else { 0 } + if fvf & 0x20 != 0 { 4 } else { 0 } + if fvf & 0x40 != 0 { 4 } else { 0 } + if fvf & 0x80 != 0 { 4 } else { 0 } + n * 8
        }
    }
}

unsafe extern "stdcall" fn dv_draw(this: u32, pt: u32, fvf: u32, verts: u32, n: u32, flags: u32) -> u32 {
    log_verts("dv_draw", this, fvf, verts, &[pt, fvf, n, flags]);
    OK
}

unsafe extern "stdcall" fn dv_draw_indexed(this: u32, pt: u32, fvf: u32, verts: u32, nv: u32, idx: u32, ni: u32, flags: u32) -> u32 {
    log_verts("dv_draw_indexed", this, fvf, verts, &[pt, fvf, nv, idx, ni, flags]);
    OK
}

// ---- viewport, material, luz, textura, vertex buffer --------------------------------------------

fn viewport_vtbl() -> &'static [u32] {
    vtable![qi, addref, release, vp_init, vp_get, vp_set, vp_transform, vp_light_el, vp_set_bg, vp_get_bg, vp_set_bgd, vp_get_bgd,
        vp_clear, vp_add_light, vp_del_light, vp_next_light, vp_get2, vp_set2, vp_set_bgd2, vp_get_bgd2, vp_clear2]
}

m! {
    vp_init = OK, (a);
    vp_get = OK, (a);
    vp_set = OK, (a);
    vp_transform = OK, (a, b, c, d);
    vp_light_el = OK, (a, b);
    vp_set_bg = OK, (a);
    vp_get_bg = OK, (a, b);
    vp_set_bgd = OK, (a);
    vp_get_bgd = OK, (a, b);
    vp_clear = OK, (a, b, c);
    vp_add_light = OK, (a);
    vp_del_light = OK, (a);
    vp_next_light = OK, (a, b, c);
    vp_get2 = OK, (a);
    vp_set2 = OK, (a);
    vp_set_bgd2 = OK, (a);
    vp_get_bgd2 = OK, (a, b);
    vp_clear2 = OK, (a, b, c, d, e, f);
    mt_set = OK, (a);
    mt_get = OK, (a);
    lt_init = OK, (a);
    lt_set = OK, (a);
    lt_get = OK, (a);
    tx_palette = OK, (a, b);
    vb_unlock = OK, ();
    vb_process = OK, (a, b, c, d, e, f, g);
    vb_desc = OK, (a);
    vb_optimize = OK, (a, b);
}

fn material_vtbl() -> &'static [u32] {
    vtable![qi, addref, release, mt_set, mt_get, handle]
}

fn light_vtbl() -> &'static [u32] {
    vtable![qi, addref, release, lt_init, lt_set, lt_get]
}

fn texture_vtbl() -> &'static [u32] {
    vtable![qi, addref, release, handle, tx_palette, tx_load]
}

fn vb_vtbl() -> &'static [u32] {
    vtable![qi, addref, release, vb_lock, vb_unlock, vb_process, vb_desc, vb_optimize]
}

/// GetHandle de material e textura: o handle é o próprio objeto.
unsafe extern "stdcall" fn handle(this: u32, dev: u32, out: u32) -> u32 {
    log("GetHandle", this, &[dev]);
    put(out, this);
    OK
}

unsafe extern "stdcall" fn vb_lock(this: u32, flags: u32, ptr: u32, size: u32) -> u32 {
    log("vb_lock", this, &[flags]);
    let mut v = VBS.lock().unwrap();
    let Some(b) = v.get_mut(&this) else { return 0x8000_4005 };
    put(ptr, b.as_mut_ptr() as u32);
    put(size, b.len() as u32);
    OK
}

/// Copia a textura `src` para esta (as superfícies têm o mesmo tamanho).
unsafe extern "stdcall" fn tx_load(this: u32, src: u32) -> u32 {
    log("tx_load", this, &[src]);
    let (d, s) = {
        let t = TEXTURES.lock().unwrap();
        let of = |obj: u32| t.iter().find(|(_, &o)| o == obj).map(|(&sf, _)| sf);
        (of(this), of(src))
    };
    match (d, s) {
        (Some(d), Some(s)) => super::ddraw::copy_surface(d, s),
        _ => 0x8000_4005,
    }
}

/// Estados de luz (D3DLIGHTSTATE_*), devolvidos como foram gravados.
static LIGHT_STATE: Mutex<BTreeMap<u32, u32>> = Mutex::new(BTreeMap::new());

unsafe extern "stdcall" fn dv_get_ls(this: u32, ty: u32, out: u32) -> u32 {
    log("dv_get_ls", this, &[ty]);
    put(out, LIGHT_STATE.lock().unwrap().get(&ty).copied().unwrap_or(0));
    OK
}

extern "stdcall" fn dv_set_ls(this: u32, ty: u32, v: u32) -> u32 {
    log("dv_set_ls", this, &[ty, v]);
    LIGHT_STATE.lock().unwrap().insert(ty, v);
    OK
}

/// IDirect3DTexture2 de uma superfície (objeto -> superfície, um por superfície).
static TEXTURES: Mutex<BTreeMap<u32, u32>> = Mutex::new(BTreeMap::new());

pub fn texture_for(surface: u32) -> u32 {
    let obj = *TEXTURES.lock().unwrap().entry(surface).or_insert_with(|| new_obj(texture_vtbl()));
    log("texture_for", surface, &[obj]);
    obj
}
