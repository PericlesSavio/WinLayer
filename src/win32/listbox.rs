//! Caixa de listagem (`listbox`): itens, ordenação, várias colunas, seleção e desenho.
//! Mensagens `LB_*` com a numeração do Win32 (0x180..); o Win16 é traduzido em `win16/user.rs`.

use super::draw::{self, Buf};
use super::text;
use std::collections::BTreeMap;
use std::sync::Mutex;

pub const LBS_NOTIFY: u32 = 0x1;
pub const LBS_SORT: u32 = 0x2;
pub const LBS_MULTICOLUMN: u32 = 0x200;
pub const WS_HSCROLL: u32 = 0x10_0000;
pub const WS_VSCROLL: u32 = 0x20_0000;

const SB: i32 = 16; // espessura da barra de rolagem

#[derive(Default)]
struct List {
    items: Vec<(Vec<u8>, u32)>,
    sel: i32,
    /// Primeira linha (coluna única) ou primeira coluna (várias colunas) visível.
    top: i32,
    colw: i32,
}

static LISTS: Mutex<BTreeMap<u32, List>> = Mutex::new(BTreeMap::new());

fn row_h() -> i32 {
    text::line_h()
}

/// Disposição: (linhas visíveis por coluna, área útil em largura/altura, barra horizontal?, vertical?).
fn layout(l: &List, style: u32, w: i32, h: i32) -> (i32, i32, i32, bool, bool) {
    let multi = style & LBS_MULTICOLUMN != 0;
    let inner_w = w - 4;
    let mut inner_h = h - 4;
    let hbar = multi && style & WS_HSCROLL != 0;
    if hbar {
        inner_h -= SB;
    }
    let rows = (inner_h / row_h()).max(1);
    let vbar = !multi && l.items.len() as i32 > rows;
    (rows, inner_w - if vbar { SB } else { 0 }, inner_h, hbar, vbar)
}

fn cstr(p: u32) -> Vec<u8> {
    if p == 0 {
        return Vec::new();
    }
    unsafe { std::ffi::CStr::from_ptr(p as usize as *const i8) }.to_bytes().to_vec()
}

fn ci_cmp(a: &[u8], b: &[u8]) -> std::cmp::Ordering {
    a.iter().map(u8::to_ascii_lowercase).cmp(b.iter().map(u8::to_ascii_lowercase))
}

fn starts_with_ci(s: &[u8], prefix: &[u8]) -> bool {
    s.len() >= prefix.len() && s[..prefix.len()].eq_ignore_ascii_case(prefix)
}

fn count_cols(l: &List, rows: i32) -> i32 {
    (l.items.len() as i32 + rows - 1) / rows.max(1)
}

/// Rola para que o item `i` fique visível.
fn show_item(l: &mut List, style: u32, rows: i32, view_w: i32, i: i32) {
    if style & LBS_MULTICOLUMN != 0 {
        let cw = l.colw.max(1);
        let vis = (view_w / cw).max(1);
        let col = i / rows;
        if col < l.top {
            l.top = col;
        } else if col >= l.top + vis {
            l.top = col - vis + 1;
        }
    } else if i < l.top {
        l.top = i;
    } else if i >= l.top + rows {
        l.top = i - rows + 1;
    }
}

