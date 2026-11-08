//! ddraw.dll — DirectDraw (DirectX 5 and 6 interfaces: IDirectDraw/2/4, IDirectDrawSurface/2/3/4,
//! IDirectDrawPalette, IDirectDrawClipper), entirely in software.
//!
//! The DirectX 6 structures (DDSURFACEDESC2, DDSCAPS2) have the same offsets as the DirectX 5 ones,
//! they are just larger: descriptions honor the caller's `dwSize` (108 or 124).
//!
//! Each surface is a memory block in the requested format (8-bit paletted, 16/24/32-bit RGB).
//! The primary surface is "presented" by converting its pixels to 0xFFRRGGBB and copying them to the
//! buffer of the game window (the one from `SetCooperativeLevel`); the viewer (X11/browser/PNG) shows
//! that buffer the way it shows any other window. Presentation happens on `Flip`, on `Unlock`/`ReleaseDC`/`Blt`
//! to the primary and when the primary's palette changes.
//!
//! COM objects: a pointer to the function table (vtable), allocated and leaked on purpose. The state
//! lives in `DD`, indexed by the object pointer itself.

use super::{call3, call4, gdi32, user32};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

pub fn resolve(name: &str) -> Option<usize> {
    exports!(name, DirectDrawCreate, DirectDrawEnumerateA)
}

const DD_OK: u32 = 0;
const E_NOINTERFACE: u32 = 0x8000_4002;
const DDERR_UNSUPPORTED: u32 = 0x8000_4001;
const DDERR_INVALIDPARAMS: u32 = 0x8007_0057;
const DDERR_INVALIDOBJECT: u32 = 0x8876_0082;
const DDERR_NOTFOUND: u32 = 0x8876_00FF;
const DDERR_NOPALETTEATTACHED: u32 = 0x8876_023B;
const DDERR_NOCLIPPERATTACHED: u32 = 0x8876_0230;
const DDERR_NOCOLORKEY: u32 = 0x8876_00D7;
const DDERR_NOTLOCKED: u32 = 0x8876_0248;

// DDSURFACEDESC
const DDSD_CAPS: u32 = 0x1;
const DDSD_HEIGHT: u32 = 0x2;
const DDSD_WIDTH: u32 = 0x4;
const DDSD_PITCH: u32 = 0x8;
const DDSD_BACKBUFFERCOUNT: u32 = 0x20;
const DDSD_LPSURFACE: u32 = 0x800;
const DDSD_PIXELFORMAT: u32 = 0x1000;
const DDSD_CKSRCBLT: u32 = 0x10000;
const DDSD_REFRESHRATE: u32 = 0x40000;

// DDSCAPS
const DDSCAPS_BACKBUFFER: u32 = 0x4;
const DDSCAPS_COMPLEX: u32 = 0x8;
const DDSCAPS_FLIP: u32 = 0x10;
const DDSCAPS_FRONTBUFFER: u32 = 0x20;
const DDSCAPS_OFFSCREENPLAIN: u32 = 0x40;
const DDSCAPS_PRIMARYSURFACE: u32 = 0x200;
const DDSCAPS_SYSTEMMEMORY: u32 = 0x800;
const DDSCAPS_VIDEOMEMORY: u32 = 0x4000;
const DDSCAPS_VISIBLE: u32 = 0x8000;
const DDSCAPS_LOCALVIDMEM: u32 = 0x1000_0000;
const DDSCAPS_ZBUFFER: u32 = 0x2_0000;

// DDPIXELFORMAT
const DDPF_PALETTEINDEXED8: u32 = 0x20;
const DDPF_RGB: u32 = 0x40;

// Blt
const DDBLT_COLORFILL: u32 = 0x400;
const DDBLT_KEYSRC: u32 = 0x8000;
const DDBLT_KEYSRCOVERRIDE: u32 = 0x10000;
const DDBLTFAST_SRCCOLORKEY: u32 = 0x1;

// SetColorKey
const DDCKEY_COLORSPACE: u32 = 0x1;
const DDCKEY_SRCBLT: u32 = 0x8;

// SetCooperativeLevel
const DDSCL_FULLSCREEN: u32 = 0x1;

/// Total "video" memory reported to the game.
const VIDMEM: u32 = 8 << 20;

fn trace() -> bool {
    std::env::var_os("WINLAYER_DDRAW_TRACE").is_some()
}

// ---- state --------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
struct Format {
    bpp: u32,
    r: u32,
    g: u32,
    b: u32,
}

impl Format {
    fn of_bpp(bpp: u32) -> Format {
        match bpp {
            8 => Format { bpp: 8, r: 0, g: 0, b: 0 },
            15 => Format { bpp: 16, r: 0x7C00, g: 0x03E0, b: 0x001F },
            16 => Format { bpp: 16, r: 0xF800, g: 0x07E0, b: 0x001F },
            24 => Format { bpp: 24, r: 0xFF_0000, g: 0xFF00, b: 0xFF },
            _ => Format { bpp: 32, r: 0xFF_0000, g: 0xFF00, b: 0xFF },
        }
    }

    fn bytes(&self) -> usize {
        (self.bpp as usize).div_ceil(8)
    }
}

/// Converts a pixel value through the mask to 0..255.
fn channel(v: u32, mask: u32) -> u32 {
    if mask == 0 {
        return 0;
    }
    let shift = mask.trailing_zeros();
    let max = mask >> shift;
    ((v & mask) >> shift) * 255 / max
}

/// Converts 0..255 to the mask field.
fn unchannel(c: u32, mask: u32) -> u32 {
    if mask == 0 {
        return 0;
    }
    let shift = mask.trailing_zeros();
    let max = mask >> shift;
    ((c * max + 127) / 255) << shift
}

struct Surface {
    w: i32,
    h: i32,
    pitch: usize,
    fmt: Format,
    mem: Vec<u8>,
    caps: u32,
    /// Attached back surface (flip chain): object pointer, 0 = none.
    back: u32,
    /// Attached Z-buffer (0 = none).
    zbuffer: u32,
    primary: bool,
    palette: u32,
    clipper: u32,
    ck_src: Option<(u32, u32)>,
    ck_dst: Option<(u32, u32)>,
    refs: u32,
    /// GetDC in progress: (hdc, hbitmap, copy of the pixels handed to GDI).
    dc: Option<(u32, u32, Vec<u32>)>,
}

impl Surface {
    fn new(w: i32, h: i32, fmt: Format, caps: u32) -> Surface {
        let pitch = (w as usize * fmt.bytes() + 3) & !3;
        Surface { w, h, pitch, fmt, mem: vec![0; pitch * h.max(1) as usize], caps, back: 0, zbuffer: 0, primary: false, palette: 0, clipper: 0, ck_src: None, ck_dst: None, refs: 1, dc: None }
    }

    fn get(&self, x: i32, y: i32) -> u32 {
        let o = y as usize * self.pitch + x as usize * self.fmt.bytes();
        let m = &self.mem;
        match self.fmt.bytes() {
            1 => m[o] as u32,
            2 => u16::from_le_bytes([m[o], m[o + 1]]) as u32,
            3 => m[o] as u32 | (m[o + 1] as u32) << 8 | (m[o + 2] as u32) << 16,
            _ => u32::from_le_bytes([m[o], m[o + 1], m[o + 2], m[o + 3]]),
        }
    }

    fn put(&mut self, x: i32, y: i32, v: u32) {
        let n = self.fmt.bytes();
        let o = y as usize * self.pitch + x as usize * n;
        self.mem[o..o + n].copy_from_slice(&v.to_le_bytes()[..n]);
    }
}

struct Palette {
    /// 0xFFRRGGBB by index.
    rgb: [u32; 256],
    refs: u32,
}

struct State {
    /// IDirectDraw and IDirectDraw2 objects (same state).
    dd1: u32,
    dd2: u32,
    dd4: u32,
    hwnd: u32,
    coop: u32,
    mode: (i32, i32, u32),
    surfaces: BTreeMap<u32, Surface>,
    palettes: BTreeMap<u32, Palette>,
    /// Clipper -> hwnd.
    clippers: BTreeMap<u32, u32>,
    primary: u32,
}

