//! Drawing primitives on 0xAARRGGBB buffers, used by the native controls.

pub const GRAY: u32 = 0xFFC0_C0C0;
pub const WHITE: u32 = 0xFFFF_FFFF;
pub const DARK: u32 = 0xFF80_8080;
pub const BLACK: u32 = 0xFF00_0000;

pub struct Buf<'a> {
    pub px: &'a mut [u32],
    pub w: i32,
    pub h: i32,
}

impl Buf<'_> {
    pub fn fill(&mut self, l: i32, t: i32, r: i32, b: i32, c: u32) {
        for y in t.max(0)..b.min(self.h) {
            for x in l.max(0)..r.min(self.w) {
                self.px[(y * self.w + x) as usize] = c;
            }
        }
    }

    /// 1 px outline with top/left color `tl` and bottom/right color `br`.
    pub fn frame(&mut self, l: i32, t: i32, r: i32, b: i32, tl: u32, br: u32) {
        self.fill(l, t, r - 1, t + 1, tl);
        self.fill(l, t, l + 1, b - 1, tl);
        self.fill(l, b - 1, r, b, br);
        self.fill(r - 1, t, r, b, br);
    }

    pub fn raised(&mut self, l: i32, t: i32, r: i32, b: i32) {
        self.frame(l, t, r, b, GRAY, BLACK);
        self.frame(l + 1, t + 1, r - 1, b - 1, WHITE, DARK);
    }

    pub fn sunken(&mut self, l: i32, t: i32, r: i32, b: i32) {
        self.frame(l, t, r, b, DARK, WHITE);
        self.frame(l + 1, t + 1, r - 1, b - 1, BLACK, GRAY);
    }

    pub fn text(&mut self, x: i32, y: i32, s: &[u8], color: u32, bold: bool) -> i32 {
        super::text::draw(self.px, self.w, self.h, x, y, s, color, bold)
    }
}
