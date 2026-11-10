//! Input: events from the viewer (or from an `--events` script) become Windows messages.
//!
//! Protocol (one line per event):
//!   `m X Y`  mouse at desktop (X, Y)   `d B` / `u B`  button B (0 left, 1 middle, 2 right) down/up
//!   `kd VK` / `ku VK`  virtual key      `ch C`  character   `wait N`  (script only) wait N frames

use super::*;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering};
use std::time::Instant;

pub(super) enum Ev {
    Move(i32, i32),
    Down(u32),
    Up(u32),
    /// (vk, explicit scan code: bit 31 = present, bits 0-7 = scancode, bit 8 = extended key)
    KeyDown(u32, u32),
    KeyUp(u32, u32),
    Char(u32),
    Wait(u32),
    /// Intermediate capture: saves `<--shot without .png>-N.png` and continues running.
    Shot(u32),
    /// Simulates window resizing: new virtual screen WxH (applied between messages).
    Size(u32, u32),
}

static EVENTS: Mutex<VecDeque<Ev>> = Mutex::new(VecDeque::new());
static VERSION: AtomicU64 = AtomicU64::new(1);
static CUR_X: AtomicI32 = AtomicI32::new(0);
static CUR_Y: AtomicI32 = AtomicI32::new(0);
static BUTTONS: AtomicU32 = AtomicU32::new(0);
static WAITS: AtomicU32 = AtomicU32::new(0);
static VNOW: AtomicU32 = AtomicU32::new(0);
static START: Mutex<Option<Instant>> = Mutex::new(None);
static SHOT: AtomicBool = AtomicBool::new(false);

const MK_LBUTTON: u32 = 0x01;
const MK_RBUTTON: u32 = 0x02;
const MK_SHIFT: u32 = 0x04;
const MK_CONTROL: u32 = 0x08;
const MK_MBUTTON: u32 = 0x10;

const WM_SETFOCUS: u32 = 0x07;
const WM_KILLFOCUS: u32 = 0x08;
const WM_KEYDOWN: u32 = 0x100;
const WM_KEYUP: u32 = 0x101;
const WM_CHAR: u32 = 0x102;
const WM_COMMAND: u32 = 0x111;
const WM_INITMENU: u32 = 0x116;
const WM_INITMENUPOPUP: u32 = 0x117;
const WM_MOUSEMOVE: u32 = 0x200;
const WM_LBUTTONDOWN: u32 = 0x201;
const CS_DBLCLKS: u32 = 0x08;
const VK_SHIFT: u32 = 16;
const VK_CONTROL: u32 = 17;
const VK_MENU: u32 = 18;
const VK_TAB: u32 = 9;
const VK_RETURN: u32 = 13;
const VK_ESCAPE: u32 = 27;
const ROW_H: i32 = 19;
const SEP_H: i32 = 7;

pub fn set_shot_mode(on: bool) {
    SHOT.store(on, Ordering::Relaxed);
}

pub fn shot_mode() -> bool {
    SHOT.load(Ordering::Relaxed)
}

/// Current cursor position on the desktop.
pub(super) fn cursor() -> (i32, i32) {
    (CUR_X.load(Ordering::Relaxed), CUR_Y.load(Ordering::Relaxed))
}

/// Flags that the image changed (wakes the viewer).
pub fn touch() {
    VERSION.fetch_add(1, Ordering::Relaxed);
}

pub fn frame_version() -> u64 {
    VERSION.load(Ordering::Relaxed)
}

/// Process clock in ms. In capture mode it is virtual (advances on each idle frame).
pub fn now_ms() -> u32 {
    if shot_mode() {
        return VNOW.load(Ordering::Relaxed);
    }
    START.lock().unwrap().get_or_insert_with(Instant::now).elapsed().as_millis() as u32
}

/// Real clock in ms since first use (also in capture mode, for busy waits).
pub fn real_ms() -> u32 {
    START.lock().unwrap().get_or_insert_with(Instant::now).elapsed().as_millis() as u32
}

/// Advances the virtual clock (capture mode). No effect in normal mode.
pub fn tick_virtual(ms: u32) {
    if shot_mode() {
        VNOW.fetch_add(ms, Ordering::Relaxed);
    }
}

/// A presented DirectDraw frame also counts as one script `wait` frame: games that poll the
/// input every frame may never leave the message queue empty, so `idle` alone would not advance the script.
pub fn frame_wait() {
    if shot_mode() {
        take_wait();
    }
}

pub(super) fn advance_virtual(ms: u32) {
    VNOW.fetch_add(ms, Ordering::Relaxed);
}

/// Remaining wait frames of the script; consumes one and returns true if there was one.
pub(super) fn take_wait() -> bool {
    let w = WAITS.load(Ordering::Relaxed);
    if w > 0 {
        WAITS.store(w - 1, Ordering::Relaxed);
        true
    } else {
        false
    }
}

pub(super) fn has_pending() -> bool {
    WAITS.load(Ordering::Relaxed) > 0 || !EVENTS.lock().unwrap().is_empty()
}