static DD: Mutex<State> = Mutex::new(State {
    dd1: 0,
    dd2: 0,
    dd4: 0,
    hwnd: 0,
    coop: 0,
    mode: (0, 0, 0),
    surfaces: BTreeMap::new(),
    palettes: BTreeMap::new(),
    clippers: BTreeMap::new(),
    primary: 0,
});

/// The game is using DirectDraw (capture mode does not exit at the first idle moment).
static ACTIVE: AtomicBool = AtomicBool::new(false);
static FRAMES: AtomicU32 = AtomicU32::new(0);
static LAST_VBLANK: AtomicU32 = AtomicU32::new(0);

pub fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

/// Current video mode: the one requested by `SetDisplayMode` or the desktop's (16-bit).
fn mode(s: &State) -> (i32, i32, u32) {
    if s.mode.0 > 0 {
        s.mode
    } else {
        (user32::desktop_w() as i32, user32::desktop_h() as i32, 16)
    }
}

// ---- COM objects --------------------------------------------------------------------------

fn new_obj(vtbl: &'static [u32]) -> u32 {
    Box::leak(Box::new(vtbl.as_ptr() as u32)) as *mut u32 as u32
}

macro_rules! vtable {
    ($($f:expr),* $(,)?) => {{
        static V: OnceLock<Vec<u32>> = OnceLock::new();
        V.get_or_init(|| vec![$($f as *const () as usize as u32),*]).as_slice()
    }};
}

unsafe fn rd(p: u32, o: u32) -> u32 {
    std::ptr::read_unaligned((p + o) as usize as *const u32)
}

unsafe fn wr(p: u32, o: u32, v: u32) {
    std::ptr::write_unaligned((p + o) as usize as *mut u32, v)
}

unsafe fn zero(p: u32, n: usize) {
    std::ptr::write_bytes(p as usize as *mut u8, 0, n);
}

/// First dword of the GUID (enough to tell the DirectDraw interfaces apart).
unsafe fn iid(p: u32) -> u32 {
    if p == 0 { 0 } else { rd(p, 0) }
}

unsafe fn rect(p: u32) -> Option<(i32, i32, i32, i32)> {
    if p == 0 {
        None
    } else {
        Some((rd(p, 0) as i32, rd(p, 4) as i32, rd(p, 8) as i32, rd(p, 12) as i32))
    }
}

// ---- exported API -------------------------------------------------------------------------

#[allow(non_snake_case)]
unsafe extern "stdcall" fn DirectDrawCreate(_guid: u32, out: *mut u32, _outer: u32) -> u32 {
    if out.is_null() {
        return DDERR_INVALIDPARAMS;
    }
    let mut s = DD.lock().unwrap();
    if s.dd1 == 0 {
        s.dd1 = new_obj(dd1_vtbl());
        s.dd2 = new_obj(dd2_vtbl());
        s.dd4 = new_obj(dd4_vtbl());
    }
    ACTIVE.store(true, Ordering::Relaxed);
    if trace() {
        eprintln!("[ddraw] DirectDrawCreate -> {:#x}", s.dd1);
    }
    *out = s.dd1;
    DD_OK
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn DirectDrawEnumerateA(cb: u32, ctx: u32) -> u32 {
    if cb != 0 {
        call4(cb, 0, c"Display".as_ptr() as u32, c"display".as_ptr() as u32, ctx);
    }
    DD_OK
}

// ---- IDirectDraw / IDirectDraw2 -------------------------------------------------------------

fn dd1_vtbl() -> &'static [u32] {
    vtable![
        dd_qi, dd_addref, dd_release, dd_ok1, dd_create_clipper, dd_create_palette, dd_create_surface,
        dd_duplicate_surface, dd_enum_modes, dd_enum_surfaces, dd_ok1, dd_get_caps, dd_get_display_mode,
        dd_get_fourcc, dd_get_gdi_surface, dd_get_monitor_freq, dd_get_scanline, dd_get_vblank_status,
        dd_ok2, dd_ok1, dd_set_coop, dd_set_mode1, dd_wait_vblank,
    ]
}

fn dd4_vtbl() -> &'static [u32] {
    vtable![
        dd_qi, dd_addref, dd_release, dd_ok1, dd_create_clipper, dd_create_palette, dd_create_surface,
        dd_duplicate_surface, dd_enum_modes, dd_enum_surfaces, dd_ok1, dd_get_caps, dd_get_display_mode,
        dd_get_fourcc, dd_get_gdi_surface, dd_get_monitor_freq, dd_get_scanline, dd_get_vblank_status,
        dd_ok2, dd_ok1, dd_set_coop, dd_set_mode2, dd_wait_vblank, dd_get_vidmem,
        // IDirectDraw4
        dd_get_surface_from_dc, dd_ok1, dd_ok1, dd_get_device_identifier,
    ]
}

unsafe extern "stdcall" fn dd_get_surface_from_dc(_this: u32, _hdc: u32, out: *mut u32) -> u32 {
    if !out.is_null() {
        *out = 0;
    }
    DDERR_NOTFOUND
}

/// DDDEVICEIDENTIFIER: szDriver[512], szDescription[512], liDriverVersion, 4 ids, GUID.
unsafe extern "stdcall" fn dd_get_device_identifier(_this: u32, out: u32, _flags: u32) -> u32 {
    if out == 0 {
        return DDERR_INVALIDPARAMS;
    }
    zero(out, 1064);
    for (o, text) in [(0u32, &b"winlayer.drv"[..]), (512, &b"winlayer DirectDraw (software)"[..])] {
        std::ptr::copy_nonoverlapping(text.as_ptr(), (out + o) as usize as *mut u8, text.len());
    }
    wr(out, 1024, 1); // driver version (low part)
    DD_OK
}

fn dd2_vtbl() -> &'static [u32] {
    vtable![
        dd_qi, dd_addref, dd_release, dd_ok1, dd_create_clipper, dd_create_palette, dd_create_surface,
        dd_duplicate_surface, dd_enum_modes, dd_enum_surfaces, dd_ok1, dd_get_caps, dd_get_display_mode,
        dd_get_fourcc, dd_get_gdi_surface, dd_get_monitor_freq, dd_get_scanline, dd_get_vblank_status,
        dd_ok2, dd_ok1, dd_set_coop, dd_set_mode2, dd_wait_vblank, dd_get_vidmem,
    ]
}

unsafe extern "stdcall" fn dd_qi(_this: u32, riid: u32, out: *mut u32) -> u32 {
    let s = DD.lock().unwrap();
    let obj = match iid(riid) {
        0x0000_0000 | 0x6C14_DB80 => s.dd1, // IUnknown, IDirectDraw
        0xB3A6_F3E0 => s.dd2,               // IDirectDraw2
        0x9C59_509A => s.dd4,               // IDirectDraw4
        super::d3d::IID_D3D3 => super::d3d::direct3d3(),
        other => {
            if trace() {
                eprintln!("[ddraw] QueryInterface: interface {other:#010x} not supported");
            }
            if !out.is_null() {
                *out = 0;
            }
            return E_NOINTERFACE;
        }
    };
    *out = obj;
    DD_OK
}

extern "stdcall" fn dd_addref(_this: u32) -> u32 {
    1
}

extern "stdcall" fn dd_release(_this: u32) -> u32 {
    0
}

extern "stdcall" fn dd_ok1(this: u32) -> u32 {
    if trace() {
        eprintln!("[ddraw] method dd_ok1 on {this:#x}");
    }
    DD_OK
}

extern "stdcall" fn dd_ok2(this: u32, _a: u32) -> u32 {
    if trace() {
        eprintln!("[ddraw] method dd_ok2 on {this:#x}");
    }
    DD_OK
}

