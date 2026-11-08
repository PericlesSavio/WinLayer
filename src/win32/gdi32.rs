//! gdi32.dll — desenho em software. Um DC desenha no buffer de uma janela ou, se tiver
//! um bitmap selecionado (DC de memória), nos pixels desse bitmap.

use std::collections::BTreeMap;
use std::sync::Mutex;

#[derive(Clone)]
enum Obj {
    /// `None` = brush nulo (não preenche).
    Brush(Option<u32>),
    /// `None` = pen nulo (sem contorno).
    Pen(Option<u32>),
    Font { bold: bool, fnt: u32 },
    /// Pixels 0xFFRRGGBB, de cima para baixo.
    Bitmap { w: i32, h: i32, px: Vec<u32> },
}

#[derive(Clone)]
struct Dc {
    hwnd: u32,
    /// Bitmap selecionado (DC de memória); 0 = desenha na janela `hwnd`.
    bitmap: u32,
    brush: u32,
    pen: u32,
    font: u32,
    text_color: u32,
    bk_color: u32,
    transparent: bool,
    pos: (i32, i32),
    /// SetTextAlign: TA_UPDATECP=1, TA_RIGHT=2, TA_CENTER=6, TA_BOTTOM=8, TA_BASELINE=24.
    align: u32,
}

impl Dc {
    fn new(hwnd: u32, bitmap: u32) -> Dc {
        Dc {
            hwnd,
            bitmap,
            brush: STOCK_BASE,      // WHITE_BRUSH
            pen: STOCK_BASE + 7,    // BLACK_PEN
            font: STOCK_BASE + 17,  // DEFAULT_GUI_FONT
            text_color: 0,
            bk_color: 0xFFFFFF,
            transparent: false,
            pos: (0, 0),
            align: 0,
        }
    }
}

struct Gdi {
    objs: BTreeMap<u32, Obj>,
    dcs: BTreeMap<u32, Dc>,
    saved: BTreeMap<u32, Vec<Dc>>,
    next: u32,
}

static GDI: Mutex<Gdi> = Mutex::new(Gdi { objs: BTreeMap::new(), dcs: BTreeMap::new(), saved: BTreeMap::new(), next: 0x1000 });

pub fn resolve(name: &str) -> Option<usize> {
    exports!(
        name,
        GetStockObject, CreateSolidBrush, CreatePen, CreateFontIndirectA, SelectObject,
        DeleteObject, Rectangle, MoveToEx, LineTo, SetTextColor, SetBkColor, SetBkMode,
        TextOutA, GetTextExtentPoint32A, GetTextMetricsA, CreateCompatibleDC, CreateCompatibleBitmap,
        CreateBitmap, DeleteDC, BitBlt, StretchBlt, PatBlt, GetPixel, SetPixel, SetPixelV,
        GetDeviceCaps, GetObjectA, SetMapMode, Ellipse, RoundRect, CreateHatchBrush, GetClipBox, CreateDCA, CreateICA,
        CreateBrushIndirect, GetTextFaceA, CreatePenIndirect, CreatePatternBrush, GetBkColor, StretchDIBits, CreateDIBitmap, SaveDC, RestoreDC, SetViewportOrgEx, SetViewportExtEx,
        SetWindowExtEx, ScaleViewportExtEx, ScaleWindowExtEx, OffsetViewportOrgEx, PtVisible,
        ExtTextOutA, GetTextExtentPointA, SetTextAlign, GetTextAlign, CreateFontA, GetCurrentPositionEx,
        GetTextExtentExPointA, SetTextCharacterExtra,
        CreatePalette, SelectPalette, RealizePalette, GetPaletteEntries, SetPaletteEntries, ResizePalette,
    )
}

/// COLORREF (0x00BBGGRR) -> pixel 0xFFRRGGBB.
pub fn colorref_to_px(c: u32) -> u32 {
    0xFF00_0000 | ((c & 0xFF) << 16) | (c & 0xFF00) | ((c >> 16) & 0xFF)
}

fn px_to_colorref(p: u32) -> u32 {
    ((p & 0xFF) << 16) | (p & 0xFF00) | ((p >> 16) & 0xFF)
}

/// Cores do sistema (COLOR_*), em COLORREF.
pub static DESKTOP_COLOR: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x808000);

pub fn sys_color(i: u32) -> u32 {
    match i {
        1 => DESKTOP_COLOR.load(std::sync::atomic::Ordering::Relaxed), // DESKTOP (padrão: azul-esverdeado)
        2 => 0x800000,        // ACTIVECAPTION
        5 | 8 => 0xFFFFFF,    // WINDOW
        6 => 0x000000,        // WINDOWFRAME
        7 => 0x000000,        // MENUTEXT
        15 => 0xC0C0C0,       // BTNFACE
        _ => 0xC0C0C0,
    }
}

fn stock(i: u32) -> Option<Obj> {
    Some(match i {
        0 => Obj::Brush(Some(0xFFFFFF)),
        1 => Obj::Brush(Some(0xC0C0C0)),
        2 => Obj::Brush(Some(0x808080)),
        3 => Obj::Brush(Some(0x404040)),
        4 => Obj::Brush(Some(0x000000)),
        5 => Obj::Brush(None),
        6 => Obj::Pen(Some(0xFFFFFF)),
        7 => Obj::Pen(Some(0x000000)),
        8 => Obj::Pen(None),
        13 => Obj::Font { bold: true, fnt: 0 }, // SYSTEM_FONT
        10..=17 => Obj::Font { bold: false, fnt: 0 },
        _ => return None,
    })
}

const STOCK_BASE: u32 = 0x0F00;

fn lookup(g: &Gdi, h: u32) -> Option<Obj> {
    if (STOCK_BASE..STOCK_BASE + 32).contains(&h) {
        stock(h - STOCK_BASE)
    } else {
        g.objs.get(&h).cloned()
    }
}

fn add_locked(g: &mut Gdi, obj: Obj) -> u32 {
    let h = g.next;
    g.next += 4;
    g.objs.insert(h, obj);
    h
}