pub fn push_events(text: &str) {
    let mut q = EVENTS.lock().unwrap();
    for line in text.split(['\n', ';']) {
        let mut it = line.split_whitespace();
        let Some(cmd) = it.next() else { continue };
        let n: Vec<i64> = it.filter_map(|v| v.parse().ok()).collect();
        let ev = match (cmd, n.as_slice()) {
            ("m", [x, y]) => Ev::Move(*x as i32, *y as i32),
            ("d", [b]) => Ev::Down(*b as u32),
            ("u", [b]) => Ev::Up(*b as u32),
            ("kd", [k]) => Ev::KeyDown(*k as u32, 0),
            ("ku", [k]) => Ev::KeyUp(*k as u32, 0),
            // with scancode and extended key flag: "kd VK SCAN EXT"
            ("kd", [k, sc, ex]) => Ev::KeyDown(*k as u32, 0x8000_0000 | (*sc as u32 & 0xFF) | ((*ex as u32 & 1) << 8)),
            ("ku", [k, sc, ex]) => Ev::KeyUp(*k as u32, 0x8000_0000 | (*sc as u32 & 0xFF) | ((*ex as u32 & 1) << 8)),
            ("ch", [c]) => Ev::Char(*c as u32),
            ("wait", [w]) => Ev::Wait(*w as u32),
            ("shot", [n]) => Ev::Shot(*n as u32),
            ("size", [w, h]) => Ev::Size(*w as u32, *h as u32),
            _ => continue,
        };
        q.push_back(ev);
    }
}

/// Desktop in BGRX (32 bpp little-endian, as X11 expects).
pub fn render_bgrx() -> (usize, usize, Vec<u8>) {
    let (w, h, desk) = render_desktop_sized();
    let mut out = Vec::with_capacity(desk.len() * 4);
    for p in desk {
        out.extend_from_slice(&[p as u8, (p >> 8) as u8, (p >> 16) as u8, 0]);
    }
    (w, h, out)
}

pub fn render_rgba() -> (usize, usize, Vec<u8>) {
    let (w, h, desk) = render_desktop_sized();
    let mut out = Vec::with_capacity(desk.len() * 4);
    for p in desk {
        out.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8, 255]);
    }
    (w, h, out)
}

// ---- popup menu state ---------------------------------------------------------------------

pub(super) struct Level {
    pub menu: u32,
    pub x: i32,
    pub y: i32,
    pub hover: Option<usize>,
}

pub(super) struct Popup {
    pub owner: u32,
    pub bar_menu: u32,
    pub levels: Vec<Level>,
}

fn item_height(it: &menu::Item) -> i32 {
    if it.flags & menu::MF_SEPARATOR != 0 { SEP_H } else { ROW_H }
}

/// Splits "text\tshortcut" into the two menu columns.
fn split_tab(label: &[u8]) -> (&[u8], &[u8]) {
    match label.iter().position(|&c| c == b'\t') {
        Some(i) => (&label[..i], &label[i + 1..]),
        None => (label, &[]),
    }
}

pub(super) fn level_size(menu_h: u32) -> (i32, i32) {
    let items = menu::items(menu_h);
    let (mut lw, mut rw) = (0, 0);
    for i in &items {
        let l = text::strip_mnemonic(&i.text);
        let (a, b) = split_tab(&l);
        lw = lw.max(text::width(false, a));
        rw = rw.max(text::width(false, b));
    }
    let w = lw + if rw > 0 { rw + 20 } else { 0 } + 44;
    (w, items.iter().map(item_height).sum::<i32>() + 4)
}

/// Index of the item under the point (relative to the top of the level) and the y of its top.
fn level_item_at(menu_h: u32, ry: i32) -> Option<(usize, i32)> {
    let mut y = 2;
    for (i, it) in menu::items(menu_h).iter().enumerate() {
        let h = item_height(it);
        if ry >= y && ry < y + h {
            return Some((i, y));
        }
        y += h;
    }
    None
}

/// Rectangles (x0, x1) of the window's menu bar items, on screen.
fn bar_layout(w: &Window, cx: i32) -> Vec<(i32, i32, menu::Item)> {
    let mut x = cx + 8;
    menu::items(w.menu)
        .into_iter()
        .map(|it| {
            let tw = text::width(false, &text::strip_mnemonic(&it.text));
            let r = (x, x + tw + 14, it);
            x += tw + 14;
            r
        })
        .collect()
}

pub(super) fn draw_popups(s: &State, desk: &mut [u32]) {
    let Some(p) = &s.popup else { return };
    let mut b = Buf { px: desk, w: desktop_w() as i32, h: desktop_h() as i32 };
    for lv in &p.levels {
        let (w, h) = level_size(lv.menu);
        b.fill(lv.x, lv.y, lv.x + w, lv.y + h, draw::GRAY);
        b.raised(lv.x, lv.y, lv.x + w, lv.y + h);
        let mut y = lv.y + 2;
        for (i, it) in menu::items(lv.menu).iter().enumerate() {
            let ih = item_height(it);
            if it.flags & menu::MF_SEPARATOR != 0 {
                b.fill(lv.x + 3, y + 3, lv.x + w - 3, y + 4, draw::DARK);
                b.fill(lv.x + 3, y + 4, lv.x + w - 3, y + 5, draw::WHITE);
            } else {
                let hot = lv.hover == Some(i);
                let off = it.flags & (menu::MF_GRAYED | menu::MF_DISABLED) != 0;
                if hot {
                    b.fill(lv.x + 3, y, lv.x + w - 3, y + ih, 0xFF00_0080);
                }
                let col = if hot { draw::WHITE } else if off { draw::DARK } else { draw::BLACK };
                let label = text::strip_mnemonic(&it.text);
                let (left, right) = split_tab(&label);
                b.text(lv.x + 22, y + 3, left, col, false);
                if !right.is_empty() {
                    b.text(lv.x + w - 22 - text::width(false, right), y + 3, right, col, false);
                }
                if it.flags & menu::MF_CHECKED != 0 {
                    b.text(lv.x + 8, y + 3, b"v", col, true);
                }
                if it.sub != 0 {
                    b.text(lv.x + w - 12, y + 3, b">", col, false);
                }
            }
            y += ih;
        }
    }
}

