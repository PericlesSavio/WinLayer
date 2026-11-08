//! GDI.EXE do Windows 3.x, sobre o `win32::gdi32` (handles de 16 bits, COLORREF em DWORD).

use super::api::{Call, Impl};
use super::user::{pack, rect16};
use crate::win32::gdi32;

unsafe fn rd16(p: u32) -> i16 {
    std::ptr::read_unaligned(p as usize as *const i16)
}
unsafe fn wr16(p: u32, v: u16) {
    std::ptr::write_unaligned(p as usize as *mut u16, v)
}

fn zero(_c: &mut Call) -> u32 {
    0
}

fn one(_c: &mut Call) -> u32 {
    1
}

fn select_object(c: &mut Call) -> u32 {
    gdi32::SelectObject(c.w(0) as u32, c.w(1) as u32)
}

fn delete_object(c: &mut Call) -> u32 {
    gdi32::DeleteObject(c.w(0) as u32) as u32
}

fn get_stock_object(c: &mut Call) -> u32 {
    gdi32::GetStockObject(c.w(0) as u32)
}

fn create_pen(c: &mut Call) -> u32 {
    gdi32::CreatePen(c.w(0) as u32, c.s(1) as i32, c.l(2))
}

fn create_solid_brush(c: &mut Call) -> u32 {
    gdi32::CreateSolidBrush(c.l(0))
}

fn create_pattern_brush(c: &mut Call) -> u32 {
    gdi32::CreatePatternBrush(c.w(0) as u32)
}

/// Fonte lógica a partir do nome da face e da altura pedidos: usa uma fonte bitmap carregada
/// (AddFontResource) se houver uma com esse nome; senão, a embutida (só o peso).
fn make_font(height: i32, weight: i32, face: u32) -> u32 {
    if face != 0 {
        if std::env::var_os("WINLAYER_GDI_TRACE").is_some() {
            eprintln!("[gdi] CreateFont altura={height} peso={weight} face={:?}", unsafe { std::ffi::CStr::from_ptr(face as usize as *const i8) });
        }
        let name = unsafe { std::ffi::CStr::from_ptr(face as usize as *const i8) }.to_string_lossy().into_owned();
        let idx = crate::win32::fnt::find(&name, height);
        if idx != 0 {
            return gdi32::create_bitmap_font(idx);
        }
    }
    let lf = [0u32, 0, 0, 0, weight as u32];
    unsafe { gdi32::CreateFontIndirectA(lf.as_ptr() as *const i32) }
}

fn create_font(c: &mut Call) -> u32 {
    make_font(c.s(0) as i32, c.s(4) as i32, c.ptr(13))
}

fn create_font_indirect(c: &mut Call) -> u32 {
    let p = c.ptr(0);
    let (height, weight) = unsafe { (rd16(p) as i32, rd16(p + 8) as i32) }; // LOGFONT de 16 bits: lfHeight, ..., lfWeight (5º campo), face em +18
    make_font(height, weight, p + 18)
}

fn create_compatible_dc(c: &mut Call) -> u32 {
    gdi32::CreateCompatibleDC(c.w(0) as u32)
}

fn delete_dc(c: &mut Call) -> u32 {
    gdi32::DeleteDC(c.w(0) as u32) as u32
}

fn create_bitmap(c: &mut Call) -> u32 {
    unsafe { gdi32::CreateBitmap(c.s(0) as i32, c.s(1) as i32, c.w(2) as u32, c.w(3) as u32, c.ptr(4) as usize as *const u8) }
}

fn create_dibitmap(c: &mut Call) -> u32 {
    unsafe { gdi32::CreateDIBitmap(c.w(0) as u32, c.ptr(1), c.l(2), c.ptr(3), c.ptr(4), c.w(5) as u32) }
}

fn bitblt(c: &mut Call) -> u32 {
    gdi32::BitBlt(
        c.w(0) as u32, c.s(1) as i32, c.s(2) as i32, c.s(3) as i32, c.s(4) as i32,
        c.w(5) as u32, c.s(6) as i32, c.s(7) as i32, c.l(8),
    ) as u32
}

fn set_bk_color(c: &mut Call) -> u32 {
    gdi32::SetBkColor(c.w(0) as u32, c.l(1))
}

fn set_text_color(c: &mut Call) -> u32 {
    gdi32::SetTextColor(c.w(0) as u32, c.l(1))
}

fn set_bk_mode(c: &mut Call) -> u32 {
    gdi32::SetBkMode(c.w(0) as u32, c.w(1) as u32)
}

fn get_text_color(c: &mut Call) -> u32 {
    gdi32::text_color_of(c.w(0) as u32)
}