unsafe extern "stdcall" fn dd_create_clipper(_this: u32, _flags: u32, out: *mut u32, _outer: u32) -> u32 {
    let obj = new_obj(clipper_vtbl());
    DD.lock().unwrap().clippers.insert(obj, 0);
    *out = obj;
    DD_OK
}

unsafe extern "stdcall" fn dd_create_palette(_this: u32, flags: u32, entries: u32, out: *mut u32, _outer: u32) -> u32 {
    if trace() {
        eprintln!("[ddraw] CreatePalette flags={flags:#x}");
    }
    // DDPCAPS_1BIT=0x100, 2BIT=0x200, 4BIT=0x1, 8BIT=0x4
    let n = if flags & 0x4 != 0 { 256 } else if flags & 0x1 != 0 { 16 } else if flags & 0x200 != 0 { 4 } else { 2 };
    let mut rgb = [0xFF00_0000u32; 256];
    if entries != 0 {
        for (i, c) in rgb.iter_mut().enumerate().take(n) {
            let e = rd(entries, i as u32 * 4);
            *c = 0xFF00_0000 | ((e & 0xFF) << 16) | (e & 0xFF00) | ((e >> 16) & 0xFF);
        }
    }
    let obj = new_obj(palette_vtbl());
    DD.lock().unwrap().palettes.insert(obj, Palette { rgb, refs: 1 });
    *out = obj;
    DD_OK
}

unsafe fn read_format(pf: u32) -> Format {
    let flags = rd(pf, 4);
    let bits = rd(pf, 12);
    if flags & DDPF_PALETTEINDEXED8 != 0 || (flags & DDPF_RGB == 0 && bits == 8) {
        return Format::of_bpp(8);
    }
    let (r, g, b) = (rd(pf, 16), rd(pf, 20), rd(pf, 24));
    if r == 0 && g == 0 && b == 0 {
        Format::of_bpp(bits)
    } else {
        Format { bpp: bits, r, g, b }
    }
}

unsafe fn write_format(pf: u32, f: Format) {
    zero(pf, 32);
    wr(pf, 0, 32);
    if f.bpp == 8 {
        wr(pf, 4, DDPF_RGB | DDPF_PALETTEINDEXED8);
        wr(pf, 12, 8);
    } else {
        wr(pf, 4, DDPF_RGB);
        wr(pf, 12, f.bpp);
        wr(pf, 16, f.r);
        wr(pf, 20, f.g);
        wr(pf, 24, f.b);
    }
}

unsafe extern "stdcall" fn dd_create_surface(_this: u32, desc: u32, out: *mut u32, _outer: u32) -> u32 {
    if desc == 0 || out.is_null() {
        return DDERR_INVALIDPARAMS;
    }
    let flags = rd(desc, 4);
    let caps = if flags & DDSD_CAPS != 0 { rd(desc, 104) } else { DDSCAPS_OFFSCREENPLAIN };
    let mut s = DD.lock().unwrap();
    let (mw, mh, mbpp) = mode(&s);
    let obj;
    if caps & DDSCAPS_PRIMARYSURFACE != 0 {
        let fmt = Format::of_bpp(mbpp);
        let backs = if flags & DDSD_BACKBUFFERCOUNT != 0 && caps & DDSCAPS_FLIP != 0 { rd(desc, 20).max(1) } else { 0 };
        let mut p = Surface::new(mw, mh, fmt, caps | DDSCAPS_VISIBLE | DDSCAPS_VIDEOMEMORY | DDSCAPS_LOCALVIDMEM | if backs > 0 { DDSCAPS_FRONTBUFFER } else { 0 });
        p.primary = true;
        obj = new_obj(surface_vtbl());
        // chain: primary -> back1 -> back2 ... (the last one points back to the primary only via Flip)
        let mut prev = obj;
        let mut chain = Vec::new();
        for i in 0..backs {
            let b = new_obj(surface_vtbl());
            let bcaps = (caps & !(DDSCAPS_PRIMARYSURFACE | DDSCAPS_VISIBLE)) | DDSCAPS_VIDEOMEMORY | DDSCAPS_LOCALVIDMEM | DDSCAPS_FLIP | DDSCAPS_COMPLEX
                | if i == 0 { DDSCAPS_BACKBUFFER } else { 0 };
            chain.push((prev, b, Surface::new(mw, mh, fmt, bcaps)));
            prev = b;
        }
        s.surfaces.insert(obj, p);
        for (parent, b, surf) in chain {
            s.surfaces.insert(b, surf);
            if let Some(ps) = s.surfaces.get_mut(&parent) {
                ps.back = b;
            }
        }
        s.primary = obj;
        if trace() {
            eprintln!("[ddraw] CreateSurface primary {mw}x{mh}x{mbpp} with {backs} back buffer(s) -> {obj:#x}");
        }
    } else {
        if flags & (DDSD_WIDTH | DDSD_HEIGHT) != (DDSD_WIDTH | DDSD_HEIGHT) {
            return DDERR_INVALIDPARAMS;
        }
        let (w, h) = (rd(desc, 12) as i32, rd(desc, 8) as i32);
        if w <= 0 || h <= 0 || w > 8192 || h > 8192 {
            return DDERR_INVALIDPARAMS;
        }
        let fmt = if flags & DDSD_PIXELFORMAT != 0 { read_format(desc + 72) } else { Format::of_bpp(mbpp) };
        let mem_caps = if caps & DDSCAPS_SYSTEMMEMORY != 0 { 0 } else { DDSCAPS_VIDEOMEMORY | DDSCAPS_LOCALVIDMEM };
        let mut surf = Surface::new(w, h, fmt, caps | mem_caps);
        if flags & DDSD_CKSRCBLT != 0 {
            surf.ck_src = Some((rd(desc, 64), rd(desc, 68)));
        }
        obj = new_obj(surface_vtbl());
        if trace() {
            eprintln!("[ddraw] CreateSurface {w}x{h}x{} caps={caps:#x} -> {obj:#x}", fmt.bpp);
        }
        s.surfaces.insert(obj, surf);
    }
    *out = obj;
    DD_OK
}

unsafe extern "stdcall" fn dd_duplicate_surface(_this: u32, src: u32, out: *mut u32) -> u32 {
    let mut s = DD.lock().unwrap();
    let Some(o) = s.surfaces.get(&src) else { return DDERR_INVALIDOBJECT };
    let mut n = Surface::new(o.w, o.h, o.fmt, o.caps & !(DDSCAPS_PRIMARYSURFACE | DDSCAPS_FRONTBUFFER | DDSCAPS_BACKBUFFER));
    n.mem = o.mem.clone();
    n.ck_src = o.ck_src;
    n.palette = o.palette;
    let obj = new_obj(surface_vtbl());
    s.surfaces.insert(obj, n);
    *out = obj;
    DD_OK
}

unsafe fn fill_desc(desc: u32, w: i32, h: i32, pitch: usize, fmt: Format, caps: u32) {
    // DDSURFACEDESC (108 bytes) or DDSURFACEDESC2 (124): keeps the caller's size.
    let size = if rd(desc, 0) == 124 { 124 } else { 108 };
    zero(desc, size as usize);
    wr(desc, 0, size);
    wr(desc, 4, DDSD_CAPS | DDSD_HEIGHT | DDSD_WIDTH | DDSD_PITCH | DDSD_PIXELFORMAT);
    wr(desc, 8, h as u32);
    wr(desc, 12, w as u32);
    wr(desc, 16, pitch as u32);
    write_format(desc + 72, fmt);
    wr(desc, 104, caps);
}