// ---- hit test -----------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum Hit {
    None,
    Caption(u32),
    Close(u32),
    Frame(u32),
    MenuBar(u32, usize, i32, i32), // window, index, x and y of the item's base
    Client(u32),
}

fn deepest(s: &State, parent: u32, x: i32, y: i32) -> u32 {
    let mut kids: Vec<u32> = s.windows.iter().filter(|(_, w)| w.parent == parent && w.style & WS_CHILD != 0 && w.visible).map(|(&h, _)| h).collect();
    kids.sort_by(|a, b| b.cmp(a)); // created later stay on top
    for k in kids {
        let (ox, oy) = client_origin(s, k);
        let c = &s.windows[&k];
        if x >= ox && y >= oy && x < ox + c.w && y < oy + c.h {
            return deepest(s, k, x, y);
        }
    }
    parent
}

fn hit(s: &State, x: i32, y: i32) -> Hit {
    let mut tops: Vec<(&u32, &Window)> = s.windows.iter().filter(|(_, w)| w.style & WS_CHILD == 0 && w.visible).collect();
    tops.sort_by_key(|(_, w)| std::cmp::Reverse(w.z));
    for (&h, w) in tops {
        if x < w.x || y < w.y || x >= w.x + w.w || y >= w.y + w.h {
            continue;
        }
        if w.style & WS_CAPTION == WS_CAPTION {
            let cap_top = w.y + BORDER - 1;
            if y >= cap_top && y < cap_top + CAPTION_H - 1 && x >= w.x + BORDER && x < w.x + w.w - BORDER {
                return if x >= w.x + w.w - BORDER - 18 { Hit::Close(h) } else { Hit::Caption(h) };
            }
            if w.menu != 0 {
                let by = w.y + BORDER + CAPTION_H - 1;
                if y >= by && y < by + MENU_H {
                    for (i, (x0, x1, _)) in bar_layout(w, w.x + BORDER).iter().enumerate() {
                        if x >= *x0 && x < *x1 {
                            return Hit::MenuBar(h, i, *x0, by + MENU_H);
                        }
                    }
                    return Hit::Frame(h);
                }
            }
        }
        let (ox, oy) = client_origin(s, h);
        if x >= ox && y >= oy && x < ox + w.cw && y < oy + w.ch {
            return Hit::Client(deepest(s, h, x, y));
        }
        return Hit::Frame(h);
    }
    Hit::None
}

fn top_of(s: &State, mut h: u32) -> u32 {
    while let Some(w) = s.windows.get(&h) {
        if w.style & WS_CHILD == 0 || w.parent == 0 {
            return h;
        }
        h = w.parent;
    }
    h
}

fn activate(s: &mut State, top: u32) {
    s.z_next += 1;
    let z = s.z_next;
    if let Some(w) = s.windows.get_mut(&top) {
        w.z = z;
    }
    s.active = top;
    // "Owned" windows (popups whose parent is `top`) always stay above the owner, in their existing order.
    let mut owned: Vec<(u64, u32)> = s.windows.iter().filter(|(_, w)| w.parent == top && w.style & WS_CHILD == 0).map(|(&h, w)| (w.z, h)).collect();
    owned.sort();
    for (_, h) in owned {
        let prev = s.active;
        activate(s, h);
        s.active = prev;
    }
}

// ---- input messages -----------------------------------------------------------------------

fn mk_flags(s: &State) -> u32 {
    let mut f = BUTTONS.load(Ordering::Relaxed);
    if s.keys[VK_SHIFT as usize] { f |= MK_SHIFT }
    if s.keys[VK_CONTROL as usize] { f |= MK_CONTROL }
    f
}

fn post(s: &mut State, hwnd: u32, message: u32, wparam: u32, lparam: u32) {
    s.queue.push_back(Msg { hwnd, message, wparam, lparam });
}

fn mouse_lparam(s: &State, hwnd: u32, x: i32, y: i32) -> u32 {
    let (ox, oy) = client_origin(s, hwnd);
    (((y - oy) as u32 & 0xFFFF) << 16) | ((x - ox) as u32 & 0xFFFF)
}

/// Window that receives the mouse: the captured one, or the one under the point.
fn mouse_target(s: &State, x: i32, y: i32) -> Option<u32> {
    if s.capture != 0 && s.windows.contains_key(&s.capture) {
        return Some(s.capture);
    }
    match hit(s, x, y) {
        Hit::Client(h) => Some(h),
        _ => None,
    }
}

fn open_popup(s: &mut State, owner: u32, bar_menu: u32, item_menu: u32, x: i32, y: i32) {
    s.popup = Some(Popup { owner, bar_menu, levels: vec![Level { menu: item_menu, x, y, hover: None }] });
    touch();
}