fn add(obj: Obj) -> u32 {
    add_locked(&mut GDI.lock().unwrap(), obj)
}

/// Cor (COLORREF) de um brush sólido; `None` se for nulo ou inválido.
pub fn brush_color(h: u32) -> Option<u32> {
    match lookup(&GDI.lock().unwrap(), h) {
        Some(Obj::Brush(c)) => c,
        _ => None,
    }
}

/// Dimensão de bitmap (SetBitmapDimension/GetBitmapDimension): só um valor guardado por handle.
static BMP_DIM: Mutex<BTreeMap<u32, (i32, i32)>> = Mutex::new(BTreeMap::new());

pub fn set_bitmap_dim(h: u32, x: i32, y: i32) -> (i32, i32) {
    BMP_DIM.lock().unwrap().insert(h, (x, y)).unwrap_or((0, 0))
}

pub fn get_bitmap_dim(h: u32) -> (i32, i32) {
    BMP_DIM.lock().unwrap().get(&h).copied().unwrap_or((0, 0))
}

// ---- DCs ----------------------------------------------------------------------------------

pub fn dc_for_window(hwnd: u32) -> u32 {
    let mut g = GDI.lock().unwrap();
    let h = g.next;
    g.next += 4;
    g.dcs.insert(h, Dc::new(hwnd, 0));
    h
}

/// Janela de um DC (0 = DC de memória ou desconhecido).
pub fn dc_hwnd(h: u32) -> u32 {
    GDI.lock().unwrap().dcs.get(&h).map_or(0, |d| d.hwnd)
}

pub fn release_dc(h: u32) {
    GDI.lock().unwrap().dcs.remove(&h);
}

/// Executa `f` sobre os pixels de destino do DC (janela ou bitmap). `f` não pode chamar o GDI.
pub fn with_dc_target<R>(hdc: u32, f: impl FnOnce(&mut [u32], i32, i32) -> R) -> Option<R> {
    let (hwnd, bmp) = {
        let g = GDI.lock().unwrap();
        let d = g.dcs.get(&hdc)?;
        (d.hwnd, d.bitmap)
    };
    if bmp != 0 {
        let mut g = GDI.lock().unwrap();
        return match g.objs.get_mut(&bmp) {
            Some(Obj::Bitmap { w, h, px }) => {
                let (w, h) = (*w, *h);
                Some(f(px, w, h))
            }
            _ => None,
        };
    }
    super::user32::with_buf(hwnd, f)
}

/// Copia a região (sx, sy, w, h) do DC; pixels fora do alvo viram preto.
fn read_region(hdc: u32, sx: i32, sy: i32, w: i32, h: i32) -> Option<Vec<u32>> {
    with_dc_target(hdc, |px, bw, bh| {
        let mut out = Vec::with_capacity((w.max(0) * h.max(0)) as usize);
        for y in 0..h {
            for x in 0..w {
                let (tx, ty) = (sx + x, sy + y);
                out.push(if tx >= 0 && ty >= 0 && tx < bw && ty < bh { px[(ty * bw + tx) as usize] } else { 0xFF00_0000 });
            }
        }
        out
    })
}

// ---- decodificação de DIB (usada por LoadBitmap e afins) ----------------------------------

/// Decodifica um DIB (BITMAPINFOHEADER + paleta + pixels) para (w, h, pixels top-down).
pub unsafe fn decode_dib(hdr: u32, bits_at: Option<u32>) -> Option<(i32, i32, Vec<u32>)> {
    // O DIB pode estar em endereço não alinhado (ex.: dentro de um bloco de recurso): leitura unaligned.
    let rd32 = |o: u32| std::ptr::read_unaligned((hdr + o) as usize as *const u32);
    let rd16 = |o: u32| std::ptr::read_unaligned((hdr + o) as usize as *const u16) as u32;
    let size = rd32(0);
    if size < 12 {
        return None;
    }
    let (w, h_raw, bpp, comp) = (rd32(4) as i32, rd32(8) as i32, rd16(14), rd32(16));
    if comp != 0 && comp != 3 {
        return None; // RLE ainda não
    }
    let h = h_raw.abs();
    let top_down = h_raw < 0;
    let ncolors = if bpp <= 8 { match rd32(32) { 0 => 1 << bpp, n => n } } else { 0 };
    let pal = hdr + size + if comp == 3 && size == 40 { 12 } else { 0 };
    let bits = bits_at.unwrap_or(pal + ncolors * 4);
    let stride = ((w as u32 * bpp + 31) / 32) * 4;
    let mut px = vec![0xFF00_0000u32; (w * h) as usize];
    for y in 0..h {
        let src_row = if top_down { y } else { h - 1 - y };
        let row = bits + src_row as u32 * stride;
        for x in 0..w as u32 {
            let b = |o: u32| *((row + o) as usize as *const u8) as u32;
            let idx_color = |i: u32| {
                let e = pal + i.min(ncolors.saturating_sub(1)) * 4;
                let c = |o| *((e + o) as usize as *const u8) as u32;
                0xFF00_0000 | (c(2) << 16) | (c(1) << 8) | c(0)
            };
            px[(y * w + x as i32) as usize] = match bpp {
                1 => idx_color((b(x / 8) >> (7 - x % 8)) & 1),
                4 => idx_color(if x % 2 == 0 { b(x / 2) >> 4 } else { b(x / 2) & 0xF }),
                8 => idx_color(b(x)),
                16 => {
                    let v = b(x * 2) | (b(x * 2 + 1) << 8);
                    let (r, g, bl) = ((v >> 10) & 31, (v >> 5) & 31, v & 31);
                    0xFF00_0000 | ((r * 255 / 31) << 16) | ((g * 255 / 31) << 8) | (bl * 255 / 31)
                }
                24 => 0xFF00_0000 | (b(x * 3 + 2) << 16) | (b(x * 3 + 1) << 8) | b(x * 3),
                32 => 0xFF00_0000 | (b(x * 4 + 2) << 16) | (b(x * 4 + 1) << 8) | b(x * 4),
                _ => return None,
            };
        }
    }
    Some((w, h, px))
}