unsafe extern "stdcall" fn dd_enum_modes(this: u32, _flags: u32, filter: u32, ctx: u32, cb: u32) -> u32 {
    let v4 = DD.lock().unwrap().dd4 == this;
    if cb == 0 {
        return DDERR_INVALIDPARAMS;
    }
    let want = if filter != 0 {
        let f = rd(filter, 4);
        (
            if f & DDSD_WIDTH != 0 { rd(filter, 12) as i32 } else { 0 },
            if f & DDSD_HEIGHT != 0 { rd(filter, 8) as i32 } else { 0 },
            if f & DDSD_PIXELFORMAT != 0 { rd(filter, 72 + 12) } else { 0 },
        )
    } else {
        (0, 0, 0)
    };
    let mut desc = [0u8; 124];
    let p = desc.as_mut_ptr() as u32;
    for (w, h) in [(320, 200), (320, 240), (640, 400), (640, 480), (800, 600), (1024, 768)] {
        for bpp in [8u32, 16, 24, 32] {
            if (want.0 != 0 && want.0 != w) || (want.1 != 0 && want.1 != h) || (want.2 != 0 && want.2 != bpp) {
                continue;
            }
            let fmt = Format::of_bpp(bpp);
            wr(p, 0, if v4 { 124 } else { 108 });
            fill_desc(p, w, h, (w as usize * fmt.bytes() + 3) & !3, fmt, 0);
            wr(p, 4, rd(p, 4) | DDSD_REFRESHRATE);
            wr(p, 24, 60);
            if call2(cb, p, ctx) == 0 {
                return DD_OK; // DDENUMRET_CANCEL
            }
        }
    }
    DD_OK
}

unsafe fn call2(f: u32, a: u32, b: u32) -> u32 {
    let f: extern "stdcall" fn(u32, u32) -> u32 = std::mem::transmute(f as usize);
    f(a, b)
}

extern "stdcall" fn dd_enum_surfaces(this: u32, _flags: u32, _desc: u32, _ctx: u32, _cb: u32) -> u32 {
    if trace() {
        eprintln!("[ddraw] method dd_enum_surfaces on {this:#x}");
    }
    DD_OK
}

unsafe fn fill_caps(caps: u32) {
    if caps == 0 {
        return;
    }
    let size = rd(caps, 0).clamp(4, 512) as usize;
    zero(caps + 4, size - 4);
    // DDCAPS_BLT | BLTCOLORFILL | BLTSTRETCH | COLORKEY | PALETTE
    wr(caps, 4, 0x40 | 0x0400_0000 | 0x200 | 0x40_0000 | 0x8000);
    if size >= 16 {
        wr(caps, 12, 0x200); // dwCKeyCaps: DDCKEYCAPS_SRCBLT
    }
    if size >= 28 {
        wr(caps, 24, 0x4 | 0x40); // dwPalCaps: 8BIT | ALLOW256
    }
    if size >= 68 {
        wr(caps, 60, VIDMEM);
        wr(caps, 64, VIDMEM);
    }
}

unsafe extern "stdcall" fn dd_get_caps(_this: u32, drv: u32, hel: u32) -> u32 {
    fill_caps(drv);
    fill_caps(hel);
    DD_OK
}

unsafe extern "stdcall" fn dd_get_display_mode(_this: u32, desc: u32) -> u32 {
    let (w, h, bpp) = mode(&DD.lock().unwrap());
    let fmt = Format::of_bpp(bpp);
    fill_desc(desc, w, h, (w as usize * fmt.bytes() + 3) & !3, fmt, 0);
    wr(desc, 4, rd(desc, 4) | DDSD_REFRESHRATE);
    wr(desc, 24, 60);
    DD_OK
}

unsafe extern "stdcall" fn dd_get_fourcc(_this: u32, num: *mut u32, _codes: u32) -> u32 {
    if !num.is_null() {
        *num = 0;
    }
    DD_OK
}

unsafe extern "stdcall" fn dd_get_gdi_surface(_this: u32, out: *mut u32) -> u32 {
    let s = DD.lock().unwrap();
    if s.primary == 0 {
        return DDERR_NOTFOUND;
    }
    *out = s.primary;
    DD_OK
}

unsafe extern "stdcall" fn dd_get_monitor_freq(_this: u32, out: *mut u32) -> u32 {
    if !out.is_null() {
        *out = 60;
    }
    DD_OK
}

unsafe extern "stdcall" fn dd_get_scanline(_this: u32, out: *mut u32) -> u32 {
    let h = mode(&DD.lock().unwrap()).1.max(1) as u32;
    if !out.is_null() {
        let us = user32::real_ms() as u64 * 1000;
        *out = ((us % 16_667) * (h as u64 + 45) / 16_667) as u32 % h;
    }
    DD_OK
}

unsafe extern "stdcall" fn dd_get_vblank_status(_this: u32, out: *mut u32) -> u32 {
    if !out.is_null() {
        *out = (user32::real_ms() % 17 < 2) as u32;
    }
    DD_OK
}

unsafe extern "stdcall" fn dd_set_coop(_this: u32, hwnd: u32, flags: u32) -> u32 {
    let mut s = DD.lock().unwrap();
    if hwnd != 0 {
        s.hwnd = hwnd;
    }
    s.coop = flags;
    if trace() {
        eprintln!("[ddraw] SetCooperativeLevel hwnd={hwnd:#x} flags={flags:#x}");
    }
    DD_OK
}

fn set_mode(w: u32, h: u32, bpp: u32) -> u32 {
    if trace() {
        eprintln!("[ddraw] SetDisplayMode {w}x{h}x{bpp}");
    }
    if w == 0 || h == 0 || w > 4096 || h > 4096 {
        return DDERR_INVALIDPARAMS;
    }
    let (hwnd, full) = {
        let mut s = DD.lock().unwrap();
        s.mode = (w as i32, h as i32, if bpp == 0 { 16 } else { bpp });
        (s.hwnd, s.coop & DDSCL_FULLSCREEN != 0)
    };
    // The "screen" takes the size of the mode; the viewer scales it to the real window.
    user32::lock_screen(w as usize, h as usize);
    if full && hwnd != 0 {
        user32::MoveWindow(hwnd, 0, 0, w as i32, h as i32, 1);
    }
    DD_OK
}

extern "stdcall" fn dd_set_mode1(_this: u32, w: u32, h: u32, bpp: u32) -> u32 {
    set_mode(w, h, bpp)
}

extern "stdcall" fn dd_set_mode2(_this: u32, w: u32, h: u32, bpp: u32, _refresh: u32, _flags: u32) -> u32 {
    set_mode(w, h, bpp)
}

/// Waits for the next "vertical retrace" (60 Hz). In capture mode time is virtual: it only advances the clock.
fn wait_vblank() {
    if user32::shot_mode() {
        user32::tick_virtual(16);
        return;
    }
    let now = user32::real_ms();
    let next = LAST_VBLANK.load(Ordering::Relaxed) + 16;
    if next > now && next - now <= 16 {
        std::thread::sleep(std::time::Duration::from_millis((next - now) as u64));
    }
    LAST_VBLANK.store(user32::real_ms(), Ordering::Relaxed);
}

extern "stdcall" fn dd_wait_vblank(_this: u32, _flags: u32, _event: u32) -> u32 {
    wait_vblank();
    DD_OK
}

unsafe extern "stdcall" fn dd_get_vidmem(_this: u32, _caps: u32, total: *mut u32, free: *mut u32) -> u32 {
    if !total.is_null() {
        *total = VIDMEM;
    }
    if !free.is_null() {
        *free = VIDMEM;
    }
    DD_OK
}

// ---- IDirectDrawSurface / 2 / 3 -----------------------------------------------------------

fn surface_vtbl() -> &'static [u32] {
    vtable![
        sf_qi, sf_addref, sf_release, sf_add_attached, sf_unsupported2, sf_blt, sf_unsupported4, sf_blt_fast,
        sf_delete_attached, sf_enum_attached, sf_unsupported4, sf_flip, sf_get_attached, sf_ok2, sf_get_caps,
        sf_get_clipper, sf_get_color_key, sf_get_dc, sf_ok2, sf_unsupported3, sf_get_palette, sf_get_pixel_format,
        sf_get_desc, sf_ok3, sf_ok1, sf_lock, sf_release_dc, sf_ok1, sf_set_clipper, sf_set_color_key,
        sf_unsupported3, sf_set_palette, sf_unlock, sf_unsupported6, sf_ok2, sf_unsupported3,
        // IDirectDrawSurface2
        sf_get_dd_interface, sf_ok2, sf_ok2,
        // IDirectDrawSurface3
        sf_unsupported3,
        // IDirectDrawSurface4: SetPrivateData, GetPrivateData, FreePrivateData, GetUniquenessValue, ChangeUniquenessValue
        sf_unsupported5, sf_unsupported4, sf_ok2, sf_get_uniqueness, sf_ok1,
    ]
}