/// Trata uma mensagem `LB_*`. `None` se não for uma mensagem de listbox.
pub fn message(hwnd: u32, style: u32, w_: i32, h_: i32, msg: u32, wp: u32, lp: u32) -> Option<u32> {
    if !(0x180..=0x1A2).contains(&msg) {
        return None;
    }
    let mut all = LISTS.lock().unwrap();
    let l = all.entry(hwnd).or_insert_with(|| List { sel: -1, ..Default::default() });
    let idx = wp as u16 as i16 as i32; // Win16 passa -1 como 0xFFFF
    let r = match msg {
        0x180 | 0x181 => {
            let item = (cstr(lp), 0);
            let at = if msg == 0x180 {
                if style & LBS_SORT != 0 {
                    l.items.iter().position(|(t, _)| ci_cmp(t, &item.0) == std::cmp::Ordering::Greater).unwrap_or(l.items.len())
                } else {
                    l.items.len()
                }
            } else if idx < 0 || idx as usize > l.items.len() {
                l.items.len()
            } else {
                idx as usize
            };
            l.items.insert(at, item);
            if l.sel >= at as i32 {
                l.sel += 1;
            }
            at as u32
        }
        0x182 => {
            if idx >= 0 && (idx as usize) < l.items.len() {
                l.items.remove(idx as usize);
                if l.sel == idx {
                    l.sel = -1;
                } else if l.sel > idx {
                    l.sel -= 1;
                }
            }
            l.items.len() as u32
        }
        0x184 => {
            l.items.clear();
            l.sel = -1;
            l.top = 0;
            0
        }
        0x186 => {
            l.sel = if idx >= 0 && (idx as usize) < l.items.len() { idx } else { -1 };
            if l.sel >= 0 {
                let (rows, vw, _, _, _) = layout(l, style, w_, h_);
                show_item(l, style, rows, vw, l.sel);
            }
            if l.sel < 0 && idx >= 0 { u32::MAX } else { 0 }
        }
        0x187 => (l.sel == idx && idx >= 0) as u32,
        0x188 => l.sel as u32,
        0x189 => match l.items.get(idx as usize) {
            Some((t, _)) if lp != 0 => unsafe {
                std::ptr::copy_nonoverlapping(t.as_ptr(), lp as usize as *mut u8, t.len());
                *((lp as usize + t.len()) as *mut u8) = 0;
                t.len() as u32
            },
            _ => u32::MAX,
        },
        0x18A => l.items.get(idx as usize).map_or(u32::MAX, |(t, _)| t.len() as u32),
        0x18B => l.items.len() as u32,
        0x18C | 0x18F | 0x1A2 => {
            let pat = cstr(lp);
            let n = l.items.len();
            let start = if idx < 0 { 0 } else { (idx as usize + 1) % n.max(1) };
            let found = (0..n).map(|k| (start + k) % n).find(|&i| if msg == 0x1A2 { l.items[i].0.eq_ignore_ascii_case(&pat) } else { starts_with_ci(&l.items[i].0, &pat) });
            match found {
                Some(i) => {
                    if msg == 0x18C {
                        l.sel = i as i32;
                        let (rows, vw, _, _, _) = layout(l, style, w_, h_);
                        show_item(l, style, rows, vw, i as i32);
                    }
                    i as u32
                }
                None => u32::MAX,
            }
        }
        0x18E => l.top as u32,
        0x197 => {
            l.top = idx.max(0);
            0
        }
        0x195 => {
            l.colw = wp as i32;
            0
        }
        0x199 => l.items.get(idx as usize).map_or(u32::MAX, |(_, d)| *d),
        0x19A => {
            if let Some(it) = l.items.get_mut(idx as usize) {
                it.1 = lp;
                0
            } else {
                u32::MAX
            }
        }
        0x1A0 | 0x193 | 0x194 | 0x192 => 1, // altura/extensão/tabs: sem efeito
        _ => 0,
    };
    Some(r)
}

pub fn remove(hwnd: u32) {
    LISTS.lock().unwrap().remove(&hwnd);
}

pub fn selection(hwnd: u32) -> i32 {
    LISTS.lock().unwrap().get(&hwnd).map_or(-1, |l| l.sel)
}