/// Messages the menu owner receives when opening it (so the program can enable/check items).
fn init_menu_msgs(owner: u32, bar: u32, popup: u32, idx: usize) {
    send(owner, WM_INITMENU, bar, 0);
    send(owner, WM_INITMENUPOPUP, popup, idx as u32);
}

fn popup_move(s: &mut State, x: i32, y: i32) -> bool {
    let Some(p) = s.popup.as_mut() else { return false };
    // innermost level containing the point
    for li in (0..p.levels.len()).rev() {
        let (w, h) = level_size(p.levels[li].menu);
        let (lx, ly, lmenu) = (p.levels[li].x, p.levels[li].y, p.levels[li].menu);
        if x >= lx && x < lx + w && y >= ly && y < ly + h {
            let hover = level_item_at(lmenu, y - ly);
            let items = menu::items(lmenu);
            p.levels.truncate(li + 1);
            p.levels[li].hover = hover.map(|h| h.0);
            if let Some((i, top)) = hover {
                if items[i].sub != 0 {
                    p.levels.push(Level { menu: items[i].sub, x: lx + w - 2, y: ly + top, hover: None });
                }
            }
            touch();
            return true;
        }
    }
    false
}

/// Click with the popup open. Returns true if the click was consumed.
fn popup_click(s: &mut State, x: i32, y: i32) -> bool {
    let Some(p) = s.popup.as_ref() else { return false };
    let owner = p.owner;
    for lv in p.levels.iter().rev() {
        let (w, h) = level_size(lv.menu);
        if x >= lv.x && x < lv.x + w && y >= lv.y && y < lv.y + h {
            if let Some((i, _)) = level_item_at(lv.menu, y - lv.y) {
                let it = menu::items(lv.menu)[i].clone();
                if it.sub == 0 && it.flags & (menu::MF_GRAYED | menu::MF_DISABLED | menu::MF_SEPARATOR) == 0 {
                    s.popup = None;
                    post(s, owner, WM_COMMAND, it.id & 0xFFFF, 0);
                }
            }
            touch();
            return true;
        }
    }
    s.popup = None; // clicked outside: closes; the click goes on to normal handling
    touch();
    false
}

// ---- event processing ---------------------------------------------------------------------

/// Drains the event queue generating messages and fires expired timers.
pub(super) fn pump() {
    loop {
        if WAITS.load(Ordering::Relaxed) > 0 {
            break;
        }
        let Some(ev) = EVENTS.lock().unwrap().pop_front() else { break };
        handle(ev);
    }
    fire_timers();
    super::super::winmm::poll(); // completed waveOut buffers and timeSetEvent
}

fn handle(ev: Ev) {
    touch();
    match ev {
        Ev::Wait(n) => WAITS.store(n, Ordering::Relaxed),
        Ev::Size(w, h) => super::request_screen(w as usize, h as usize),
        Ev::Shot(n) => {
            if let Ok(base) = std::env::var("WINLAYER_SHOT") {
                let stem = base.strip_suffix(".png").unwrap_or(&base);
                dump_screenshot(&format!("{stem}-{n}.png"));
            }
        }
        Ev::Move(x, y) => {
            CUR_X.store(x, Ordering::Relaxed);
            CUR_Y.store(y, Ordering::Relaxed);
            let mut s = STATE.lock().unwrap();
            if s.resize.is_some() {
                return; // the size only changes when the button is released
            }
            if let Some((h, dx, dy)) = s.drag {
                if let Some(w) = s.windows.get_mut(&h) {
                    w.x = x - dx;
                    w.y = y - dy;
                }
                return;
            }
            if popup_move(&mut s, x, y) {
                return;
            }
            if let Some(t) = mouse_target(&s, x, y) {
                let lp = mouse_lparam(&s, t, x, y);
                let f = mk_flags(&s);
                // merges consecutive moves
                if let Some(last) = s.queue.back_mut() {
                    if last.hwnd == t && last.message == WM_MOUSEMOVE {
                        *last = Msg { hwnd: t, message: WM_MOUSEMOVE, wparam: f, lparam: lp };
                        return;
                    }
                }
                post(&mut s, t, WM_MOUSEMOVE, f, lp);
            }
        }
        Ev::Down(b) => on_button(b, true),
        Ev::Up(b) => on_button(b, false),
        Ev::KeyDown(vk, sc) => on_key(vk, true, sc),
        Ev::KeyUp(vk, sc) => on_key(vk, false, sc),
        Ev::Char(c) => {
            let (target, _) = {
                let s = STATE.lock().unwrap();
                (if s.focus != 0 { s.focus } else { s.active }, 0)
            };
            if target != 0 {
                let mut s = STATE.lock().unwrap();
                post(&mut s, target, WM_CHAR, c, 1);
            }
        }
    }
}

fn button_msgs(b: u32) -> (u32, u32, u32, u32) {
    // (MK mask, DOWN, UP, DBLCLK)
    match b {
        0 => (MK_LBUTTON, 0x201, 0x202, 0x203),
        2 => (MK_RBUTTON, 0x204, 0x205, 0x206),
        _ => (MK_MBUTTON, 0x207, 0x208, 0x209),
    }
}

const WS_THICKFRAME: u32 = 0x0004_0000;
/// Within this distance of a corner, the drag moves both sides.
const CORNER: i32 = 16;
const MIN_W: i32 = 120;
const MIN_H: i32 = 80;