unsafe extern "stdcall" fn sf_qi(this: u32, riid: u32, out: *mut u32) -> u32 {
    match iid(riid) {
        // IUnknown, IDirectDrawSurface, 2, 3
        0x0000_0000 | 0x6C14_DB81 | 0x5780_5885 | 0xDA04_4E00 | 0x0B2B_8630 => {
            if let Some(sf) = DD.lock().unwrap().surfaces.get_mut(&this) {
                sf.refs += 1;
            }
            *out = this;
            DD_OK
        }
        super::d3d::IID_TEXTURE2 => {
            *out = super::d3d::texture_for(this);
            DD_OK
        }
        other => {
            if trace() {
                eprintln!("[ddraw] Surface::QueryInterface: {other:#010x} not supported");
            }
            *out = 0;
            E_NOINTERFACE
        }
    }
}

extern "stdcall" fn sf_addref(this: u32) -> u32 {
    let mut s = DD.lock().unwrap();
    match s.surfaces.get_mut(&this) {
        Some(sf) => {
            sf.refs += 1;
            sf.refs
        }
        None => 0,
    }
}

extern "stdcall" fn sf_release(this: u32) -> u32 {
    let mut s = DD.lock().unwrap();
    let Some(sf) = s.surfaces.get_mut(&this) else { return 0 };
    sf.refs = sf.refs.saturating_sub(1);
    let left = sf.refs;
    if left == 0 {
        let mut next = s.surfaces.remove(&this).map_or(0, |x| if x.primary { x.back } else { 0 });
        // the primary takes along the back buffers created with it
        while next != 0 && next != this {
            next = s.surfaces.remove(&next).map_or(0, |x| x.back);
        }
        if s.primary == this {
            s.primary = 0;
        }
    }
    left
}

extern "stdcall" fn sf_ok1(this: u32) -> u32 {
    if trace() {
        eprintln!("[ddraw] method sf_ok1 on {this:#x}");
    }
    DD_OK
}

extern "stdcall" fn sf_ok2(this: u32, _a: u32) -> u32 {
    if trace() {
        eprintln!("[ddraw] method sf_ok2 on {this:#x}");
    }
    DD_OK
}

extern "stdcall" fn sf_ok3(this: u32, _a: u32, _b: u32) -> u32 {
    if trace() {
        eprintln!("[ddraw] method sf_ok3 on {this:#x}");
    }
    DD_OK
}

extern "stdcall" fn sf_unsupported2(this: u32, _a: u32) -> u32 {
    if trace() {
        eprintln!("[ddraw] method sf_unsupported2 on {this:#x}");
    }
    DDERR_UNSUPPORTED
}

extern "stdcall" fn sf_unsupported3(this: u32, _a: u32, _b: u32) -> u32 {
    if trace() {
        eprintln!("[ddraw] method sf_unsupported3 on {this:#x}");
    }
    DDERR_UNSUPPORTED
}

extern "stdcall" fn sf_unsupported4(this: u32, _a: u32, _b: u32, _c: u32) -> u32 {
    if trace() {
        eprintln!("[ddraw] method sf_unsupported4 on {this:#x}");
    }
    DDERR_UNSUPPORTED
}

extern "stdcall" fn sf_unsupported5(_this: u32, _a: u32, _b: u32, _c: u32, _d: u32) -> u32 {
    DDERR_UNSUPPORTED
}

unsafe extern "stdcall" fn sf_get_uniqueness(_this: u32, out: *mut u32) -> u32 {
    if !out.is_null() {
        *out = FRAMES.load(Ordering::Relaxed);
    }
    DD_OK
}

extern "stdcall" fn sf_unsupported6(this: u32, _a: u32, _b: u32, _c: u32, _d: u32, _e: u32) -> u32 {
    if trace() {
        eprintln!("[ddraw] method sf_unsupported6 on {this:#x}");
    }
    DDERR_UNSUPPORTED
}

extern "stdcall" fn sf_add_attached(this: u32, other: u32) -> u32 {
    let mut s = DD.lock().unwrap();
    if !s.surfaces.contains_key(&other) {
        return DDERR_INVALIDOBJECT;
    }
    // An attached z-buffer is not part of the flip chain.
    let is_z = s.surfaces[&other].caps & DDSCAPS_ZBUFFER != 0;
    if let Some(sf) = s.surfaces.get_mut(&this) {
        if is_z {
            sf.zbuffer = other;
        } else {
            sf.back = other;
        }
    }
    if let Some(o) = s.surfaces.get_mut(&other) {
        o.refs += 1;
        if !is_z {
            o.caps |= DDSCAPS_BACKBUFFER;
        }
    }
    DD_OK
}

extern "stdcall" fn sf_delete_attached(this: u32, _flags: u32, other: u32) -> u32 {
    let mut s = DD.lock().unwrap();
    if let Some(sf) = s.surfaces.get_mut(&this) {
        if other == 0 || sf.back == other {
            sf.back = 0;
        }
    }
    DD_OK
}

unsafe extern "stdcall" fn sf_enum_attached(this: u32, ctx: u32, cb: u32) -> u32 {
    let info = {
        let mut s = DD.lock().unwrap();
        let back = s.surfaces.get(&this).map_or(0, |x| x.back);
        match s.surfaces.get_mut(&back) {
            Some(b) => {
                b.refs += 1;
                Some((back, b.w, b.h, b.pitch, b.fmt, b.caps))
            }
            None => None,
        }
    };
    if let (Some((back, w, h, pitch, fmt, caps)), true) = (info, cb != 0) {
        let mut desc = [0u8; 124];
        let p = desc.as_mut_ptr() as u32;
        fill_desc(p, w, h, pitch, fmt, caps);
        call3(cb, back, p, ctx);
    }
    DD_OK
}

unsafe extern "stdcall" fn sf_get_attached(this: u32, caps: u32, out: *mut u32) -> u32 {
    let want = if caps == 0 { 0 } else { rd(caps, 0) };
    let mut s = DD.lock().unwrap();
    let back = s.surfaces.get(&this).map_or(0, |x| x.back);
    match s.surfaces.get_mut(&back) {
        Some(b) if want == 0 || b.caps & want != 0 || want & (DDSCAPS_BACKBUFFER | DDSCAPS_FLIP) != 0 => {
            b.refs += 1;
            *out = back;
            DD_OK
        }
        _ => {
            if trace() {
                eprintln!("[ddraw] GetAttachedSurface({want:#x}): none");
            }
            *out = 0;
            DDERR_NOTFOUND
        }
    }
}

unsafe extern "stdcall" fn sf_get_caps(this: u32, out: *mut u32) -> u32 {
    let s = DD.lock().unwrap();
    let Some(sf) = s.surfaces.get(&this) else { return DDERR_INVALIDOBJECT };
    *out = sf.caps;
    DD_OK
}

unsafe extern "stdcall" fn sf_get_clipper(this: u32, out: *mut u32) -> u32 {
    let s = DD.lock().unwrap();
    match s.surfaces.get(&this).map(|x| x.clipper) {
        Some(c) if c != 0 => {
            *out = c;
            DD_OK
        }
        _ => DDERR_NOCLIPPERATTACHED,
    }
}

unsafe extern "stdcall" fn sf_set_clipper(this: u32, clipper: u32) -> u32 {
    if let Some(sf) = DD.lock().unwrap().surfaces.get_mut(&this) {
        sf.clipper = clipper;
    }
    DD_OK
}