fn get_bk_mode(c: &mut Call) -> u32 {
    gdi32::bk_mode_of(c.w(0) as u32)
}

fn get_bk_color(_c: &mut Call) -> u32 {
    0x00FF_FFFF
}

fn text_out(c: &mut Call) -> u32 {
    unsafe { gdi32::TextOutA(c.w(0) as u32, c.s(1) as i32, c.s(2) as i32, c.ptr(3) as usize as *const u8, c.s(4) as i32) as u32 }
}

fn get_text_extent(c: &mut Call) -> u32 {
    let mut sz = [0i32; 2];
    unsafe { gdi32::GetTextExtentPoint32A(c.w(0) as u32, c.ptr(1) as usize as *const u8, c.s(2) as i32, sz.as_mut_ptr()) };
    pack(sz[0] as u16, sz[1] as u16) // DWORD: LOWORD = largura, HIWORD = altura
}

fn get_text_metrics(c: &mut Call) -> u32 {
    let p = c.ptr(1);
    let mut tm = [0i32; 14];
    unsafe {
        gdi32::GetTextMetricsA(c.w(0) as u32, tm.as_mut_ptr());
        std::ptr::write_bytes(p as usize as *mut u8, 0, 31);
        // tmHeight, tmAscent, tmDescent, tmInternalLeading, tmExternalLeading, tmAveCharWidth, tmMaxCharWidth, tmWeight
        for (i, v) in [tm[0], tm[1], tm[2], tm[3], tm[4], tm[5], tm[6], 400].iter().enumerate() {
            wr16(p + i as u32 * 2, *v as u16);
        }
    }
    1
}

fn get_device_caps(c: &mut Call) -> u32 {
    gdi32::GetDeviceCaps(c.w(0) as u32, c.w(1) as u32) as u32
}

fn move_to(c: &mut Call) -> u32 {
    let mut old = [0i32; 2];
    unsafe { gdi32::MoveToEx(c.w(0) as u32, c.s(1) as i32, c.s(2) as i32, old.as_mut_ptr()) };
    pack(old[0] as u16, old[1] as u16)
}

fn line_to(c: &mut Call) -> u32 {
    gdi32::LineTo(c.w(0) as u32, c.s(1) as i32, c.s(2) as i32) as u32
}

unsafe fn points(p: u32, n: usize) -> Vec<(i32, i32)> {
    (0..n).map(|i| (rd16(p + i as u32 * 4) as i32, rd16(p + i as u32 * 4 + 2) as i32)).collect()
}

fn polygon(c: &mut Call) -> u32 {
    let pts = unsafe { points(c.ptr(1), c.s(2).max(0) as usize) };
    gdi32::polygon(c.w(0) as u32, &pts) as u32
}

fn polyline(c: &mut Call) -> u32 {
    let pts = unsafe { points(c.ptr(1), c.s(2).max(0) as usize) };
    gdi32::polyline(c.w(0) as u32, &pts) as u32
}

fn get_pixel(c: &mut Call) -> u32 {
    gdi32::GetPixel(c.w(0) as u32, c.s(1) as i32, c.s(2) as i32)
}

fn set_pixel(c: &mut Call) -> u32 {
    gdi32::SetPixel(c.w(0) as u32, c.s(1) as i32, c.s(2) as i32, c.l(3))
}

// ---- fontes (AddFontResource / EnumFontFamilies) ------------------------------------------------

static FACES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
/// Bloco global (LOGFONT16 + TEXTMETRIC16) reutilizado nas chamadas de EnumFontFamilies.
static ENUM_BLOCK: std::sync::Mutex<u16> = std::sync::Mutex::new(0);

/// Nomes de família da tabela FONTDIR (tipo 7) de um arquivo de fontes `.FON` (módulo NE).
fn font_faces(d: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let u16_at = |o: usize| d.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize);
    let Some(h) = d.get(0x3C..0x40).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize) else { return out };
    if d.get(h..h + 2) != Some(b"NE") {
        return out;
    }
    let (Some(rs), Some(restab)) = (u16_at(h + 0x24), u16_at(h + 0x26)) else { return out };
    if rs == restab {
        return out;
    }
    let base = h + rs;
    let Some(shift) = u16_at(base) else { return out };
    let mut p = base + 2;
    while let Some(tid) = u16_at(p) {
        if tid == 0 {
            break;
        }
        let count = u16_at(p + 2).unwrap_or(0);
        p += 8;
        for _ in 0..count {
            let (o, l) = (u16_at(p).unwrap_or(0) << shift, u16_at(p + 2).unwrap_or(0) << shift);
            p += 12;
            if tid & 0x7FFF != 7 {
                continue;
            }
            // FONTDIR: contagem; por fonte: ordinal(2) + FONTINFO(113) + nome do dispositivo\0 + nome da face\0
            let Some(blob) = d.get(o..o + l) else { continue };
            let n = u16::from_le_bytes([blob[0], blob[1]]) as usize;
            let mut q = 2;
            for _ in 0..n {
                let hdr = q + 2;
                // o descritor tem o tamanho dfSize no começo? usamos a varredura das duas strings após 113 bytes
                let s = hdr + 113;
                if s >= blob.len() {
                    break;
                }
                let dev_end = blob[s..].iter().position(|&c| c == 0).map_or(blob.len(), |i| s + i);
                let face_start = dev_end + 1;
                let face_end = blob[face_start.min(blob.len())..].iter().position(|&c| c == 0).map_or(blob.len(), |i| face_start + i);
                if face_start < blob.len() {
                    out.push(String::from_utf8_lossy(&blob[face_start..face_end]).into_owned());
                }
                q = face_end + 1;
            }
        }
    }
    out
}