/// Frame drag: which sides move and the geometry at the start.
#[derive(Clone, Copy)]
pub(super) struct Resize {
    pub(super) hwnd: u32,
    left: bool,
    right: bool,
    top: bool,
    bottom: bool,
    start: (i32, i32),
    rect: (i32, i32, i32, i32),
}

impl Resize {
    /// `None` if the window is not resizable.
    fn start(s: &State, hwnd: u32, x: i32, y: i32) -> Option<Resize> {
        let w = s.windows.get(&hwnd)?;
        if w.style & WS_THICKFRAME == 0 {
            return None;
        }
        let (dl, dr, dt, db) = (x - w.x, w.x + w.w - x, y - w.y, w.y + w.h - y);
        let near_x = dl < CORNER || dr <= CORNER;
        let near_y = dt < CORNER || db <= CORNER;
        let (left, right) = (dl < BORDER || (near_y && dl < CORNER), dr <= BORDER || (near_y && dr <= CORNER));
        let (top, bottom) = (dt < BORDER || (near_x && dt < CORNER), db <= BORDER || (near_x && db <= CORNER));
        // The empty area of the menu bar also counts as frame, but it is not an edge.
        (left || right || top || bottom).then_some(Resize { hwnd, left, right, top, bottom, start: (x, y), rect: (w.x, w.y, w.w, w.h) })
    }

    /// New geometry with the cursor at (x, y); the opposite side stays still even at minimum size.
    fn geometry(&self, x: i32, y: i32) -> (i32, i32, i32, i32) {
        let (mut l, mut t, w, h) = self.rect;
        let (dx, dy) = (x - self.start.0, y - self.start.1);
        let (r, b) = (l + w, t + h);
        let (mut nw, mut nh) = (w, h);
        if self.right {
            nw = (w + dx).max(MIN_W);
        }
        if self.bottom {
            nh = (h + dy).max(MIN_H);
        }
        if self.left {
            nw = (w - dx).max(MIN_W);
            l = r - nw;
        }
        if self.top {
            nh = (h - dy).max(MIN_H);
            t = b - nh;
        }
        (l, t, nw, nh)
    }
}

fn on_button(b: u32, down: bool) {
    let (mk, m_down, m_up, m_dbl) = button_msgs(b);
    let (x, y) = (CUR_X.load(Ordering::Relaxed), CUR_Y.load(Ordering::Relaxed));
    if down {
        BUTTONS.fetch_or(mk, Ordering::Relaxed);
    } else {
        BUTTONS.fetch_and(!mk, Ordering::Relaxed);
    }
    let mut pending_init: Option<(u32, u32, u32, usize)> = None;
    {
        let mut s = STATE.lock().unwrap();
        if !down {
            // As on Windows of the era, a single WM_SIZE on release: programs recreate bitmaps on each message,
            // and one per mouse move exhausts the memory of 16-bit games.
            if let Some(r) = s.resize.take() {
                drop(s);
                let (nx, ny, nw, nh) = r.geometry(x, y);
                super::MoveWindow(r.hwnd, nx, ny, nw, nh, 1);
                return;
            }
            if s.drag.take().is_some() {
                return;
            }
            if s.popup.is_some() {
                return; // the click was handled in `down`
            }
            if let Some(t) = mouse_target(&s, x, y) {
                let lp = mouse_lparam(&s, t, x, y);
                let f = mk_flags(&s);
                post(&mut s, t, m_up, f, lp);
            }
            return;
        }
        if popup_click(&mut s, x, y) {
            return;
        }
        // with capture active the click goes straight to the captured window
        if s.capture != 0 {
            if let Some(t) = mouse_target(&s, x, y) {
                let lp = mouse_lparam(&s, t, x, y);
                let f = mk_flags(&s);
                post(&mut s, t, m_down, f, lp);
            }
            return;
        }
        if std::env::var_os("WINLAYER_TRACE").is_some() {
            let kind = match hit(&s, x, y) { Hit::None => "nothing".to_string(), Hit::Caption(h) => format!("caption {h:#x}"), Hit::Close(h) => format!("close {h:#x}"), Hit::Frame(h) => format!("frame {h:#x}"), Hit::MenuBar(h, ..) => format!("menu bar {h:#x}"), Hit::Client(h) => format!("client {h:#x}") };
            let tops: Vec<String> = s.windows.iter().filter(|(_, w)| w.style & WS_CHILD == 0).map(|(h, w)| format!("{h:#x}[z={} vis={} {},{} {}x{}]", w.z, w.visible, w.x, w.y, w.w, w.h)).collect();
            eprintln!("[input] click at ({x},{y}) -> {kind}; capture={:#x}; tops: {}", s.capture, tops.join(" "));
        }
        match hit(&s, x, y) {
            Hit::None => {}
            Hit::Frame(h) => {
                let top = top_of(&s, h);
                activate(&mut s, top);
                s.resize = Resize::start(&s, top, x, y);
            }
            Hit::Caption(h) => {
                activate(&mut s, h);
                let w = &s.windows[&h];
                s.drag = Some((h, x - w.x, y - w.y));
            }
            Hit::Close(h) => {
                activate(&mut s, h);
                post(&mut s, h, 0x10, 0, 0); // WM_CLOSE
            }
            Hit::MenuBar(h, i, bx, by) => {
                activate(&mut s, h);
                let w = &s.windows[&h];
                let bar = w.menu;
                let it = menu::items(bar)[i].clone();
                if it.sub != 0 {
                    open_popup(&mut s, h, bar, it.sub, bx, by);
                    pending_init = Some((h, bar, it.sub, i));
                } else if it.flags & (menu::MF_GRAYED | menu::MF_DISABLED) == 0 {
                    post(&mut s, h, WM_COMMAND, it.id & 0xFFFF, 0);
                }
            }
            Hit::Client(t) => {
                let top = top_of(&s, t);
                activate(&mut s, top);
                let info = s.windows.get(&t).map(|w| (w.class.to_ascii_lowercase(), w.enabled, s.classes.get(&w.class).is_some_and(|c| c.raw[0] & CS_DBLCLKS != 0)));
                if let Some((class, enabled, dbl_ok)) = info {
                    if matches!(class.as_str(), "edit" | "button" | "listbox" | "combobox") && enabled {
                        s.focus = t;
                    } else if s.focus == 0 {
                        s.focus = t;
                    }
                    let now = now_ms();
                    let is_dbl = dbl_ok
                        && s.last_click.0 == t
                        && s.last_click.1 == b
                        && now.wrapping_sub(s.last_click.2) < 500
                        && (s.last_click.3 - x).abs() < 4
                        && (s.last_click.4 - y).abs() < 4;
                    s.last_click = (t, b, now, x, y);
                    if enabled {
                        let lp = mouse_lparam(&s, t, x, y);
                        let f = mk_flags(&s);
                        post(&mut s, t, if is_dbl { m_dbl } else { m_down }, f, lp);
                    }
                }
            }
        }
    }
    if let Some((h, bar, sub, i)) = pending_init {
        init_menu_msgs(h, bar, sub, i);
    }
}