unsafe extern "stdcall" fn sf_get_color_key(this: u32, flags: u32, out: u32) -> u32 {
    let s = DD.lock().unwrap();
    let Some(sf) = s.surfaces.get(&this) else { return DDERR_INVALIDOBJECT };
    let ck = if flags & DDCKEY_SRCBLT != 0 { sf.ck_src } else { sf.ck_dst };
    match ck {
        Some((lo, hi)) => {
            wr(out, 0, lo);
            wr(out, 4, hi);
            DD_OK
        }
        None => DDERR_NOCOLORKEY,
    }
}

unsafe extern "stdcall" fn sf_set_color_key(this: u32, flags: u32, key: u32) -> u32 {
    if flags & DDCKEY_COLORSPACE != 0 && trace() {
        eprintln!("[ddraw] SetColorKey with a color range (only the lower bound is used)");
    }
    let mut s = DD.lock().unwrap();
    let Some(sf) = s.surfaces.get_mut(&this) else { return DDERR_INVALIDOBJECT };
    let v = if key == 0 { None } else { Some((rd(key, 0), rd(key, 4))) };
    if flags & DDCKEY_SRCBLT != 0 {
        sf.ck_src = v;
    } else {
        sf.ck_dst = v;
    }
    DD_OK
}

unsafe extern "stdcall" fn sf_get_palette(this: u32, out: *mut u32) -> u32 {
    let mut s = DD.lock().unwrap();
    let pal = s.surfaces.get(&this).map_or(0, |x| x.palette);
    match s.palettes.get_mut(&pal) {
        Some(p) => {
            p.refs += 1;
            *out = pal;
            DD_OK
        }
        None => DDERR_NOPALETTEATTACHED,
    }
}

unsafe extern "stdcall" fn sf_set_palette(this: u32, pal: u32) -> u32 {
    if trace() {
        eprintln!("[ddraw] SetPalette {this:#x} <- {pal:#x}");
    }
    let primary = {
        let mut s = DD.lock().unwrap();
        if let Some(p) = s.palettes.get_mut(&pal) {
            p.refs += 1;
        }
        let Some(sf) = s.surfaces.get_mut(&this) else { return DDERR_INVALIDOBJECT };
        sf.palette = pal;
        let primary = sf.primary;
        // the primary's palette applies to the whole flip chain
        if primary {
            let mut b = sf.back;
            while b != 0 && b != this {
                match s.surfaces.get_mut(&b) {
                    Some(x) => {
                        x.palette = pal;
                        b = x.back;
                    }
                    None => break,
                }
            }
        }
        primary
    };
    if primary {
        present();
    }
    DD_OK
}

unsafe extern "stdcall" fn sf_get_pixel_format(this: u32, pf: u32) -> u32 {
    let s = DD.lock().unwrap();
    let Some(sf) = s.surfaces.get(&this) else { return DDERR_INVALIDOBJECT };
    write_format(pf, sf.fmt);
    DD_OK
}

unsafe extern "stdcall" fn sf_get_desc(this: u32, desc: u32) -> u32 {
    let s = DD.lock().unwrap();
    let Some(sf) = s.surfaces.get(&this) else { return DDERR_INVALIDOBJECT };
    fill_desc(desc, sf.w, sf.h, sf.pitch, sf.fmt, sf.caps);
    if let Some((lo, hi)) = sf.ck_src {
        wr(desc, 4, rd(desc, 4) | DDSD_CKSRCBLT);
        wr(desc, 64, lo);
        wr(desc, 68, hi);
    }
    DD_OK
}

unsafe extern "stdcall" fn sf_lock(this: u32, r: u32, desc: u32, _flags: u32, _event: u32) -> u32 {
    if trace() {
        eprintln!("[ddraw] Lock {this:#x} {:?}", rect(r));
    }
    let mut s = DD.lock().unwrap();
    let Some(sf) = s.surfaces.get_mut(&this) else { return DDERR_INVALIDOBJECT };
    if desc == 0 {
        return DDERR_INVALIDPARAMS;
    }
    let mut ptr = sf.mem.as_mut_ptr() as u32;
    if let Some((l, t, _, _)) = rect(r) {
        let (l, t) = (l.clamp(0, sf.w - 1), t.clamp(0, sf.h - 1));
        ptr += t as u32 * sf.pitch as u32 + l as u32 * sf.fmt.bytes() as u32;
    }
    fill_desc(desc, sf.w, sf.h, sf.pitch, sf.fmt, sf.caps);
    wr(desc, 4, rd(desc, 4) | DDSD_LPSURFACE);
    wr(desc, 36, ptr);
    DD_OK
}

extern "stdcall" fn sf_unlock(this: u32, _ptr: u32) -> u32 {
    if trace() {
        eprintln!("[ddraw] Unlock {this:#x}");
    }
    let primary = match DD.lock().unwrap().surfaces.get(&this) {
        Some(sf) => sf.primary,
        None => return DDERR_NOTLOCKED,
    };
    if primary {
        present();
    }
    DD_OK
}

unsafe extern "stdcall" fn sf_get_dd_interface(_this: u32, out: *mut u32) -> u32 {
    *out = DD.lock().unwrap().dd1;
    DD_OK
}

// ---- colors ---------------------------------------------------------------------------------

/// Effective palette of an 8-bit surface: its own or, without one, the primary's.
fn palette_of(s: &State, sf: &Surface) -> [u32; 256] {
    let pal = if sf.palette != 0 { sf.palette } else { s.surfaces.get(&s.primary).map_or(0, |p| p.palette) };
    match s.palettes.get(&pal) {
        Some(p) => p.rgb,
        None => std::array::from_fn(|i| 0xFF00_0000 | (i as u32) * 0x01_0101), // grayscale
    }
}

/// Converts the whole surface to 0xFFRRGGBB.
fn to_rgb(s: &State, sf: &Surface) -> Vec<u32> {
    let mut out = Vec::with_capacity((sf.w * sf.h) as usize);
    if sf.fmt.bpp == 8 {
        let pal = palette_of(s, sf);
        for y in 0..sf.h as usize {
            let row = &sf.mem[y * sf.pitch..y * sf.pitch + sf.w as usize];
            out.extend(row.iter().map(|&i| pal[i as usize]));
        }
    } else {
        let f = sf.fmt;
        for y in 0..sf.h {
            for x in 0..sf.w {
                let v = sf.get(x, y);
                out.push(0xFF00_0000 | channel(v, f.r) << 16 | channel(v, f.g) << 8 | channel(v, f.b));
            }
        }
    }
    out
}

/// Pixel value closest to a 0xFFRRGGBB color in the surface format.
fn from_rgb(f: Format, pal: &[u32; 256], c: u32) -> u32 {
    let (r, g, b) = ((c >> 16) & 0xFF, (c >> 8) & 0xFF, c & 0xFF);
    if f.bpp == 8 {
        let dist = |p: u32| {
            let (pr, pg, pb) = ((p >> 16) & 0xFF, (p >> 8) & 0xFF, p & 0xFF);
            let (dr, dg, db) = (pr as i32 - r as i32, pg as i32 - g as i32, pb as i32 - b as i32);
            dr * dr + dg * dg + db * db
        };
        (0..256).min_by_key(|&i| dist(pal[i])).unwrap_or(0) as u32
    } else {
        unchannel(r, f.r) | unchannel(g, f.g) | unchannel(b, f.b)
    }
}

// ---- presentation -------------------------------------------------------------------------

/// Copies the primary surface to the game window.
fn present() {
    let (hwnd, w, h, px) = {
        let s = DD.lock().unwrap();
        let Some(p) = s.surfaces.get(&s.primary) else { return };
        (s.hwnd, p.w, p.h, to_rgb(&s, p))
    };
    if hwnd == 0 {
        return;
    }
    // The primary covers the whole screen; the window shows the part under its client area.
    let (ox, oy) = user32::client_screen_origin(hwnd);
    user32::with_buf(hwnd, |buf, cw, ch| {
        for y in 0..ch {
            let sy = oy + y;
            if sy < 0 || sy >= h {
                continue;
            }
            for x in 0..cw {
                let sx = ox + x;
                if sx >= 0 && sx < w {
                    buf[(y * cw + x) as usize] = px[(sy * w + sx) as usize];
                }
            }
        }
    });
    frame_done();
}