/// Clique em (x, y) do cliente: seleciona o item (ou rola pelas barras). Devolve se a seleção mudou.
pub fn click(hwnd: u32, style: u32, w: i32, h: i32, x: i32, y: i32) -> Option<bool> {
    let mut all = LISTS.lock().unwrap();
    let l = all.entry(hwnd).or_insert_with(|| List { sel: -1, ..Default::default() });
    let multi = style & LBS_MULTICOLUMN != 0;
    let (rows, vw, vh, hbar, vbar) = layout(l, style, w, h);
    if hbar && y >= h - 2 - SB {
        // barra horizontal: setas nas pontas, trilho = página
        let cw = l.colw.max(1);
        let vis = (vw / cw).max(1);
        let ncols = count_cols(l, rows);
        l.top = if x < 2 + SB { l.top - 1 } else if x >= w - 2 - SB { l.top + 1 } else if x < w / 2 { l.top - vis } else { l.top + vis };
        l.top = l.top.clamp(0, (ncols - vis).max(0));
        return Some(false);
    }
    if vbar && x >= w - 2 - SB {
        l.top = if y < 2 + SB { l.top - 1 } else if y >= h - 2 - SB { l.top + 1 } else if y < h / 2 { l.top - rows } else { l.top + rows };
        l.top = l.top.clamp(0, (l.items.len() as i32 - rows).max(0));
        return Some(false);
    }
    if x < 2 || y < 2 || y >= 2 + vh || x >= 2 + vw {
        return None;
    }
    let row = (y - 2) / row_h();
    let i = if multi { (l.top + (x - 2) / l.colw.max(1)) * rows + row } else { l.top + row };
    if i < 0 || i as usize >= l.items.len() {
        return None;
    }
    let changed = l.sel != i;
    l.sel = i;
    Some(changed)
}

/// Desenha a lista no buffer do controle.
pub fn paint(hwnd: u32, style: u32, b: &mut Buf, focused: bool) {
    let mut all = LISTS.lock().unwrap();
    let l = all.entry(hwnd).or_insert_with(|| List { sel: -1, ..Default::default() });
    let (w, h) = (b.w, b.h);
    b.fill(0, 0, w, h, draw::WHITE);
    b.sunken(0, 0, w, h);
    let multi = style & LBS_MULTICOLUMN != 0;
    let (rows, vw, vh, hbar, vbar) = layout(l, style, w, h);
    let cw = if multi { l.colw.max(1) } else { vw };
    let rh = row_h();
    let ncols = count_cols(l, rows);
    for (i, (t, _)) in l.items.iter().enumerate() {
        let i = i as i32;
        let (col, row) = if multi { (i / rows - l.top, i % rows) } else { (0, i - l.top) };
        if col < 0 || row < 0 || row >= rows || (multi && col * cw >= vw) || (!multi && row >= rows) {
            continue;
        }
        let (x, y) = (2 + col * cw, 2 + row * rh);
        let sel = i == l.sel;
        if sel {
            b.fill(x, y, (x + cw).min(2 + vw), y + rh, 0xFF00_0080);
            if focused {
                b.fill(x, y, (x + cw).min(2 + vw), y + 1, draw::DARK);
            }
        }
        // recorta o texto à coluna
        let mut label = t.clone();
        while label.len() > 1 && text::width(false, &label) > cw - 4 {
            label.pop();
        }
        b.text(x + 2, y, &label, if sel { draw::WHITE } else { draw::BLACK }, false);
    }
    // barras de rolagem
    let arrow = |b: &mut Buf, l: i32, t: i32, r: i32, bt: i32| {
        b.fill(l, t, r, bt, draw::GRAY);
        b.raised(l, t, r, bt);
    };
    if hbar {
        let y = h - 2 - SB;
        b.fill(2, y, w - 2, y + SB, 0xFFE0_E0E0);
        arrow(b, 2, y, 2 + SB, y + SB);
        arrow(b, w - 2 - SB, y, w - 2, y + SB);
        let cwv = (vw / cw.max(1)).max(1);
        if ncols > cwv {
            let track = w - 4 - 3 * SB;
            let tx = 2 + SB + (track * l.top / ncols).max(0);
            arrow(b, tx, y, tx + (track * cwv / ncols).max(8), y + SB);
        }
    }
    if vbar {
        let x = w - 2 - SB;
        b.fill(x, 2, x + SB, 2 + vh, 0xFFE0_E0E0);
        arrow(b, x, 2, x + SB, 2 + SB);
        arrow(b, x, 2 + vh - SB, x + SB, 2 + vh);
        let n = l.items.len() as i32;
        let track = vh - 3 * SB;
        let ty = 2 + SB + (track * l.top / n).max(0);
        arrow(b, x, ty, x + SB, ty + (track * rows / n).max(8));
    }
}