fn vk_index(vk: u32) -> usize {
    (vk & 0xFF) as usize
}

/// Scan code (PC/AT set 1) and extended key flag of a virtual key, for when
/// the key sender did not provide it (test scripts, browser). Programs like Civilization use
/// this to tell the numeric keypad arrows apart.
pub fn default_scan(vk: u32) -> (u32, u32) {
    const ROW_Q: &[u8] = b"QWERTYUIOP";
    const ROW_A: &[u8] = b"ASDFGHJKL";
    const ROW_Z: &[u8] = b"ZXCVBNM";
    let c = vk as u8;
    if let Some(i) = ROW_Q.iter().position(|&k| k as u32 == vk) {
        return (0x10 + i as u32, 0);
    }
    if let Some(i) = ROW_A.iter().position(|&k| k as u32 == vk) {
        return (0x1E + i as u32, 0);
    }
    if let Some(i) = ROW_Z.iter().position(|&k| k as u32 == vk) {
        return (0x2C + i as u32, 0);
    }
    match vk {
        0x31..=0x39 => (0x02 + (c - 0x31) as u32, 0),
        0x30 => (0x0B, 0),
        0x70..=0x79 => (0x3B + (vk - 0x70), 0),
        0x7A => (0x57, 0),
        0x7B => (0x58, 0),
        0x60 => (0x52, 0),
        0x61 => (0x4F, 0),
        0x62 => (0x50, 0),
        0x63 => (0x51, 0),
        0x64 => (0x4B, 0),
        0x65 => (0x4C, 0),
        0x66 => (0x4D, 0),
        0x67 => (0x47, 0),
        0x68 => (0x48, 0),
        0x69 => (0x49, 0),
        0x6A => (0x37, 0),
        0x6B => (0x4E, 0),
        0x6D => (0x4A, 0),
        0x6E => (0x53, 0),
        0x6F => (0x35, 1),
        0x25 => (0x4B, 1),
        0x26 => (0x48, 1),
        0x27 => (0x4D, 1),
        0x28 => (0x50, 1),
        0x24 => (0x47, 1),
        0x23 => (0x4F, 1),
        0x21 => (0x49, 1),
        0x22 => (0x51, 1),
        0x2D => (0x52, 1),
        0x2E => (0x53, 1),
        0x0D => (0x1C, 0),
        0x1B => (0x01, 0),
        0x20 => (0x39, 0),
        0x09 => (0x0F, 0),
        0x08 => (0x0E, 0),
        0x10 => (0x2A, 0),
        0x11 => (0x1D, 0),
        0x12 => (0x38, 0),
        _ => (0, 0),
    }
}

