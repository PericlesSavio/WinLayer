//! Fonte bitmap embutida (ver `assets/README.md`) e desenho de texto em buffers de pixels.

static REGULAR: &[u8] = include_bytes!("../../assets/font_regular.bin");
static BOLD: &[u8] = include_bytes!("../../assets/font_bold.bin");

const FIRST: u8 = 32;
const NGLYPHS: usize = 224;
const HEADER: usize = 2;
const ENTRY: usize = 8;

pub const ASCENT: i32 = 10;
/// Altura de linha usada nas métricas (tmHeight).
pub const LINE_H: i32 = 13;

/// Programa Win16: a fonte padrão dos DCs é a "System" do Windows 3.1 (altura 16, largura média 8),
/// maior que a embutida; os programas medem o texto com ela antes de escolher a fonte própria.
pub static WIN16: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn win16() -> bool {
    WIN16.load(std::sync::atomic::Ordering::Relaxed)
}

/// Altura de linha e ascendente da fonte embutida (conforme o modo).
pub fn line_h() -> i32 {
    if win16() { 16 } else { LINE_H }
}

pub fn ascent() -> i32 {
    if win16() { 13 } else { ASCENT }
}

fn font(bold: bool) -> &'static [u8] {
    if bold { BOLD } else { REGULAR }
}

struct Glyph<'a> {
    adv: i32,
    left: i32,
    w: i32,
    h: i32,
    top: i32,
    bits: &'a [u8],
}

fn glyph(bold: bool, ch: u8) -> Option<Glyph<'static>> {
    if ch < FIRST || (ch - FIRST) as usize >= NGLYPHS {
        return None;
    }
    let f = font(bold);
    let e = HEADER + (ch - FIRST) as usize * ENTRY;
    let (adv, left, w, h, top) = (f[e] as i32, f[e + 1] as i8 as i32, f[e + 2] as i32, f[e + 3] as i32, f[e + 4] as i8 as i32);
    let off = u16::from_le_bytes([f[e + 6], f[e + 7]]) as usize;
    let start = HEADER + NGLYPHS * ENTRY + off;
    let len = ((w as usize + 7) / 8) * h as usize;
    Some(Glyph { adv, left, w, h, top, bits: &f[start..start + len] })
}

/// Largura em pixels, ignorando `&` (mnemônico) como o Windows faz em controles.
pub fn width(bold: bool, s: &[u8]) -> i32 {
    let extra = win16() as i32;
    s.iter().filter_map(|&c| glyph(bold, c)).map(|g| g.adv + extra).sum()
}

/// Desenha `s` com o canto superior esquerdo em (x, y). Devolve a largura desenhada.
pub fn draw(px: &mut [u32], bw: i32, bh: i32, x: i32, y: i32, s: &[u8], color: u32, bold: bool) -> i32 {
    let mut pen = x;
    for &c in s {
        let Some(g) = glyph(bold, c) else { continue };
        let stride = (g.w as usize + 7) / 8;
        for row in 0..g.h {
            for col in 0..g.w {
                if g.bits[row as usize * stride + col as usize / 8] & (0x80 >> (col % 8)) != 0 {
                    let (gx, gy) = (pen + g.left + col, y + ascent() - g.top + row);
                    if gx >= 0 && gy >= 0 && gx < bw && gy < bh {
                        px[(gy * bw + gx) as usize] = color;
                    }
                }
            }
        }
        pen += g.adv + win16() as i32;
    }
    pen - x
}

/// Remove `&` de mnemônicos ("&OK" -> "OK"; "&&" -> "&").
pub fn strip_mnemonic(s: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'&' {
            if b.get(i + 1) == Some(&b'&') { out.push(b'&'); i += 1 }
        } else {
            out.push(b[i]);
        }
        i += 1;
    }
    out
}
