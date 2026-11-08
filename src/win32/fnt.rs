//! Windows 2.x/3.x bitmap fonts (FNT format, inside NE `.FON` files): reading and drawing.

use std::sync::{Arc, Mutex};

pub struct Fnt {
    pub face: String,
    pub height: i32,
    pub ascent: i32,
    pub internal_leading: i32,
    pub external_leading: i32,
    pub weight: i32,
    pub avg_width: i32,
    pub max_width: i32,
    first: u8,
    last: u8,
    default: u8,
    /// (width, file offset) per character from `first` to `last`.
    glyphs: Vec<(u16, u32)>,
    data: Vec<u8>,
}

static FONTS: Mutex<Vec<Arc<Fnt>>> = Mutex::new(Vec::new());

fn u16_at(d: &[u8], o: usize) -> Option<u16> {
    d.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]))
}

fn u32_at(d: &[u8], o: usize) -> Option<u32> {
    d.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// Parses a FNT file (versions 0x200 and 0x300), proportional or fixed bitmap.
fn parse_fnt(d: &[u8]) -> Option<Fnt> {
    let ver = u16_at(d, 0)?;
    if (d.len() < 118) || (ver != 0x200 && ver != 0x300) || u16_at(d, 66)? & 1 != 0 {
        return None; // dfType bit 0 = vector
    }
    let (first, last, default) = (*d.get(95)?, *d.get(96)?, *d.get(97)?);
    let n = (last as usize + 1).checked_sub(first as usize)?;
    let height = u16_at(d, 88)? as i32;
    let table = if ver == 0x300 { 148 } else { 118 };
    let entry = if ver == 0x300 { 6 } else { 4 };
    let mut glyphs = Vec::with_capacity(n + 1);
    for i in 0..=n {
        let o = table + i * entry;
        let w = u16_at(d, o)?;
        let off = if ver == 0x300 { u32_at(d, o + 2)? } else { u16_at(d, o + 2)? as u32 };
        glyphs.push((w, off));
    }
    let face_off = u32_at(d, 105)? as usize;
    let face_end = d.get(face_off..)?.iter().position(|&c| c == 0)? + face_off;
    Some(Fnt {
        face: String::from_utf8_lossy(&d[face_off..face_end]).into_owned(),
        height,
        ascent: u16_at(d, 74)? as i32,
        internal_leading: u16_at(d, 76)? as i32,
        external_leading: u16_at(d, 78)? as i32,
        weight: u16_at(d, 83)? as i32,
        avg_width: u16_at(d, 91)? as i32,
        max_width: u16_at(d, 93)? as i32,
        first,
        last,
        default,
        glyphs,
        data: d.to_vec(),
    })
}

/// Loads all fonts (RT_FONT = 8 resource) of a NE `.FON` module; returns how many were added.
pub fn load_fon(d: &[u8]) -> usize {
    let Some(h) = u32_at(d, 0x3C).map(|v| v as usize) else { return 0 };
    if d.get(h..h + 2) != Some(b"NE") {
        return parse_fnt(d).map_or(0, |f| add(f));
    }
    let (Some(rs), Some(restab)) = (u16_at(d, h + 0x24), u16_at(d, h + 0x26)) else { return 0 };
    if rs == restab {
        return 0;
    }
    let base = h + rs as usize;
    let Some(shift) = u16_at(d, base) else { return 0 };
    let (mut p, mut count) = (base + 2, 0);
    while let Some(tid) = u16_at(d, p) {
        if tid == 0 {
            break;
        }
        let n = u16_at(d, p + 2).unwrap_or(0);
        p += 8;
        for _ in 0..n {
            let (o, l) = ((u16_at(d, p).unwrap_or(0) as usize) << shift, (u16_at(d, p + 2).unwrap_or(0) as usize) << shift);
            p += 12;
            if tid & 0x7FFF == 8 {
                if let Some(f) = d.get(o..o + l).and_then(parse_fnt) {
                    count += add(f);
                }
            }
        }
    }
    count
}

fn add(f: Fnt) -> usize {
    let mut all = FONTS.lock().unwrap();
    if all.iter().any(|x| x.face.eq_ignore_ascii_case(&f.face) && x.height == f.height) {
        return 0;
    }
    all.push(Arc::new(f));
    1
}

/// Picks the loaded font with the given name and the cell height closest to `want_h`
/// (`want_h` < 0: character height, without internal leading; 0: any). Returns index+1.
pub fn find(face: &str, want_h: i32) -> u32 {
    let all = FONTS.lock().unwrap();
    all.iter()
        .enumerate()
        .filter(|(_, f)| f.face.eq_ignore_ascii_case(face))
        .min_by_key(|(_, f)| {
            let h = if want_h < 0 { f.height - f.internal_leading } else { f.height };
            (h - want_h.abs()).abs()
        })
        .map_or(0, |(i, _)| i as u32 + 1)
}

pub fn get(idx: u32) -> Option<Arc<Fnt>> {
    if idx == 0 { None } else { FONTS.lock().unwrap().get(idx as usize - 1).cloned() }
}

pub fn faces() -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for f in FONTS.lock().unwrap().iter() {
        if !v.iter().any(|x| x.eq_ignore_ascii_case(&f.face)) {
            v.push(f.face.clone());
        }
    }
    v
}

impl Fnt {
    fn index(&self, c: u8) -> usize {
        let c = if c < self.first || c > self.last { self.default } else { c };
        c.wrapping_sub(self.first) as usize
    }

    pub fn char_width(&self, c: u8) -> i32 {
        self.glyphs.get(self.index(c)).map_or(0, |g| g.0 as i32)
    }

    pub fn width(&self, s: &[u8]) -> i32 {
        s.iter().map(|&c| self.char_width(c)).sum()
    }

    /// Draws `s` with the top-left corner at (x, y); returns the width.
    pub fn draw(&self, px: &mut [u32], bw: i32, bh: i32, x: i32, y: i32, s: &[u8], color: u32) -> i32 {
        let mut pen = x;
        for &c in s {
            let Some(&(w, off)) = self.glyphs.get(self.index(c)) else { continue };
            // bitmap in 8-pixel columns: for each column, `height` bytes top to bottom
            for col in 0..(w as usize + 7) / 8 {
                for row in 0..self.height as usize {
                    let Some(&byte) = self.data.get(off as usize + col * self.height as usize + row) else { continue };
                    for bit in 0..8 {
                        let cx = col * 8 + bit;
                        if cx >= w as usize || byte & (0x80 >> bit) == 0 {
                            continue;
                        }
                        let (gx, gy) = (pen + cx as i32, y + row as i32);
                        if gx >= 0 && gy >= 0 && gx < bw && gy < bh {
                            px[(gy * bw + gx) as usize] = color;
                        }
                    }
                }
            }
            pen += w as i32;
        }
        pen - x
    }
}