fn on_key(vk: u32, down: bool, explicit: u32) {
    let mut dialog_action: Option<(u32, u32)> = None; // (dialog, command id)
    {
        let mut s = STATE.lock().unwrap();
        let was = s.keys[vk_index(vk)];
        s.keys[vk_index(vk)] = down;
        let target = if s.focus != 0 && s.windows.contains_key(&s.focus) { s.focus } else { s.active };
        if target == 0 {
            return;
        }
        if down && s.popup.is_some() && vk == VK_ESCAPE {
            s.popup = None;
            return;
        }
        // Dialog keys: Enter/Esc/Tab, handled before delivering the key.
        if down {
            let top = top_of(&s, target);
            let is_dialog = s.windows.get(&top).is_some_and(|w| w.class == DIALOG_CLASS);
            if is_dialog {
                match vk {
                    VK_RETURN => {
                        let def = s.windows.iter().find(|(_, w)| w.parent == top && w.class.eq_ignore_ascii_case("button") && w.style & 0xF == 1).map(|(_, w)| w.id);
                        dialog_action = Some((top, def.unwrap_or(1)));
                    }
                    VK_ESCAPE => dialog_action = Some((top, 2)),
                    VK_TAB => {
                        let mut tabs: Vec<u32> = s.windows.iter().filter(|(_, w)| w.parent == top && w.style & WS_TABSTOP != 0 && w.enabled && w.visible).map(|(&h, _)| h).collect();
                        tabs.sort();
                        if let Some(pos) = tabs.iter().position(|&h| h == s.focus) {
                            let step = if s.keys[VK_SHIFT as usize] { tabs.len() - 1 } else { 1 };
                            s.focus = tabs[(pos + step) % tabs.len()];
                        } else if let Some(&f) = tabs.first() {
                            s.focus = f;
                        }
                        return;
                    }
                    _ => {}
                }
            }
        }
        if dialog_action.is_none() {
            let (scan, ext) = if explicit & 0x8000_0000 != 0 { (explicit & 0xFF, (explicit >> 8) & 1) } else { default_scan(vk) };
            let base = 1 | (scan << 16) | (ext << 24);
            let (m, lp) = if down {
                (WM_KEYDOWN, base | if was { 1 << 30 } else { 0 })
            } else {
                (WM_KEYUP, base | (1 << 30) | (1 << 31))
            };
            post(&mut s, target, m, vk, lp);
        }
    }
    if let Some((dlg, id)) = dialog_action {
        send(dlg, WM_COMMAND, id & 0xFFFF, 0);
    }
}

// ---- timers -------------------------------------------------------------------------------

pub(super) struct Timer {
    pub hwnd: u32,
    pub id: u32,
    pub interval: u32,
    pub due: u32,
    pub proc_: u32,
}

fn fire_timers() {
    let now = now_ms();
    let mut s = STATE.lock().unwrap();
    let mut fire = Vec::new();
    for t in s.timers.iter_mut() {
        if now.wrapping_sub(t.due) < 0x8000_0000 {
            t.due = now + t.interval;
            fire.push(Msg { hwnd: t.hwnd, message: WM_TIMER, wparam: t.id, lparam: t.proc_ });
        }
    }
    for m in fire {
        // at most one pending WM_TIMER per timer
        if !s.timer_msgs.iter().any(|q| q.hwnd == m.hwnd && q.wparam == m.wparam) {
            s.timer_msgs.push_back(m);
        }
    }
}

// ---- exported API -------------------------------------------------------------------------