/// Counts frames. In capture mode, `WINLAYER_DDRAW_FRAMES` (default 300) frames exit the program
/// and save the PNG (a DirectDraw game is never idle).
fn frame_done() {
    let n = FRAMES.fetch_add(1, Ordering::Relaxed) + 1;
    if user32::shot_mode() {
        let max: u32 = std::env::var("WINLAYER_DDRAW_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(300);
        if n >= max {
            if let Ok(path) = std::env::var("WINLAYER_SHOT") {
                user32::dump_screenshot(&path);
            }
            super::kernel32::exit_process(0);
        }
    }
}

unsafe extern "stdcall" fn sf_flip(this: u32, _target: u32, _flags: u32) -> u32 {
    if trace() {
        eprintln!("[ddraw] Flip {this:#x}");
    }
    {
        let mut s = DD.lock().unwrap();
        // rotates the memory through the chain: front <- back1 <- back2 ... <- front
        let mut chain = vec![this];
        let mut b = s.surfaces.get(&this).map_or(0, |x| x.back);
        while b != 0 && b != this && chain.len() < 8 {
            chain.push(b);
            b = s.surfaces.get(&b).map_or(0, |x| x.back);
        }
        if chain.len() < 2 {
            return DDERR_NOTFOUND;
        }
        let mut mems: Vec<Vec<u8>> = chain.iter().map(|k| std::mem::take(&mut s.surfaces.get_mut(k).unwrap().mem)).collect();
        mems.rotate_left(1);
        for (k, m) in chain.iter().zip(mems) {
            s.surfaces.get_mut(k).unwrap().mem = m;
        }
    }
    wait_vblank();
    present();
    DD_OK
}

// ---- Blt ------------------------------------------------------------------------------------

/// Copies (with optional scaling and color key) from `src` to `dst`. Rectangles [l, t, r, b).
fn blit(s: &mut State, dst: u32, d: (i32, i32, i32, i32), src: u32, sr: (i32, i32, i32, i32), key: Option<u32>) -> u32 {
    let (dw, dh) = (d.2 - d.0, d.3 - d.1);
    let (sw, sh) = (sr.2 - sr.0, sr.3 - sr.1);
    if dw <= 0 || dh <= 0 || sw <= 0 || sh <= 0 {
        return DD_OK;
    }
    // Reads the source first (allows source == destination, e.g. scrolling).
    let Some(so) = s.surfaces.get(&src) else { return DDERR_INVALIDOBJECT };
    let sfmt = so.fmt;
    let (sw_full, sh_full) = (so.w, so.h);
    let mut srcpx = Vec::with_capacity((sw * sh) as usize);
    for y in sr.1..sr.3 {
        for x in sr.0..sr.2 {
            srcpx.push(if x >= 0 && y >= 0 && x < sw_full && y < sh_full { so.get(x, y) } else { 0 });
        }
    }
    let src_pal = if sfmt.bpp == 8 { Some(palette_of(s, so)) } else { None };
    let Some(ds) = s.surfaces.get(&dst) else { return DDERR_INVALIDOBJECT };
    let dfmt = ds.fmt;
    let convert = sfmt != dfmt;
    let dst_pal = if convert && dfmt.bpp == 8 { palette_of(s, ds) } else { [0; 256] };
    let ds = s.surfaces.get_mut(&dst).unwrap();
    let mut cache: BTreeMap<u32, u32> = BTreeMap::new();
    for y in 0..dh {
        let ty = d.1 + y;
        if ty < 0 || ty >= ds.h {
            continue;
        }
        let sy = (y * sh / dh) as usize;
        for x in 0..dw {
            let tx = d.0 + x;
            if tx < 0 || tx >= ds.w {
                continue;
            }
            let v = srcpx[sy * sw as usize + (x * sw / dw) as usize];
            if key == Some(v) {
                continue;
            }
            let v = if convert {
                *cache.entry(v).or_insert_with(|| {
                    let rgb = match &src_pal {
                        Some(p) => p[v as usize & 0xFF],
                        None => 0xFF00_0000 | channel(v, sfmt.r) << 16 | channel(v, sfmt.g) << 8 | channel(v, sfmt.b),
                    };
                    from_rgb(dfmt, &dst_pal, rgb)
                })
            } else {
                v
            };
            ds.put(tx, ty, v);
        }
    }
    DD_OK
}

/// Copies one whole surface to another, with the palette (IDirect3DTexture2::Load).
pub fn copy_surface(dst: u32, src: u32) -> u32 {
    let mut s = DD.lock().unwrap();
    let Some(so) = s.surfaces.get(&src) else { return DDERR_INVALIDOBJECT };
    let (w, h, pal) = (so.w, so.h, so.palette);
    let rc = blit(&mut s, dst, (0, 0, w, h), src, (0, 0, w, h), None);
    if let Some(d) = s.surfaces.get_mut(&dst) {
        d.palette = pal;
    }
    rc
}

fn fill(sf: &mut Surface, r: (i32, i32, i32, i32), v: u32) {
    let (l, t, rr, b) = (r.0.max(0), r.1.max(0), r.2.min(sf.w), r.3.min(sf.h));
    for y in t..b {
        for x in l..rr {
            sf.put(x, y, v);
        }
    }
}

unsafe extern "stdcall" fn sf_blt(this: u32, dr: u32, src: u32, sr: u32, flags: u32, fx: u32) -> u32 {
    let primary = {
        let mut s = DD.lock().unwrap();
        let Some(ds) = s.surfaces.get(&this) else { return DDERR_INVALIDOBJECT };
        let primary = ds.primary;
        let d = rect(dr).unwrap_or((0, 0, ds.w, ds.h));
        if trace() {
            eprintln!("[ddraw] Blt {this:#x} {d:?} <- {src:#x} {:?} flags={flags:#x}", rect(sr));
        }
        if flags & DDBLT_COLORFILL != 0 {
            let color = if fx != 0 { rd(fx, 80) } else { 0 };
            fill(s.surfaces.get_mut(&this).unwrap(), d, color);
        } else if src != 0 {
            let Some(ss) = s.surfaces.get(&src) else { return DDERR_INVALIDOBJECT };
            let r = rect(sr).unwrap_or((0, 0, ss.w, ss.h));
            let key = if flags & DDBLT_KEYSRCOVERRIDE != 0 && fx != 0 {
                Some(rd(fx, 92))
            } else if flags & DDBLT_KEYSRC != 0 {
                ss.ck_src.map(|k| k.0)
            } else {
                None
            };
            let rc = blit(&mut s, this, d, src, r, key);
            if rc != DD_OK {
                return rc;
            }
        }
        primary
    };
    if primary {
        present();
    }
    DD_OK
}

unsafe extern "stdcall" fn sf_blt_fast(this: u32, x: i32, y: i32, src: u32, sr: u32, trans: u32) -> u32 {
    let primary = {
        let mut s = DD.lock().unwrap();
        let Some(ds) = s.surfaces.get(&this) else { return DDERR_INVALIDOBJECT };
        let primary = ds.primary;
        let Some(ss) = s.surfaces.get(&src) else { return DDERR_INVALIDOBJECT };
        let r = rect(sr).unwrap_or((0, 0, ss.w, ss.h));
        let key = if trans & DDBLTFAST_SRCCOLORKEY != 0 { ss.ck_src.map(|k| k.0) } else { None };
        let d = (x, y, x + r.2 - r.0, y + r.3 - r.1);
        let rc = blit(&mut s, this, d, src, r, key);
        if rc != DD_OK {
            return rc;
        }
        primary
    };
    if primary {
        present();
    }
    DD_OK
}

// ---- GetDC / ReleaseDC --------------------------------------------------------------------

unsafe extern "stdcall" fn sf_get_dc(this: u32, out: *mut u32) -> u32 {
    if trace() {
        eprintln!("[ddraw] GetDC {this:#x}");
    }
    let (w, h, px) = {
        let s = DD.lock().unwrap();
        let Some(sf) = s.surfaces.get(&this) else { return DDERR_INVALIDOBJECT };
        if let Some((hdc, _, _)) = sf.dc {
            *out = hdc;
            return DD_OK;
        }
        (sf.w, sf.h, to_rgb(&s, sf))
    };
    let bmp = gdi32::create_bitmap(w, h, px.clone());
    let hdc = gdi32::CreateCompatibleDC(0);
    gdi32::SelectObject(hdc, bmp);
    if let Some(sf) = DD.lock().unwrap().surfaces.get_mut(&this) {
        sf.dc = Some((hdc, bmp, px));
    }
    *out = hdc;
    DD_OK
}

extern "stdcall" fn sf_release_dc(this: u32, _hdc: u32) -> u32 {
    let Some((hdc, bmp, before)) = DD.lock().unwrap().surfaces.get_mut(&this).and_then(|sf| sf.dc.take()) else { return DD_OK };
    let after = gdi32::with_dc_target(hdc, |px, _, _| px.to_vec()).unwrap_or_default();
    gdi32::DeleteDC(hdc);
    gdi32::DeleteObject(bmp);
    let primary = {
        let mut s = DD.lock().unwrap();
        let Some(sf) = s.surfaces.get(&this) else { return DD_OK };
        let pal = if sf.fmt.bpp == 8 { palette_of(&s, sf) } else { [0; 256] };
        let sf = s.surfaces.get_mut(&this).unwrap();
        let fmt = sf.fmt;
        let mut cache: BTreeMap<u32, u32> = BTreeMap::new();
        // only the pixels GDI changed come back (avoids losing precision in the rest)
        for (i, (&a, &b)) in after.iter().zip(&before).enumerate() {
            if a != b {
                let v = *cache.entry(a).or_insert_with(|| from_rgb(fmt, &pal, a));
                sf.put(i as i32 % sf.w, i as i32 / sf.w, v);
            }
        }
        sf.primary
    };
    if primary {
        present();
    }
    DD_OK
}

// ---- IDirectDrawPalette -----------------------------------------------------------------------

fn palette_vtbl() -> &'static [u32] {
    vtable![pal_qi, pal_addref, pal_release, pal_get_caps, pal_get_entries, pal_init, pal_set_entries]
}