/// Cria um bitmap a partir de pixels já decodificados; devolve o HBITMAP.
pub fn create_bitmap(w: i32, h: i32, px: Vec<u32>) -> u32 {
    add(Obj::Bitmap { w, h, px })
}

// ---- objetos ------------------------------------------------------------------------------

#[allow(non_snake_case)]
pub extern "stdcall" fn GetStockObject(i: u32) -> u32 {
    if stock(i).is_some() { STOCK_BASE + i } else { 0 }
}

#[allow(non_snake_case)]
pub extern "stdcall" fn CreateSolidBrush(color: u32) -> u32 {
    add(Obj::Brush(Some(color & 0xFFFFFF)))
}

#[allow(non_snake_case)]
pub extern "stdcall" fn CreatePen(style: u32, _width: i32, color: u32) -> u32 {
    add(Obj::Pen(if style == 5 { None } else { Some(color & 0xFFFFFF) })) // PS_NULL = 5
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn CreateFontIndirectA(logfont: *const i32) -> u32 {
    // LOGFONT: lfHeight, lfWidth, lfEscapement, lfOrientation, lfWeight (offset 16)
    add(Obj::Font { bold: *logfont.add(4) >= 600, fnt: 0 })
}

#[allow(non_snake_case)]
pub extern "stdcall" fn CreateCompatibleBitmap(_hdc: u32, w: i32, h: i32) -> u32 {
    let (w, h) = (w.max(1), h.max(1));
    add(Obj::Bitmap { w, h, px: vec![0xFF00_0000; (w * h) as usize] })
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn CreateBitmap(w: i32, h: i32, _planes: u32, bpp: u32, bits: *const u8) -> u32 {
    let (w, h) = (w.max(1), h.max(1));
    let mut px = vec![0xFF00_0000u32; (w * h) as usize];
    if bpp == 1 && !bits.is_null() {
        let stride = ((w + 15) / 16 * 2) as usize; // linhas alinhadas em 16 bits
        for y in 0..h as usize {
            for x in 0..w as usize {
                if *bits.add(y * stride + x / 8) & (0x80 >> (x % 8)) != 0 {
                    px[y * w as usize + x] = 0xFFFF_FFFF;
                }
            }
        }
    }
    add(Obj::Bitmap { w, h, px })
}

#[allow(non_snake_case)]
pub extern "stdcall" fn CreateCompatibleDC(_hdc: u32) -> u32 {
    let mut g = GDI.lock().unwrap();
    let bmp = add_locked(&mut g, Obj::Bitmap { w: 1, h: 1, px: vec![0xFF00_0000] }); // bitmap padrão 1x1
    let h = g.next;
    g.next += 4;
    g.dcs.insert(h, Dc::new(0, bmp));
    h
}

#[allow(non_snake_case)]
pub extern "stdcall" fn DeleteDC(hdc: u32) -> i32 {
    GDI.lock().unwrap().dcs.remove(&hdc).is_some() as i32
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SelectObject(hdc: u32, h: u32) -> u32 {
    let mut g = GDI.lock().unwrap();
    let Some(obj) = lookup(&g, h) else { return 0 };
    let Some(dc) = g.dcs.get_mut(&hdc) else { return 0 };
    let slot = match obj {
        Obj::Brush(_) => &mut dc.brush,
        Obj::Pen(_) => &mut dc.pen,
        Obj::Font { .. } => &mut dc.font,
        Obj::Bitmap { .. } => &mut dc.bitmap,
    };
    std::mem::replace(slot, h)
}

#[allow(non_snake_case)]
pub extern "stdcall" fn DeleteObject(h: u32) -> i32 {
    GDI.lock().unwrap().objs.remove(&h).is_some() as i32
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn GetObjectA(h: u32, cb: i32, out: *mut u32) -> i32 {
    match lookup(&GDI.lock().unwrap(), h) {
        // BITMAP: bmType, bmWidth, bmHeight, bmWidthBytes, bmPlanes(u16), bmBitsPixel(u16), bmBits
        Some(Obj::Bitmap { w, h, .. }) if cb >= 24 => {
            *out = 0;
            *out.add(1) = w as u32;
            *out.add(2) = h as u32;
            *out.add(3) = (w * 4) as u32;
            *out.add(4) = 1 | (32 << 16);
            *out.add(5) = 0;
            24
        }
        // LOGBRUSH: lbStyle, lbColor, lbHatch
        Some(Obj::Brush(c)) if cb >= 12 => {
            *out = c.is_none() as u32;
            *out.add(1) = c.unwrap_or(0);
            *out.add(2) = 0;
            12
        }
        _ => 0,
    }
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetDeviceCaps(_hdc: u32, idx: u32) -> i32 {
    match idx {
        8 => super::user32::desktop_w() as i32,   // HORZRES
        10 => super::user32::desktop_h() as i32,  // VERTRES
        12 => 32,   // BITSPIXEL
        14 => 1,    // PLANES
        24 => -1,   // NUMCOLORS (> 8 bpp)
        88 | 90 => 96, // LOGPIXELSX / Y
        _ => 0,
    }
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SetMapMode(_hdc: u32, _mode: u32) -> u32 {
    1 // MM_TEXT
}

// ---- estado do DC -------------------------------------------------------------------------

#[allow(non_snake_case)]
pub extern "stdcall" fn SetTextColor(hdc: u32, c: u32) -> u32 {
    let mut g = GDI.lock().unwrap();
    g.dcs.get_mut(&hdc).map_or(u32::MAX, |d| std::mem::replace(&mut d.text_color, c & 0xFFFFFF))
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SetBkColor(hdc: u32, c: u32) -> u32 {
    let mut g = GDI.lock().unwrap();
    g.dcs.get_mut(&hdc).map_or(u32::MAX, |d| std::mem::replace(&mut d.bk_color, c & 0xFFFFFF))
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SetBkMode(hdc: u32, mode: u32) -> u32 {
    let mut g = GDI.lock().unwrap();
    g.dcs.get_mut(&hdc).map_or(0, |d| {
        let old = if d.transparent { 1 } else { 2 };
        d.transparent = mode == 1; // TRANSPARENT
        old
    })
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn MoveToEx(hdc: u32, x: i32, y: i32, old: *mut i32) -> i32 {
    let mut g = GDI.lock().unwrap();
    let Some(dc) = g.dcs.get_mut(&hdc) else { return 0 };
    if !old.is_null() {
        *old = dc.pos.0;
        *old.add(1) = dc.pos.1;
    }
    dc.pos = (x, y);
    1
}

/// Cores (brush, pen) do DC.
fn style_of(hdc: u32) -> Option<(Option<u32>, Option<u32>)> {
    let g = GDI.lock().unwrap();
    let dc = g.dcs.get(&hdc)?;
    let brush = match lookup(&g, dc.brush) { Some(Obj::Brush(c)) => c, _ => None };
    let pen = match lookup(&g, dc.pen) { Some(Obj::Pen(c)) => c, _ => None };
    Some((brush, pen))
}

/// Fonte bitmap carregada (de um `.FON`) criando uma fonte lógica; `fnt` = índice+1 de `fnt::find`.
pub fn create_bitmap_font(fnt: u32) -> u32 {
    add(Obj::Font { bold: false, fnt })
}

/// Índice da fonte bitmap carregada selecionada no DC (0 = fonte embutida).
pub fn font_idx(hdc: u32) -> u32 {
    let g = GDI.lock().unwrap();
    match g.dcs.get(&hdc).and_then(|d| lookup(&g, d.font)) {
        Some(Obj::Font { fnt, .. }) => fnt,
        _ => 0,
    }
}

/// (cor do texto, cor de fundo ou None se transparente, negrito) do DC.
pub fn text_style(hdc: u32) -> Option<(u32, Option<u32>, bool)> {
    let g = GDI.lock().unwrap();
    let dc = g.dcs.get(&hdc)?;
    let bold = matches!(lookup(&g, dc.font), Some(Obj::Font { bold: true, .. }));
    Some((dc.text_color, (!dc.transparent).then_some(dc.bk_color), bold))
}

// ---- desenho ------------------------------------------------------------------------------

#[allow(non_snake_case)]
pub extern "stdcall" fn Rectangle(hdc: u32, l: i32, t: i32, r: i32, b: i32) -> i32 {
    let Some((brush, pen)) = style_of(hdc) else { return 0 };
    with_dc_target(hdc, |px, w, h| {
        let put = |px: &mut [u32], x: i32, y: i32, c: u32| {
            if x >= 0 && y >= 0 && x < w && y < h {
                px[(y * w + x) as usize] = colorref_to_px(c);
            }
        };
        if let Some(c) = brush {
            for y in t + 1..b - 1 {
                for x in l + 1..r - 1 {
                    put(px, x, y, c);
                }
            }
        }
        if let Some(c) = pen {
            for x in l..r {
                put(px, x, t, c);
                put(px, x, b - 1, c);
            }
            for y in t..b {
                put(px, l, y, c);
                put(px, r - 1, y, c);
            }
        }
    });
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn LineTo(hdc: u32, x1: i32, y1: i32) -> i32 {
    let Some((_, pen)) = style_of(hdc) else { return 0 };
    let from = {
        let mut g = GDI.lock().unwrap();
        let dc = g.dcs.get_mut(&hdc).unwrap();
        std::mem::replace(&mut dc.pos, (x1, y1))
    };
    let Some(c) = pen else { return 1 };
    with_dc_target(hdc, |px, w, h| {
        // Bresenham
        let (mut x, mut y) = from;
        let (dx, dy) = ((x1 - x).abs(), -(y1 - y).abs());
        let (sx, sy) = (if x < x1 { 1 } else { -1 }, if y < y1 { 1 } else { -1 });
        let mut err = dx + dy;
        loop {
            if x >= 0 && y >= 0 && x < w && y < h {
                px[(y * w + x) as usize] = colorref_to_px(c);
            }
            if x == x1 && y == y1 { break }
            let e2 = 2 * err;
            if e2 >= dy { err += dy; x += sx }
            if e2 <= dx { err += dx; y += sy }
        }
    });
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SetTextAlign(hdc: u32, v: u32) -> u32 {
    GDI.lock().unwrap().dcs.get_mut(&hdc).map_or(u32::MAX, |d| std::mem::replace(&mut d.align, v))
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetTextAlign(hdc: u32) -> u32 {
    GDI.lock().unwrap().dcs.get(&hdc).map_or(u32::MAX, |d| d.align)
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn TextOutA(hdc: u32, x: i32, y: i32, s: *const u8, n: i32) -> i32 {
    let Some((fg, bk, bold)) = text_style(hdc) else { return 0 };
    let text = std::slice::from_raw_parts(s, n.max(0) as usize);
    let (mut x, mut y) = (x, y);
    let align = GetTextAlign(hdc);
    let font = super::fnt::get(font_idx(hdc));
    let (tw, lh, asc) = match &font {
        Some(f) => (f.width(text), f.height, f.ascent),
        None => (super::text::width(bold, text), super::text::line_h(), super::text::ascent()),
    };
    if align & 1 != 0 {
        // TA_UPDATECP: ignora x/y, usa a posição atual e a avança pela largura do texto.
        let mut g = GDI.lock().unwrap();
        if let Some(d) = g.dcs.get_mut(&hdc) {
            (x, y) = d.pos;
            d.pos.0 += tw;
        }
    }
    match align & 6 {
        6 => x -= tw / 2,
        2 => x -= tw,
        _ => {}
    }
    if align & 24 == 24 {
        y -= asc;
    } else if align & 8 != 0 {
        y -= lh;
    }
    if std::env::var_os("WINLAYER_GDI_TRACE").is_some() {
        let d = with_dc_target(hdc, |_, w, h| (w, h));
        eprintln!("[gdi] text dc={hdc:x}{d:?} ({x},{y}) {:?} fg={fg:x}", String::from_utf8_lossy(text));
    }
    with_dc_target(hdc, |px, w, h| {
        let mut b = super::draw::Buf { px, w, h };
        if let Some(c) = bk {
            b.fill(x, y, x + tw, y + lh, colorref_to_px(c));
        }
        match &font {
            Some(f) => {
                f.draw(b.px, b.w, b.h, x, y, text, colorref_to_px(fg));
            }
            None => {
                b.text(x, y, text, colorref_to_px(fg), bold);
            }
        }
    });
    1
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn GetTextExtentPoint32A(hdc: u32, s: *const u8, n: i32, size: *mut i32) -> i32 {
    let bold = text_style(hdc).is_some_and(|t| t.2);
    let text = std::slice::from_raw_parts(s, n.max(0) as usize);
    match super::fnt::get(font_idx(hdc)) {
        Some(f) => {
            *size = f.width(text);
            *size.add(1) = f.height;
        }
        None => {
            *size = super::text::width(bold, text);
            *size.add(1) = super::text::line_h();
        }
    }
    1
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn GetTextMetricsA(hdc: u32, tm: *mut i32) -> i32 {
    // TEXTMETRIC: tmHeight, tmAscent, tmDescent, tmInternalLeading, tmExternalLeading,
    //             tmAveCharWidth, tmMaxCharWidth, ...
    std::ptr::write_bytes(tm as *mut u8, 0, 56);
    if let Some(f) = super::fnt::get(font_idx(hdc)) {
        *tm = f.height;
        *tm.add(1) = f.ascent;
        *tm.add(2) = f.height - f.ascent;
        *tm.add(3) = f.internal_leading;
        *tm.add(4) = f.external_leading;
        *tm.add(5) = f.avg_width;
        *tm.add(6) = f.max_width;
        return 1;
    }
    *tm = super::text::line_h();
    *tm.add(1) = super::text::ascent();
    *tm.add(2) = super::text::line_h() - super::text::ascent();
    let w16 = super::text::line_h() == 16;
    *tm.add(5) = if w16 { 8 } else { 6 };
    *tm.add(6) = if w16 { 16 } else { 14 };
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetPixel(hdc: u32, x: i32, y: i32) -> u32 {
    with_dc_target(hdc, |px, w, h| {
        if x >= 0 && y >= 0 && x < w && y < h { px_to_colorref(px[(y * w + x) as usize]) } else { u32::MAX }
    })
    .unwrap_or(u32::MAX)
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SetPixel(hdc: u32, x: i32, y: i32, c: u32) -> u32 {
    with_dc_target(hdc, |px, w, h| {
        if x >= 0 && y >= 0 && x < w && y < h {
            px[(y * w + x) as usize] = colorref_to_px(c);
        }
    });
    c
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SetPixelV(hdc: u32, x: i32, y: i32, c: u32) -> i32 {
    SetPixel(hdc, x, y, c);
    1
}

// ---- blits --------------------------------------------------------------------------------

const SRCCOPY: u32 = 0x00CC_0020;
const NOTSRCCOPY: u32 = 0x0033_0008;
const SRCPAINT: u32 = 0x00EE_0086;
const SRCAND: u32 = 0x0088_00C6;
const SRCINVERT: u32 = 0x0066_0046;
const PATCOPY: u32 = 0x00F0_0021;
const PATINVERT: u32 = 0x005A_0049;
const DSTINVERT: u32 = 0x0055_0009;
const BLACKNESS: u32 = 0x0000_0042;
const WHITENESS: u32 = 0x00FF_0062;

fn needs_source(rop: u32) -> bool {
    !matches!(rop, PATCOPY | PATINVERT | DSTINVERT | BLACKNESS | WHITENESS)
}

fn apply_rop(rop: u32, src: u32, dst: u32, pat: u32) -> u32 {
    const A: u32 = 0xFF00_0000;
    match rop {
        SRCCOPY => src,
        NOTSRCCOPY => !src | A,
        SRCPAINT => src | dst,
        SRCAND => src & dst,
        SRCINVERT => (src ^ dst) | A,
        PATCOPY => pat,
        PATINVERT => (pat ^ dst) | A,
        DSTINVERT => !dst | A,
        BLACKNESS => A,
        WHITENESS => 0xFFFF_FFFF,
        _ => src,
    }
}

fn pattern_px(hdc: u32) -> u32 {
    style_of(hdc).and_then(|s| s.0).map_or(0xFF00_0000, colorref_to_px)
}

#[allow(non_snake_case)]
pub extern "stdcall" fn BitBlt(dst: u32, dx: i32, dy: i32, w: i32, h: i32, src: u32, sx: i32, sy: i32, rop: u32) -> i32 {
    StretchBlt(dst, dx, dy, w, h, src, sx, sy, w, h, rop)
}

#[allow(non_snake_case)]
pub extern "stdcall" fn StretchBlt(dst: u32, dx: i32, dy: i32, dw: i32, dh: i32, src: u32, sx: i32, sy: i32, sw: i32, sh: i32, rop: u32) -> i32 {
    if std::env::var_os("WINLAYER_GDI_TRACE").is_some() {
        let d = with_dc_target(dst, |_, w, h| (w, h));
        let s = with_dc_target(src, |p, w, h| (w, h, p.iter().filter(|&&c| c & 0xFFFFFF != 0).count()));
        eprintln!("[gdi] blit dst={dst:x}{d:?} ({dx},{dy},{dw},{dh}) src={src:x}{s:?} ({sx},{sy},{sw},{sh}) rop={rop:x}");
    }
    let source = if needs_source(rop) {
        match read_region(src, sx, sy, sw, sh) {
            Some(v) if sw > 0 && sh > 0 => Some(v),
            _ => return 0,
        }
    } else {
        None
    };
    blit_pixels(dst, dx, dy, dw, dh, source.as_deref(), sw, sh, rop)
}

/// Desenha `source` (sw x sh) escalado em (dx, dy, dw, dh) do DC, aplicando a ROP.
#[allow(clippy::too_many_arguments)]
fn blit_pixels(dst: u32, dx: i32, dy: i32, dw: i32, dh: i32, source: Option<&[u32]>, sw: i32, sh: i32, rop: u32) -> i32 {
    if dw <= 0 || dh <= 0 {
        return 0;
    }
    let pat = pattern_px(dst);
    with_dc_target(dst, |px, bw, bh| {
        for y in 0..dh {
            for x in 0..dw {
                let (tx, ty) = (dx + x, dy + y);
                if tx < 0 || ty < 0 || tx >= bw || ty >= bh {
                    continue;
                }
                let s = source.map_or(0, |v| v[((y * sh / dh) * sw + x * sw / dw) as usize]);
                let i = (ty * bw + tx) as usize;
                px[i] = apply_rop(rop, s, px[i], pat);
            }
        }
    })
    .is_some() as i32
}

#[allow(non_snake_case)]
pub extern "stdcall" fn PatBlt(hdc: u32, x: i32, y: i32, w: i32, h: i32, rop: u32) -> i32 {
    if needs_source(rop) {
        return 0;
    }
    StretchBlt(hdc, x, y, w, h, 0, 0, 0, 0, 0, rop)
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SaveDC(hdc: u32) -> i32 {
    let mut g = GDI.lock().unwrap();
    let Some(dc) = g.dcs.get(&hdc).cloned() else { return 0 };
    let stack = g.saved.entry(hdc).or_default();
    stack.push(dc);
    stack.len() as i32
}

#[allow(non_snake_case)]
pub extern "stdcall" fn RestoreDC(hdc: u32, n: i32) -> i32 {
    let mut g = GDI.lock().unwrap();
    let len = g.saved.get(&hdc).map_or(0, |s| s.len() as i32);
    let target = if n < 0 { len + n } else { n - 1 };
    if target < 0 || target >= len {
        return 0;
    }
    let stack = g.saved.get_mut(&hdc).unwrap();
    let dc = stack[target as usize].clone();
    stack.truncate(target as usize);
    g.dcs.insert(hdc, dc);
    1
}

// Transformações de mapa/viewport: ainda não implementadas (mapeamento 1:1).
#[allow(non_snake_case)]
pub extern "stdcall" fn SetViewportOrgEx(_hdc: u32, _x: i32, _y: i32, _old: u32) -> i32 { 1 }
#[allow(non_snake_case)]
pub extern "stdcall" fn SetViewportExtEx(_hdc: u32, _x: i32, _y: i32, _old: u32) -> i32 { 1 }
#[allow(non_snake_case)]
pub extern "stdcall" fn SetWindowExtEx(_hdc: u32, _x: i32, _y: i32, _old: u32) -> i32 { 1 }
#[allow(non_snake_case)]
pub extern "stdcall" fn ScaleViewportExtEx(_hdc: u32, _a: i32, _b: i32, _c: i32, _d: i32, _old: u32) -> i32 { 1 }
#[allow(non_snake_case)]
pub extern "stdcall" fn ScaleWindowExtEx(_hdc: u32, _a: i32, _b: i32, _c: i32, _d: i32, _old: u32) -> i32 { 1 }
#[allow(non_snake_case)]
pub extern "stdcall" fn OffsetViewportOrgEx(_hdc: u32, _x: i32, _y: i32, _old: u32) -> i32 { 1 }
#[allow(non_snake_case)]
pub extern "stdcall" fn PtVisible(_hdc: u32, _x: i32, _y: i32) -> i32 { 1 }

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn ExtTextOutA(hdc: u32, x: i32, y: i32, _opts: u32, _rect: u32, s: *const u8, n: i32, _dx: u32) -> i32 {
    TextOutA(hdc, x, y, s, n)
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn GetTextExtentPointA(hdc: u32, s: *const u8, n: i32, size: *mut i32) -> i32 {
    GetTextExtentPoint32A(hdc, s, n, size)
}

#[allow(non_snake_case)]
pub extern "stdcall" fn Ellipse(hdc: u32, l: i32, t: i32, r: i32, b: i32) -> i32 {
    let Some((brush, pen)) = style_of(hdc) else { return 0 };
    let (cx, cy) = ((l + r) as f32 / 2.0, (t + b) as f32 / 2.0);
    let (rx, ry) = (((r - l) as f32 / 2.0).max(0.5), ((b - t) as f32 / 2.0).max(0.5));
    with_dc_target(hdc, |px, w, h| {
        for y in t..b {
            for x in l..r {
                if x < 0 || y < 0 || x >= w || y >= h {
                    continue;
                }
                let d = ((x as f32 + 0.5 - cx) / rx).powi(2) + ((y as f32 + 0.5 - cy) / ry).powi(2);
                // contorno: faixa de ~1 px na borda; interior: preenchimento
                let edge = 1.0 / rx.min(ry);
                if d <= 1.0 {
                    let on_edge = d >= (1.0 - 2.0 * edge).max(0.0);
                    let c = if on_edge { pen.or(brush) } else { brush };
                    if let Some(c) = c {
                        px[(y * w + x) as usize] = colorref_to_px(c);
                    }
                }
            }
        }
    });
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn RoundRect(hdc: u32, l: i32, t: i32, r: i32, b: i32, _rw: i32, _rh: i32) -> i32 {
    Rectangle(hdc, l, t, r, b) // cantos arredondados ainda não
}

#[allow(non_snake_case)]
pub extern "stdcall" fn CreateHatchBrush(_style: u32, color: u32) -> u32 {
    add(Obj::Brush(Some(color & 0xFFFFFF))) // sem padrão de hachura: cor sólida
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn GetClipBox(hdc: u32, rect: *mut i32) -> i32 {
    match with_dc_target(hdc, |_, w, h| (w, h)) {
        Some((w, h)) => {
            *rect = 0;
            *rect.add(1) = 0;
            *rect.add(2) = w;
            *rect.add(3) = h;
            2 // SIMPLEREGION
        }
        None => 0,
    }
}

#[allow(non_snake_case)]
pub extern "stdcall" fn CreateDCA(_driver: u32, _device: u32, _port: u32, _init: u32) -> u32 {
    dc_for_window(0) // "DISPLAY": DC da tela (ainda sem alvo de desenho)
}

#[allow(non_snake_case)]
pub extern "stdcall" fn CreateICA(d: u32, e: u32, p: u32, i: u32) -> u32 {
    CreateDCA(d, e, p, i)
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn CreateBrushIndirect(lb: *const u32) -> u32 {
    // LOGBRUSH: style (0 sólido, 1 oco), color, hatch
    add(Obj::Brush(if *lb == 1 { None } else { Some(*lb.add(1) & 0xFFFFFF) }))
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn CreatePenIndirect(lp: *const u32) -> u32 {
    // LOGPEN: style, width (POINT), color
    add(Obj::Pen(if *lp == 5 { None } else { Some(*lp.add(3) & 0xFFFFFF) }))
}

#[allow(non_snake_case)]
pub extern "stdcall" fn CreatePatternBrush(_bitmap: u32) -> u32 {
    add(Obj::Brush(Some(0x808080))) // sem brush com padrão: cinza
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetBkColor(hdc: u32) -> u32 {
    GDI.lock().unwrap().dcs.get(&hdc).map_or(u32::MAX, |d| d.bk_color)
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn StretchDIBits(
    hdc: u32, xd: i32, yd: i32, wd: i32, hd: i32, xs: i32, ys: i32, ws: i32, hs: i32,
    bits: u32, bmi: u32, usage: u32, rop: u32,
) -> i32 {
    // DIB_PAL_COLORS: a tabela de cores são índices na paleta selecionada.
    let converted = if usage == 1 { dib_with_palette_colors(bmi) } else { Vec::new() };
    let header = if usage == 1 { converted.as_ptr() as u32 } else { bmi };
    let Some((w, h, px)) = decode_dib(header, Some(bits)) else { return 0 };
    // `ys` é contado a partir de baixo no DIB bottom-up; nosso buffer é top-down.
    let top = (h - ys - hs).max(0);
    let (ws, hs) = (ws.min(w - xs).max(1), hs.min(h - top).max(1));
    let mut crop = Vec::with_capacity((ws * hs) as usize);
    for y in 0..hs {
        for x in 0..ws {
            crop.push(px[((top + y) * w + xs + x) as usize]);
        }
    }
    blit_pixels(hdc, xd, yd, wd, hd, Some(&crop), ws, hs, rop);
    hs
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn CreateDIBitmap(_hdc: u32, bih: u32, init: u32, bits: u32, bmi: u32, _usage: u32) -> u32 {
    // CBM_INIT (4): inicializa com os pixels; senão, bitmap em branco do tamanho do cabeçalho.
    if init & 4 != 0 && bits != 0 && bmi != 0 {
        if let Some((w, h, px)) = decode_dib(bmi, Some(bits)) {
            return add(Obj::Bitmap { w, h, px });
        }
    }
    let rd = |o: u32| std::ptr::read_unaligned((bih + o) as usize as *const i32);
    let (w, h) = (rd(4), rd(8).abs());
    add(Obj::Bitmap { w: w.max(1), h: h.max(1), px: vec![0xFF00_0000; (w.max(1) * h.max(1)) as usize] })
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn GetTextFaceA(_hdc: u32, n: i32, buf: *mut u8) -> i32 {
    let face = b"MS Sans Serif";
    if buf.is_null() || n <= 0 {
        return face.len() as i32 + 1;
    }
    let k = face.len().min(n as usize - 1);
    std::ptr::copy_nonoverlapping(face.as_ptr(), buf, k);
    *buf.add(k) = 0;
    k as i32
}

/// Polilinha: liga os pontos com a pen atual.
pub fn polyline(hdc: u32, pts: &[(i32, i32)]) -> i32 {
    let Some(first) = pts.first() else { return 0 };
    let mut old = [0i32; 2];
    unsafe { MoveToEx(hdc, first.0, first.1, old.as_mut_ptr()) };
    for p in &pts[1..] {
        LineTo(hdc, p.0, p.1);
    }
    1
}

/// Polígono: preenche (regra par-ímpar, linha de varredura) com o brush e contorna com a pen.
pub fn polygon(hdc: u32, pts: &[(i32, i32)]) -> i32 {
    if pts.len() < 3 {
        return polyline(hdc, pts);
    }
    let Some((brush, _)) = style_of(hdc) else { return 0 };
    if let Some(c) = brush {
        let (ymin, ymax) = (pts.iter().map(|p| p.1).min().unwrap(), pts.iter().map(|p| p.1).max().unwrap());
        let px = colorref_to_px(c);
        with_dc_target(hdc, |buf, w, h| {
            for y in ymin.max(0)..=ymax.min(h - 1) {
                let mut xs: Vec<i32> = Vec::new();
                for i in 0..pts.len() {
                    let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
                    if (a.1 <= y && b.1 > y) || (b.1 <= y && a.1 > y) {
                        xs.push(a.0 + (y - a.1) * (b.0 - a.0) / (b.1 - a.1));
                    }
                }
                xs.sort();
                for pair in xs.chunks(2) {
                    if let [x0, x1] = pair {
                        for x in (*x0).max(0)..(*x1).min(w) {
                            buf[(y * w + x) as usize] = px;
                        }
                    }
                }
            }
        });
    }
    let mut closed = pts.to_vec();
    closed.push(pts[0]);
    polyline(hdc, &closed)
}

/// Cor do texto do DC (COLORREF).
pub fn text_color_of(hdc: u32) -> u32 {
    text_style(hdc).map_or(0, |t| t.0)
}

/// Modo de fundo: 1 = TRANSPARENT, 2 = OPAQUE.
pub fn bk_mode_of(hdc: u32) -> u32 {
    text_style(hdc).map_or(2, |t| if t.1.is_none() { 1 } else { 2 })
}

#[allow(non_snake_case, clippy::too_many_arguments)]
pub unsafe extern "stdcall" fn CreateFontA(
    h: i32, w: i32, esc: i32, orient: i32, weight: i32, italic: u32, under: u32, strike: u32, charset: u32,
    out_prec: u32, clip_prec: u32, quality: u32, pitch: u32, face: *const u8,
) -> u32 {
    // LOGFONTA: 5 LONGs, 8 BYTEs, nome[32]
    let mut lf = [0u8; 60];
    for (i, v) in [h, w, esc, orient, weight].iter().enumerate() {
        lf[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    for (i, v) in [italic, under, strike, charset, out_prec, clip_prec, quality, pitch].iter().enumerate() {
        lf[20 + i] = *v as u8;
    }
    if !face.is_null() {
        let name = std::ffi::CStr::from_ptr(face as *const i8).to_bytes();
        let n = name.len().min(31);
        lf[28..28 + n].copy_from_slice(&name[..n]);
    }
    CreateFontIndirectA(lf.as_ptr() as *const i32)
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn GetCurrentPositionEx(hdc: u32, pt: *mut i32) -> i32 {
    let g = GDI.lock().unwrap();
    let Some(d) = g.dcs.get(&hdc) else { return 0 };
    if !pt.is_null() {
        *pt = d.pos.0;
        *pt.add(1) = d.pos.1;
    }
    1
}

/// Quantos caracteres cabem em `max` pixels e a largura acumulada de cada prefixo.
#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn GetTextExtentExPointA(hdc: u32, s: *const u8, n: i32, max: i32, fit: *mut i32, dx: *mut i32, size: *mut i32) -> i32 {
    let mut sz = [0i32; 2];
    let mut count = 0;
    for i in 1..=n.max(0) {
        GetTextExtentPoint32A(hdc, s, i, sz.as_mut_ptr());
        if !dx.is_null() {
            *dx.add(i as usize - 1) = sz[0];
        }
        if sz[0] <= max {
            count = i;
        }
    }
    GetTextExtentPoint32A(hdc, s, n.max(0), sz.as_mut_ptr());
    if !fit.is_null() {
        *fit = count;
    }
    if !size.is_null() {
        *size = sz[0];
        *size.add(1) = sz[1];
    }
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SetTextCharacterExtra(_hdc: u32, _extra: i32) -> i32 {
    0
}

// ---- paletas lógicas --------------------------------------------------------------------------

/// Entradas no formato PALETTEENTRY (R, G, B, flags em bytes, lido como little-endian).
static PALETTES: Mutex<BTreeMap<u32, Vec<u32>>> = Mutex::new(BTreeMap::new());
/// Paleta em uso. Há uma só tela, então a última selecionada vale para todos os DCs.
static CURRENT_PALETTE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static NEXT_PALETTE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x8002);

/// Copia cabeçalho e tabela de um BITMAPINFO trocando os índices (u16) pelas cores RGBQUAD da paleta em uso.
unsafe fn dib_with_palette_colors(bmi: u32) -> Vec<u8> {
    let rd = |o: u32| std::ptr::read_unaligned((bmi + o) as usize as *const u32);
    let size = rd(0) as usize;
    let bpp = rd(14) & 0xFFFF;
    let colors = if bpp <= 8 { match rd(32) { 0 => 1usize << bpp, n => n as usize } } else { 0 };
    let mut out = vec![0u8; size + colors * 4];
    std::ptr::copy_nonoverlapping(bmi as usize as *const u8, out.as_mut_ptr(), size);
    let pal = PALETTES.lock().unwrap().get(&CURRENT_PALETTE.load(std::sync::atomic::Ordering::Relaxed)).cloned().unwrap_or_default();
    for i in 0..colors {
        let idx = std::ptr::read_unaligned((bmi as usize + size + i * 2) as *const u16) as usize;
        let e = pal.get(idx).copied().unwrap_or(0);
        // PALETTEENTRY = R, G, B; RGBQUAD = B, G, R
        out[size + i * 4..size + i * 4 + 3].copy_from_slice(&[(e >> 16) as u8, (e >> 8) as u8, e as u8]);
    }
    out
}

/// LOGPALETTE: palVersion, palNumEntries (u16) e as entradas.
#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn CreatePalette(log: *const u16) -> u32 {
    if log.is_null() {
        return 0;
    }
    let n = *log.add(1) as usize;
    let first = log.add(2) as *const u32;
    let entries = (0..n).map(|i| std::ptr::read_unaligned(first.add(i))).collect();
    let h = NEXT_PALETTE.fetch_add(4, std::sync::atomic::Ordering::Relaxed);
    PALETTES.lock().unwrap().insert(h, entries);
    h
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SelectPalette(_hdc: u32, pal: u32, _background: i32) -> u32 {
    use std::sync::atomic::Ordering;
    if PALETTES.lock().unwrap().contains_key(&pal) {
        CURRENT_PALETTE.swap(pal, Ordering::Relaxed)
    } else {
        CURRENT_PALETTE.load(Ordering::Relaxed) // paletas de estoque não trocam a cor das imagens
    }
}

#[allow(non_snake_case)]
pub extern "stdcall" fn RealizePalette(_hdc: u32) -> u32 {
    PALETTES.lock().unwrap().get(&CURRENT_PALETTE.load(std::sync::atomic::Ordering::Relaxed)).map_or(0, |p| p.len() as u32)
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn GetPaletteEntries(pal: u32, start: u32, n: u32, out: *mut u32) -> u32 {
    let t = PALETTES.lock().unwrap();
    let Some(p) = t.get(&pal) else { return 0 };
    if out.is_null() {
        return p.len() as u32;
    }
    let count = n.min((p.len() as u32).saturating_sub(start));
    for i in 0..count {
        std::ptr::write_unaligned(out.add(i as usize), p[(start + i) as usize]);
    }
    count
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn SetPaletteEntries(pal: u32, start: u32, n: u32, entries: *const u32) -> u32 {
    let mut t = PALETTES.lock().unwrap();
    let Some(p) = t.get_mut(&pal) else { return 0 };
    let count = n.min((p.len() as u32).saturating_sub(start));
    for i in 0..count {
        p[(start + i) as usize] = std::ptr::read_unaligned(entries.add(i as usize));
    }
    count
}

#[allow(non_snake_case)]
pub extern "stdcall" fn ResizePalette(pal: u32, n: u32) -> i32 {
    match PALETTES.lock().unwrap().get_mut(&pal) {
        Some(p) => {
            p.resize(n as usize, 0);
            1
        }
        None => 0,
    }
}