pub fn resolve(name: &str) -> Option<usize> {
    exports!(
        name,
        SetTimer, KillTimer, GetCursorPos, SetCursorPos, GetMessagePos, GetMessageTime, GetKeyState,
        GetAsyncKeyState, GetKeyboardState, GetFocus, SetFocus, GetActiveWindow, SetActiveWindow,
        SetCapture, ReleaseCapture, GetCapture, LoadAcceleratorsA, TranslateAcceleratorA,
        BringWindowToTop, SetForegroundWindow, GetForegroundWindow,
    )
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SetTimer(hwnd: u32, id: u32, ms: u32, proc_: u32) -> u32 {
    let mut s = STATE.lock().unwrap();
    let id = if hwnd == 0 && id == 0 {
        s.next_timer += 1;
        0x1000 + s.next_timer
    } else {
        id
    };
    s.timers.retain(|t| !(t.hwnd == hwnd && t.id == id));
    let interval = ms.max(10);
    let due = now_ms() + interval;
    s.timers.push(Timer { hwnd, id, interval, due, proc_ });
    id
}

#[allow(non_snake_case)]
pub extern "stdcall" fn KillTimer(hwnd: u32, id: u32) -> i32 {
    let mut s = STATE.lock().unwrap();
    let n = s.timers.len();
    s.timers.retain(|t| !(t.hwnd == hwnd && t.id == id));
    s.timer_msgs.retain(|m| !(m.hwnd == hwnd && m.wparam == id));
    (s.timers.len() != n) as i32
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn GetCursorPos(pt: *mut i32) -> i32 {
    *pt = CUR_X.load(Ordering::Relaxed);
    *pt.add(1) = CUR_Y.load(Ordering::Relaxed);
    1
}

#[allow(non_snake_case)]
extern "stdcall" fn SetCursorPos(x: i32, y: i32) -> i32 {
    CUR_X.store(x, Ordering::Relaxed);
    CUR_Y.store(y, Ordering::Relaxed);
    1
}

#[allow(non_snake_case)]
extern "stdcall" fn GetMessagePos() -> u32 {
    ((CUR_Y.load(Ordering::Relaxed) as u32 & 0xFFFF) << 16) | (CUR_X.load(Ordering::Relaxed) as u32 & 0xFFFF)
}

#[allow(non_snake_case)]
extern "stdcall" fn GetMessageTime() -> u32 {
    now_ms()
}

fn key_down(vk: u32) -> bool {
    match vk {
        1 => BUTTONS.load(Ordering::Relaxed) & MK_LBUTTON != 0,
        2 => BUTTONS.load(Ordering::Relaxed) & MK_RBUTTON != 0,
        4 => BUTTONS.load(Ordering::Relaxed) & MK_MBUTTON != 0,
        _ => STATE.lock().unwrap().keys[vk_index(vk)],
    }
}

#[allow(non_snake_case)]
extern "stdcall" fn GetKeyState(vk: i32) -> i16 {
    if key_down(vk as u32) { -128 } else { 0 } // bit 15 = pressed
}

#[allow(non_snake_case)]
extern "stdcall" fn GetAsyncKeyState(vk: i32) -> i16 {
    // Unlike GetKeyState this reflects the hardware right now: games spin on it waiting for a key to be released.
    pump();
    GetKeyState(vk)
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn GetKeyboardState(out: *mut u8) -> i32 {
    let s = STATE.lock().unwrap();
    for i in 0..256 {
        *out.add(i) = if s.keys[i] { 0x80 } else { 0 };
    }
    1
}

#[allow(non_snake_case)]
extern "stdcall" fn GetFocus() -> u32 {
    let s = STATE.lock().unwrap();
    if s.windows.contains_key(&s.focus) { s.focus } else { 0 }
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SetFocus(hwnd: u32) -> u32 {
    let old = {
        let mut s = STATE.lock().unwrap();
        if !s.windows.contains_key(&hwnd) {
            return 0;
        }
        std::mem::replace(&mut s.focus, hwnd)
    };
    if old != hwnd {
        if old != 0 { send(old, WM_KILLFOCUS, hwnd, 0); }
        send(hwnd, WM_SETFOCUS, old, 0);
    }
    old
}

#[allow(non_snake_case)]
extern "stdcall" fn GetActiveWindow() -> u32 {
    let s = STATE.lock().unwrap();
    if s.windows.contains_key(&s.active) { s.active } else { 0 }
}

#[allow(non_snake_case)]
extern "stdcall" fn SetActiveWindow(hwnd: u32) -> u32 {
    let mut s = STATE.lock().unwrap();
    if !s.windows.contains_key(&hwnd) {
        return 0;
    }
    let old = s.active;
    let top = top_of(&s, hwnd);
    activate(&mut s, top);
    touch();
    old
}

#[allow(non_snake_case)]
extern "stdcall" fn GetForegroundWindow() -> u32 {
    GetActiveWindow()
}

#[allow(non_snake_case)]
extern "stdcall" fn SetForegroundWindow(hwnd: u32) -> i32 {
    SetActiveWindow(hwnd);
    1
}

#[allow(non_snake_case)]
extern "stdcall" fn BringWindowToTop(hwnd: u32) -> i32 {
    SetActiveWindow(hwnd);
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SetCapture(hwnd: u32) -> u32 {
    std::mem::replace(&mut STATE.lock().unwrap().capture, hwnd)
}

#[allow(non_snake_case)]
pub extern "stdcall" fn ReleaseCapture() -> i32 {
    STATE.lock().unwrap().capture = 0;
    1
}

#[allow(non_snake_case)]
extern "stdcall" fn GetCapture() -> u32 {
    STATE.lock().unwrap().capture
}

// ---- accelerators -------------------------------------------------------------------------

#[allow(non_snake_case)]
unsafe extern "stdcall" fn LoadAcceleratorsA(inst: u32, name: u32) -> u32 {
    let base = if inst == 0 { super::super::kernel32::image_base() } else { inst };
    resources::find(base, &resources::ResId::Id(resources::RT_ACCELERATOR), &resources::res_id(name)).map_or(0, |r| r.1)
}

/// If the keyboard message matches a shortcut in the table, sends WM_COMMAND and returns 1.
#[allow(non_snake_case)]
unsafe extern "stdcall" fn TranslateAcceleratorA(hwnd: u32, table: u32, msg: *const u32) -> i32 {
    if table == 0 {
        return 0;
    }
    let (message, w) = (*msg.add(1), *msg.add(2));
    if message != WM_KEYDOWN && message != WM_CHAR {
        return 0;
    }
    let (ctrl, shift, alt) = {
        let s = STATE.lock().unwrap();
        (s.keys[VK_CONTROL as usize], s.keys[VK_SHIFT as usize], s.keys[VK_MENU as usize])
    };
    let mut p = table;
    loop {
        let flags = *(p as usize as *const u16) as u32;
        let key = *((p + 2) as usize as *const u16) as u32;
        let cmd = *((p + 4) as usize as *const u16) as u32;
        let virt = flags & 1 != 0;
        let mods_ok = (flags & 8 != 0) == ctrl && (flags & 4 != 0) == shift && (flags & 0x10 != 0) == alt;
        let hit = if virt { message == WM_KEYDOWN && key == w && mods_ok } else { message == WM_CHAR && key == w };
        if hit {
            send(hwnd, WM_COMMAND, cmd | (1 << 16), 0);
            return 1;
        }
        if flags & 0x80 != 0 {
            return 0;
        }
        p += 8;
    }
}

/// Virtual key pressed (for the Win16 layer and accelerators).
pub fn input_key_down(vk: i32) -> bool {
    key_down(vk as u32)
}

#[cfg(test)]
mod resize_tests {
    use super::Resize;

    fn r(left: bool, right: bool, top: bool, bottom: bool) -> Resize {
        Resize { hwnd: 1, left, right, top, bottom, start: (0, 0), rect: (100, 100, 300, 200) }
    }

    #[test]
    fn bottom_right_corner_only_changes_size() {
        assert_eq!(r(false, true, false, true).geometry(50, 20), (100, 100, 350, 220));
    }

    #[test]
    fn left_edge_moves_origin_and_respects_minimum() {
        assert_eq!(r(true, false, false, false).geometry(-40, 0), (60, 100, 340, 200));
        assert_eq!(r(true, false, false, false).geometry(1000, 0), (280, 100, 120, 200));
    }
}