unsafe extern "stdcall" fn pal_qi(this: u32, riid: u32, out: *mut u32) -> u32 {
    if matches!(iid(riid), 0 | 0x6C14_DB84) {
        *out = this;
        DD_OK
    } else {
        *out = 0;
        E_NOINTERFACE
    }
}

extern "stdcall" fn pal_addref(this: u32) -> u32 {
    let mut s = DD.lock().unwrap();
    s.palettes.get_mut(&this).map_or(0, |p| {
        p.refs += 1;
        p.refs
    })
}

extern "stdcall" fn pal_release(this: u32) -> u32 {
    // The palette stays stored while some surface uses it (games often release it right after SetPalette).
    let mut s = DD.lock().unwrap();
    s.palettes.get_mut(&this).map_or(0, |p| {
        p.refs = p.refs.saturating_sub(1).max(1);
        p.refs - 1
    })
}

unsafe extern "stdcall" fn pal_get_caps(_this: u32, out: *mut u32) -> u32 {
    *out = 0x4 | 0x40; // DDPCAPS_8BIT | ALLOW256
    DD_OK
}

extern "stdcall" fn pal_init(_this: u32, _dd: u32, _flags: u32, _entries: u32) -> u32 {
    DD_OK
}

unsafe extern "stdcall" fn pal_get_entries(this: u32, _flags: u32, base: u32, n: u32, out: u32) -> u32 {
    let s = DD.lock().unwrap();
    let Some(p) = s.palettes.get(&this) else { return DDERR_INVALIDOBJECT };
    for i in 0..n.min(256u32.saturating_sub(base)) {
        let c = p.rgb[(base + i) as usize];
        wr(out, i * 4, ((c >> 16) & 0xFF) | (c & 0xFF00) | ((c & 0xFF) << 16));
    }
    DD_OK
}

unsafe extern "stdcall" fn pal_set_entries(this: u32, _flags: u32, base: u32, n: u32, entries: u32) -> u32 {
    if trace() {
        eprintln!("[ddraw] SetEntries {this:#x} base={base} n={n}");
    }
    let on_primary = {
        let mut s = DD.lock().unwrap();
        let Some(p) = s.palettes.get_mut(&this) else { return DDERR_INVALIDOBJECT };
        for i in 0..n.min(256u32.saturating_sub(base)) {
            let e = rd(entries, i * 4);
            p.rgb[(base + i) as usize] = 0xFF00_0000 | ((e & 0xFF) << 16) | (e & 0xFF00) | ((e >> 16) & 0xFF);
        }
        s.surfaces.get(&s.primary).is_some_and(|p| p.palette == this && p.fmt.bpp == 8)
    };
    if on_primary {
        present();
    }
    DD_OK
}

// ---- IDirectDrawClipper -------------------------------------------------------------------

fn clipper_vtbl() -> &'static [u32] {
    vtable![cl_qi, cl_addref, cl_release, cl_get_clip_list, cl_get_hwnd, cl_init, cl_is_changed, cl_set_clip_list, cl_set_hwnd]
}

unsafe extern "stdcall" fn cl_qi(this: u32, riid: u32, out: *mut u32) -> u32 {
    if matches!(iid(riid), 0 | 0x6C14_DB85) {
        *out = this;
        DD_OK
    } else {
        *out = 0;
        E_NOINTERFACE
    }
}

extern "stdcall" fn cl_addref(_this: u32) -> u32 {
    1
}

extern "stdcall" fn cl_release(_this: u32) -> u32 {
    0
}

unsafe extern "stdcall" fn cl_get_clip_list(_this: u32, _rect: u32, _data: u32, size: *mut u32) -> u32 {
    if !size.is_null() {
        *size = 0;
    }
    DDERR_UNSUPPORTED
}

unsafe extern "stdcall" fn cl_get_hwnd(this: u32, out: *mut u32) -> u32 {
    *out = DD.lock().unwrap().clippers.get(&this).copied().unwrap_or(0);
    DD_OK
}

extern "stdcall" fn cl_init(_this: u32, _dd: u32, _flags: u32) -> u32 {
    DD_OK
}

unsafe extern "stdcall" fn cl_is_changed(_this: u32, out: *mut u32) -> u32 {
    if !out.is_null() {
        *out = 0;
    }
    DD_OK
}

extern "stdcall" fn cl_set_clip_list(_this: u32, _data: u32, _flags: u32) -> u32 {
    DD_OK
}

extern "stdcall" fn cl_set_hwnd(this: u32, _flags: u32, hwnd: u32) -> u32 {
    let mut s = DD.lock().unwrap();
    s.clippers.insert(this, hwnd);
    if s.hwnd == 0 {
        s.hwnd = hwnd;
    }
    DD_OK
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb565_channels_round_trip() {
        let f = Format::of_bpp(16);
        let v = unchannel(255, f.r) | unchannel(128, f.g) | unchannel(0, f.b);
        assert_eq!(channel(v, f.r), 255);
        assert!((channel(v, f.g) as i32 - 128).abs() <= 4);
        assert_eq!(channel(v, f.b), 0);
    }

    #[test]
    fn surface_put_get_24_bits() {
        let mut s = Surface::new(3, 2, Format::of_bpp(24), 0);
        s.put(2, 1, 0x12_3456);
        assert_eq!(s.get(2, 1), 0x12_3456);
        assert_eq!(s.pitch, 12);
    }
}