fn add_font_resource(c: &mut Call) -> u32 {
    let p = c.ptr(0);
    if p == 0 {
        return 0;
    }
    let path = unsafe { std::ffi::CStr::from_ptr(p as usize as *const i8) }.to_string_lossy().into_owned();
    let Ok(data) = std::fs::read(crate::win32::files::host_path(&path)) else { return 0 };
    crate::win32::fnt::load_fon(&data);
    let faces = font_faces(&data);
    let n = faces.len().max(1) as u32;
    let mut all = FACES.lock().unwrap();
    for f in faces {
        if !all.iter().any(|x| x.eq_ignore_ascii_case(&f)) {
            all.push(f);
        }
    }
    n
}

/// EnumFontFamilies(hdc, família, callback far, lParam): chama o callback para cada família.
fn enum_font_families(c: &mut Call) -> u32 {
    let (fam, proc16, lparam) = (c.ptr(1), c.l(2), c.l(3));
    let wanted = if fam == 0 { None } else { Some(unsafe { std::ffi::CStr::from_ptr(fam as usize as *const i8) }.to_string_lossy().into_owned()) };
    let mut names: Vec<String> = FACES.lock().unwrap().clone();
    for stock in ["MS Sans Serif", "System", "Courier"] {
        names.push(stock.to_string());
    }
    let blk = {
        let mut g = ENUM_BLOCK.lock().unwrap();
        if *g == 0 {
            *g = super::kernel::global_alloc(128).unwrap_or(0);
        }
        *g
    };
    let Some((base, _)) = super::kernel::global_info(blk) else { return 0 };
    let mut last = 1;
    for name in names {
        if wanted.as_ref().is_some_and(|w| !w.eq_ignore_ascii_case(&name)) {
            continue;
        }
        unsafe {
            std::ptr::write_bytes(base as usize as *mut u8, 0, 128);
            // LOGFONT16 (50 bytes): altura, largura, escape, orientação, peso | itálico... | face[32]
            wr16(base, 12);
            wr16(base + 8, 400);
            let n = name.len().min(31);
            std::ptr::copy_nonoverlapping(name.as_ptr(), (base + 18) as usize as *mut u8, n);
            // TEXTMETRIC16 (31 bytes) logo depois do LOGFONT
            wr16(base + 64, 12);
            wr16(base + 66, 10);
            wr16(base + 68, 2);
            wr16(base + 74, 6);
            wr16(base + 76, 14);
            wr16(base + 78, 400);
        }
        let (lf, tm) = (blk as u32, blk as u32);
        let r = super::user::trans_call(
            proc16,
            &[lf as u16, 0, tm as u16, 64, 1, (lparam >> 16) as u16, lparam as u16], // RASTER_FONTTYPE
        );
        last = r & 0xFFFF;
        if last == 0 {
            break;
        }
    }
    last
}

fn set_bitmap_dimension(c: &mut Call) -> u32 {
    let (ox, oy) = gdi32::set_bitmap_dim(c.w(0) as u32, c.s(1) as i32, c.s(2) as i32);
    pack(ox as u16, oy as u16)
}

fn get_bitmap_dimension(c: &mut Call) -> u32 {
    let (x, y) = gdi32::get_bitmap_dim(c.w(0) as u32);
    pack(x as u16, y as u16)
}

fn set_text_align(c: &mut Call) -> u32 {
    gdi32::SetTextAlign(c.w(0) as u32, c.w(1) as u32)
}

fn create_palette(_c: &mut Call) -> u32 {
    0x8002 // handle de paleta simbólico (o desktop é truecolor, sem paleta de hardware)
}

pub static IMPLS: &[Impl] = &[
    Impl { module: "GDI", name: "SELECTOBJECT", sig: "ww", f: select_object },
    Impl { module: "GDI", name: "DELETEOBJECT", sig: "w", f: delete_object },
    Impl { module: "GDI", name: "GETSTOCKOBJECT", sig: "w", f: get_stock_object },
    Impl { module: "GDI", name: "CREATEPEN", sig: "wwl", f: create_pen },
    Impl { module: "GDI", name: "CREATESOLIDBRUSH", sig: "l", f: create_solid_brush },
    Impl { module: "GDI", name: "CREATEPATTERNBRUSH", sig: "w", f: create_pattern_brush },
    Impl { module: "GDI", name: "CREATEFONT", sig: "wwwwwwwwwwwwwp", f: create_font },
    Impl { module: "GDI", name: "CREATEFONTINDIRECT", sig: "p", f: create_font_indirect },
    Impl { module: "GDI", name: "CREATECOMPATIBLEDC", sig: "w", f: create_compatible_dc },
    Impl { module: "GDI", name: "DELETEDC", sig: "w", f: delete_dc },
    Impl { module: "GDI", name: "CREATEBITMAP", sig: "wwwwp", f: create_bitmap },
    Impl { module: "GDI", name: "CREATEDIBITMAP", sig: "wplppw", f: create_dibitmap },
    Impl { module: "GDI", name: "BITBLT", sig: "wwwwwwwwl", f: bitblt },
    Impl { module: "GDI", name: "SETBKCOLOR", sig: "wl", f: set_bk_color },
    Impl { module: "GDI", name: "SETTEXTCOLOR", sig: "wl", f: set_text_color },
    Impl { module: "GDI", name: "SETBKMODE", sig: "ww", f: set_bk_mode },
    Impl { module: "GDI", name: "GETTEXTCOLOR", sig: "w", f: get_text_color },
    Impl { module: "GDI", name: "GETBKMODE", sig: "w", f: get_bk_mode },
    Impl { module: "GDI", name: "GETBKCOLOR", sig: "w", f: get_bk_color },
    Impl { module: "GDI", name: "TEXTOUT", sig: "wwwpw", f: text_out },
    Impl { module: "GDI", name: "GETTEXTEXTENT", sig: "wpw", f: get_text_extent },
    Impl { module: "GDI", name: "GETTEXTMETRICS", sig: "wp", f: get_text_metrics },
    Impl { module: "GDI", name: "GETDEVICECAPS", sig: "ww", f: get_device_caps },
    Impl { module: "GDI", name: "MOVETO", sig: "www", f: move_to },
    Impl { module: "GDI", name: "LINETO", sig: "www", f: line_to },
    Impl { module: "GDI", name: "POLYGON", sig: "wpw", f: polygon },
    Impl { module: "GDI", name: "POLYLINE", sig: "wpw", f: polyline },
    Impl { module: "GDI", name: "GETPIXEL", sig: "www", f: get_pixel },
    Impl { module: "GDI", name: "SETPIXEL", sig: "wwwl", f: set_pixel },
    Impl { module: "GDI", name: "SETPOLYFILLMODE", sig: "ww", f: one },
    Impl { module: "GDI", name: "SETTEXTALIGN", sig: "ww", f: set_text_align },
    Impl { module: "GDI", name: "UNREALIZEOBJECT", sig: "w", f: one },
    Impl { module: "GDI", name: "ADDFONTRESOURCE", sig: "p", f: add_font_resource },
    Impl { module: "GDI", name: "REMOVEFONTRESOURCE", sig: "p", f: one },
    Impl { module: "GDI", name: "ENUMFONTFAMILIES", sig: "wpll", f: enum_font_families },
    Impl { module: "GDI", name: "GETBITMAPBITS", sig: "wlp", f: zero },
    Impl { module: "GDI", name: "SETBITMAPBITS", sig: "wlp", f: zero },
    Impl { module: "GDI", name: "GETBITMAPDIMENSION", sig: "w", f: get_bitmap_dimension },
    Impl { module: "GDI", name: "SETBITMAPDIMENSION", sig: "www", f: set_bitmap_dimension },
    Impl { module: "GDI", name: "CREATEPALETTE", sig: "p", f: create_palette },
    Impl { module: "GDI", name: "ANIMATEPALETTE", sig: "wwwp", f: zero },
    Impl { module: "GDI", name: "SETPALETTEENTRIES", sig: "wwwp", f: zero },
    Impl { module: "GDI", name: "GETSYSTEMPALETTEENTRIES", sig: "wwwp", f: zero },
];

#[allow(dead_code)]
fn _use(_: u32) {
    let _ = rect16;
}
