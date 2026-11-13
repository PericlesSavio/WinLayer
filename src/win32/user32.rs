//! user32.dll — windows, classes, message queue. Each window has a pixel buffer for its
//! client area; `gdi32` draws on it and `dump_screenshot` generates a PNG.

mod input;
pub use input::{default_scan, frame_wait, tick_virtual, real_ms, input_key_down, GetCursorPos, KillTimer, ReleaseCapture, SetCapture, SetFocus, SetTimer, frame_version, now_ms, push_events, render_bgrx, render_rgba, set_shot_mode, shot_mode, touch};

use super::draw::{self, Buf};
use super::{call4, fnt, gdi32, listbox, menu, resources, text};
use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;

// messages
const WM_CREATE: u32 = 0x01;
const WM_DESTROY: u32 = 0x02;
const WM_SIZE: u32 = 0x05;
const WM_PAINT: u32 = 0x0F;
const WM_CLOSE: u32 = 0x10;
const WM_QUIT: u32 = 0x12;
const WM_ERASEBKGND: u32 = 0x14;
const WM_NCCREATE: u32 = 0x81;
const WM_DRAWITEM: u32 = 0x2B;
/// Never-painted pixel of a child window (alpha 0): when compositing, shows what is behind it.
pub const TRANSPARENT: u32 = 0x0000_0000;
const WM_COMMAND: u32 = 0x111;
const WM_CHAR: u32 = 0x102;
const WM_LBUTTONDOWN: u32 = 0x201;
const WM_LBUTTONUP: u32 = 0x202;
const WM_LBUTTONDBLCLK: u32 = 0x203;
const WM_INITDIALOG: u32 = 0x110;
pub const WS_TABSTOP: u32 = 0x0001_0000;
pub const DIALOG_CLASS: &str = "#32770";
pub const DLU_X: i32 = 6; // dialog base unit (Win9x default font): 6x13 px per 4x8 units
pub const DLU_Y: i32 = 13;

/// Dialog base unit (x, y): on Win16 it is that of the Windows 3.1 System font (8x16); on Win32, 6x13.
pub fn dlu() -> (i32, i32) {
    if text::line_h() == 16 { (8, 16) } else { (DLU_X, DLU_Y) }
}
const WM_TIMER: u32 = 0x113;

pub const WS_CAPTION: u32 = 0x00C0_0000;
pub const WS_CHILD: u32 = 0x4000_0000;
pub const WS_VISIBLE: u32 = 0x1000_0000;
const CW_USEDEFAULT: i32 = i32::MIN;
const PM_REMOVE: u32 = 1;

/// Virtual desktop size (default 640x480; change with `--screen WxH` before opening windows).
static SCREEN: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new((640 << 16) | 480);

/// New screen size requested by another thread (the X11 window was resized): (width << 16) | height.
/// The program thread applies it, between messages, because resizing sends WM_SIZE to the game code.
static PENDING_SCREEN: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

pub fn request_screen(w: usize, h: usize) {
    PENDING_SCREEN.store(((w.min(0xFFFF) as u32) << 16) | h.min(0xFFFF) as u32, std::sync::atomic::Ordering::Relaxed);
}

/// Applies a screen size change request: changes the virtual desktop and stretches the main window
/// (the largest one with a caption) to the right/bottom edge if it already reached it.
fn apply_pending_screen() {
    let v = PENDING_SCREEN.swap(0, std::sync::atomic::Ordering::Relaxed);
    if v == 0 {
        return;
    }
    if SCREEN_LOCKED.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let (nw, nh) = ((v >> 16) as i32, (v & 0xFFFF) as i32);
    let (ow, oh) = (desktop_w() as i32, desktop_h() as i32);
    if (nw, nh) == (ow, oh) {
        return;
    }
    {
        let _g = STATE.lock().unwrap(); // no drawing in progress during the swap
        set_screen(nw as usize, nh as usize);
    }
    let main = {
        let s = STATE.lock().unwrap();
        s.windows
            .iter()
            .filter(|(_, w)| w.style & WS_CHILD == 0 && w.visible && w.style & WS_CAPTION == WS_CAPTION)
            .max_by_key(|(_, w)| w.w * w.h)
            .map(|(&h, w)| (h, w.x, w.y, w.w, w.h))
    };
    if let Some((h, x, y, w, hh)) = main {
        let to_right = x + w >= ow - 8;
        let to_bottom = y + hh >= oh - 8;
        let (new_w, new_h) = (if to_right { (nw - x).max(200) } else { w }, if to_bottom { (nh - y).max(150) } else { hh });
        if (new_w, new_h) != (w, hh) {
            MoveWindow(h, x, y, new_w, new_h, 1);
        }
    }
    touch();
}

/// Screen size fixed by the program (DirectDraw `SetDisplayMode`): resizing the X11 window
/// then only scales the image, without changing the "resolution" the game sees.
static SCREEN_LOCKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Changes the virtual desktop size (under the `STATE` lock, like `apply_pending_screen`) and fixes it.
pub fn lock_screen(w: usize, h: usize) {
    SCREEN_LOCKED.store(true, std::sync::atomic::Ordering::Relaxed);
    let _g = STATE.lock().unwrap();
    set_screen(w, h);
    drop(_g);
    touch();
}

/// Origin of the client area of `hwnd` in screen coordinates.
pub fn client_screen_origin(hwnd: u32) -> (i32, i32) {
    client_origin(&STATE.lock().unwrap(), hwnd)
}

pub fn set_screen(w: usize, h: usize) {
    SCREEN.store(((w as u32) << 16) | h as u32, std::sync::atomic::Ordering::Relaxed);
}

pub fn desktop_w() -> usize {
    (SCREEN.load(std::sync::atomic::Ordering::Relaxed) >> 16) as usize
}

pub fn desktop_h() -> usize {
    (SCREEN.load(std::sync::atomic::Ordering::Relaxed) & 0xFFFF) as usize
}
pub const CAPTION_H: i32 = 19;
pub const BORDER: i32 = 4;

/// Upward offset (px) of the buttons of Win16 double-frame dialogs.
const BUTTON_NUDGE: i32 = 6;

/// Double frame of a dialog without caption (WS_DLGFRAME): 3 px on each side, outside the client area.
pub fn dlg_frame(style: u32) -> i32 {
    if style & WS_CHILD == 0 && style & WS_CAPTION == 0x40_0000 { 3 } else { 0 }
}
const MENU_H: i32 = 19;

struct Class {
    proc_: u32,
    bg: u32,
    /// lpszMenuName of the WNDCLASS (ordinal or pointer).
    menu: u32,
    /// Full WNDCLASSA, as registered (for GetClassInfoA).
    raw: [u32; 10],
    /// WndProc is a 16-bit far pointer (class registered by a Win16 program).
    is16: bool,
}

struct Window {
    class: String,
    proc_: u32,
    style: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    cw: i32,
    ch: i32,
    parent: u32,
    id: u32,
    /// `proc_`/`dlgproc` are 16-bit far pointers.
    is16: bool,
    /// Stacking order among top-level windows (higher = closer to the front).
    z: u64,
    /// Control state: bit 0 = checked, bit 1 = pressed.
    state: u32,
    enabled: bool,
    /// HMENU of the menu bar (0 = no menu).
    menu: u32,
    longs: BTreeMap<i32, u32>,
    dlgproc: u32,
    ended: Option<i32>,
    title: String,
    visible: bool,
    need_paint: bool,
    need_erase: bool,
    buf: Vec<u32>,
}

#[derive(Clone, Copy)]
struct Msg {
    hwnd: u32,
    message: u32,
    wparam: u32,
    lparam: u32,
}

struct State {
    classes: BTreeMap<String, Class>,
    windows: BTreeMap<u32, Window>,
    queue: VecDeque<Msg>,
    quit: Option<u32>,
    next_hwnd: u32,
    timers: Vec<input::Timer>,
    /// Expired WM_TIMERs: delivered only when no message or paint is pending (lowest priority).
    timer_msgs: VecDeque<Msg>,
    frames: u32,
    props: BTreeMap<(u32, String), u32>,
    z_next: u64,
    focus: u32,
    active: u32,
    capture: u32,
    popup: Option<input::Popup>,
    /// Window being dragged by the title bar: (hwnd, x offset, y).
    drag: Option<(u32, i32, i32)>,
    /// Window being resized by the frame.
    resize: Option<input::Resize>,
    keys: [bool; 256],
    /// Last click: (window, button, time in ms, x, y), to detect double clicks.
    last_click: (u32, u32, u32, i32, i32),
    next_timer: u32,
}

static STATE: Mutex<State> = Mutex::new(State {
    classes: BTreeMap::new(),
    windows: BTreeMap::new(),
    queue: VecDeque::new(),
    quit: None,
    next_hwnd: 0x0100,
    timers: Vec::new(),
    timer_msgs: VecDeque::new(),
    frames: 0,
    props: BTreeMap::new(),
    z_next: 0,
    focus: 0,
    active: 0,
    capture: 0,
    popup: None,
    drag: None,
    resize: None,
    keys: [false; 256],
    last_click: (0, 0, 0, 0, 0),
    next_timer: 0,
});

pub fn resolve(name: &str) -> Option<usize> {
    exports!(
        name, DialogBoxIndirectParamA, RedrawWindow, ChangeDisplaySettingsA, GetKeyNameTextA, MapVirtualKeyA, RegisterClassExA, CreateDialogIndirectParamA, DrawTextExA, ClipCursor, WindowFromDC,
        MessageBoxA, RegisterClassA, CreateWindowExA, DefWindowProcA, ShowWindow, UpdateWindow,
        GetMessageA, PeekMessageA, TranslateMessage, DispatchMessageA, PostMessageA, SendMessageA,
        PostQuitMessage, BeginPaint, EndPaint, InvalidateRect, GetClientRect, MoveWindow,
        GetDC, ReleaseDC, LoadCursorA, LoadIconA, FindWindowA, GetLastActivePopup, IsIconic,
        GetMenu, CheckMenuItem, DialogBoxParamA, EndDialog,
        SendDlgItemMessageA, wsprintfA, wvsprintfA, GetWindowRect, DestroyWindow, SetWindowPos, GetWindowLongA,
        SetWindowLongA, ScreenToClient, ClientToScreen, GetParent, GetWindow, GetDesktopWindow,
        GetClassInfoA, EnableWindow, IsWindowEnabled, CallWindowProcA, ValidateRect, LoadMenuA, DestroyMenu, SetMenu, GetMenu, EnableMenuItem,
        CheckMenuItem, ModifyMenuA, GetSubMenu, GetMenuItemCount, GetMenuItemID, AdjustWindowRect,
        AdjustWindowRectEx, SetPropA, GetPropA, RemovePropA, SetWindowsHookExA, UnhookWindowsHookEx,
        CallNextHookEx, OemToCharA, GrayStringA, TabbedTextOutA, IsWindowVisible,
        IsWindow, IsChild, GetWindowTextLengthA, GetDlgCtrlID, CopyRect,
        IntersectRect, CharNextA, CharPrevA, CharLowerA, CharUpperA, CharUpperBuffA, CharUpperBuffW,
        CharLowerBuffA, CharLowerBuffW, GetWindowDC, SetRect, CharToOemA, CharToOemBuffA, IsRectEmpty,
        CreatePopupMenu, CreateMenu, AppendMenuA, OffsetRect, PtInRect, SetParent, FrameRect, GetWindowThreadProcessId, WaitMessage, GetTopWindow, GetDialogBaseUnits, InflateRect, GetClassNameA, IsDialogMessageA, CreateDialogParamA, GetDlgItem,
        SetDlgItemTextA, GetDlgItemTextA, SetDlgItemInt, SetWindowTextA, GetWindowTextA, LoadStringA,
        FillRect, LoadBitmapA, GetSysColor, GetSystemMetrics, DrawTextA, MessageBeep, SetCursor,
        ShowCursor, WinHelpA,
    )
    .or_else(|| input::resolve(name))
}

pub unsafe fn cstr(p: u32) -> String {
    if p == 0 {
        String::new()
    } else if p < 0x10000 {
        format!("#atom{p}") // class atom
    } else {
        std::ffi::CStr::from_ptr(p as usize as *const i8).to_string_lossy().into_owned()
    }
}

fn trace() -> bool {
    std::env::var_os("WINLAYER_TRACE").is_some()
}

fn proc_of(hwnd: u32) -> u32 {
    STATE.lock().unwrap().windows.get(&hwnd).map_or(0, |w| w.proc_)
}

/// Sends the message directly to the WndProc (bypassing the queue).
pub fn send(hwnd: u32, msg: u32, w: u32, l: u32) -> u32 {
    let (p, dp, is16) = STATE.lock().unwrap().windows.get(&hwnd).map_or((0, 0, false), |win| (win.proc_, win.dlgproc, win.is16));
    if trace() { eprintln!("[user32] send hwnd={hwnd:#x} msg={msg:#x} w={w:#x} l={l:#x} proc={p:#x} dlgproc={dp:#x}") }
    if p != 0 {
        // Class with its own procedure (including a "#32770" registered by the program): it is the one that
        // calls DefDlgProc, which in turn invokes the DLGPROC.
        return call_proc(is16, p, hwnd, msg, w, l);
    }
    if dp != 0 {
        return def_dlg_proc(hwnd, msg, w, l);
    }
    builtin_proc(hwnd, msg, w, l)
}

/// DefDlgProc: the dialog's DLGPROC sees the message first; if it returns 0 (not handled), the
/// system defaults apply.
pub fn def_dlg_proc(hwnd: u32, msg: u32, w: u32, l: u32) -> u32 {
    let (dp, is16) = STATE.lock().unwrap().windows.get(&hwnd).map_or((0, false), |win| (win.dlgproc, win.is16));
    if dp != 0 {
        let mut r = call_proc(is16, dp, hwnd, msg, w, l);
        if is16 {
            r &= 0xFFFF; // DlgProc returns BOOL (AX only); DX is garbage
        }
        if r != 0 || msg == WM_INITDIALOG {
            return r;
        }
    }
    builtin_proc(hwnd, msg, w, l)
}

/// Calls a WndProc/DlgProc: 32-bit directly; 16-bit through the Win16 layer hook.
fn call_proc(is16: bool, f: u32, hwnd: u32, msg: u32, w: u32, l: u32) -> u32 {
    if is16 {
        let hook = HOOK16.lock().unwrap().call;
        return hook.map_or(0, |h| h(f, hwnd, msg, w, l));
    }
    unsafe { call4(f, hwnd, msg, w, l) }
}

/// Hooks registered by the Win16 layer: calling 16-bit procedures and loading NE menus.
pub struct Hooks {
    pub call: Option<fn(u32, u32, u32, u32, u32) -> u32>,
    pub menu: Option<fn(u32) -> u32>,
}

pub static HOOK16: Mutex<Hooks> = Mutex::new(Hooks { call: None, menu: None });
/// When true, dialog windows created with `dlgproc` use a 16-bit procedure.
pub static DLG16: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_win16_hooks(call: fn(u32, u32, u32, u32, u32) -> u32, menu: fn(u32) -> u32) {
    let mut h = HOOK16.lock().unwrap();
    h.call = Some(call);
    h.menu = Some(menu);
}

/// Registers a window class of a Win16 program.
pub fn register_class16(name: String, proc16: u32, style: u32, bg: u32, menu_name: u32) {
    let mut raw = [0u32; 10];
    raw[0] = style;
    raw[1] = proc16;
    raw[7] = bg;
    raw[8] = menu_name;
    STATE.lock().unwrap().classes.insert(name, Class { proc_: proc16, bg, menu: menu_name, raw, is16: true });
}

/// Native classes (dialog, button, static, edit...) without a program WndProc.
fn builtin_proc(hwnd: u32, msg: u32, w: u32, l: u32) -> u32 {
    let (class, style, id, parent, cw, ch, title, state, focused) = {
        let s = STATE.lock().unwrap();
        match s.windows.get(&hwnd) {
            Some(win) => (win.class.to_ascii_lowercase(), win.style, win.id, win.parent, win.cw, win.ch, win.title.clone(), win.state, s.focus == hwnd),
            None => return 0,
        }
    };
    if class == "listbox" {
        if let Some(r) = listbox::message(hwnd, style, cw, ch, msg, w, l) {
            set_state_bit(hwnd, 0, false); // only requests a repaint
            return r;
        }
        if msg == WM_LBUTTONDOWN || msg == WM_LBUTTONDBLCLK {
            let (x, y) = ((l & 0xFFFF) as i16 as i32, (l >> 16) as i16 as i32);
            if let Some(changed) = listbox::click(hwnd, style, cw, ch, x, y) {
                set_state_bit(hwnd, 0, false);
                if style & listbox::LBS_NOTIFY != 0 {
                    let code = if msg == WM_LBUTTONDBLCLK { 2 } else if changed { 1 } else { 0 };
                    if code != 0 {
                        send(parent, WM_COMMAND, (code << 16) | id, hwnd);
                    }
                }
            }
            return 0;
        }
    }
    // Messages that are not painting.
    match (class.as_str(), msg) {
        ("button", WM_LBUTTONDOWN) | ("button", WM_LBUTTONDBLCLK) => {
            button_down(hwnd);
            return 0;
        }
        ("button", WM_LBUTTONUP) => {
            button_up(hwnd, l);
            return 0;
        }
        ("button", 0xF0) => return state & 1, // BM_GETCHECK
        ("button", 0xF1) => {
            set_state_bit(hwnd, 1, w != 0); // BM_SETCHECK
            return 0;
        }
        ("button", 0xF2) => return state & 3, // BM_GETSTATE
        ("button", 0xF3) => {
            set_state_bit(hwnd, 2, w != 0); // BM_SETSTATE
            return 0;
        }
        ("edit", WM_CHAR) => {
            edit_char(hwnd, w);
            return 0;
        }
        (DIALOG_CLASS, 0x10) => {
            send(hwnd, WM_COMMAND, 2, 0); // WM_CLOSE on a dialog = IDCANCEL
            return 0;
        }
        _ => {}
    }
    if msg == WM_ERASEBKGND && class == DIALOG_CLASS {
        with_buf(hwnd, |px, _, _| px.fill(draw::GRAY));
        return 1;
    }
    if msg != WM_PAINT || !["#32770", "button", "static", "edit", "listbox", "combobox", "scrollbar"].contains(&class.as_str()) {
        return DefWindowProcA(hwnd, msg, w, l);
    }
    if let Some(win) = STATE.lock().unwrap().windows.get_mut(&hwnd) {
        win.need_paint = false;
    }
    let label = text::strip_mnemonic(&title);
    let text_y = (ch - text::LINE_H) / 2;
    let (checked, pressed) = (state & 1 != 0, state & 2 != 0);
    match class.as_str() {
        DIALOG_CLASS => {
            // 16-bit window: the background was already erased in ShowWindow and the game may have drawn over it.
            let is16 = STATE.lock().unwrap().windows.get(&hwnd).is_some_and(|w| w.is16);
            if !is16 {
                with_buf(hwnd, |px, _, _| px.fill(draw::GRAY));
            }
        }
        "button" => {
            let kind = style & 0xF;
            if kind == 0xB {
                // BS_OWNERDRAW: the parent draws via WM_DRAWITEM (DRAWITEMSTRUCT).
                let hdc = gdi32::dc_for_window(hwnd);
                let item_state = (pressed as u32) | if focused { 0x10 } else { 0 }; // ODS_SELECTED | ODS_FOCUS
                let dis: [u32; 12] = [4, id, 0, 1, item_state, hwnd, hdc, 0, 0, cw as u32, ch as u32, 0];
                send(parent, WM_DRAWITEM, id, dis.as_ptr() as u32);
                gdi32::release_dc(hdc);
                return 0;
            }
            with_buf(hwnd, |px, w, h| {
                let mut b = Buf { px, w, h };
                b.fill(0, 0, w, h, draw::GRAY);
                let ty = text_y.max(0);
                match kind {
                    0 | 1 => {
                        if pressed { b.sunken(0, 0, w, h) } else { b.raised(0, 0, w, h) }
                        if kind == 1 && !pressed { b.frame(0, 0, w, h, draw::BLACK, draw::BLACK) } // default button
                        let tw = text::width(false, &label);
                        let o = pressed as i32;
                        b.text((w - tw) / 2 + o, text_y + o, &label, draw::BLACK, false);
                    }
                    2 | 3 | 5 => {
                        b.sunken(0, ty, 13, ty + 13);
                        b.fill(2, ty + 2, 11, ty + 11, draw::WHITE);
                        if checked {
                            for i in 0..3 {
                                b.fill(3 + i, ty + 6 + i, 4 + i, ty + 9 + i, draw::BLACK);
                            }
                            for i in 0..5 {
                                b.fill(6 + i, ty + 8 - i, 7 + i, ty + 11 - i, draw::BLACK);
                            }
                        }
                        b.text(19, text_y, &label, draw::BLACK, false);
                    }
                    4 | 9 => {
                        b.sunken(0, ty, 12, ty + 12);
                        b.fill(2, ty + 2, 10, ty + 10, draw::WHITE);
                        if checked {
                            b.fill(4, ty + 4, 8, ty + 8, draw::BLACK);
                        }
                        b.text(19, text_y, &label, draw::BLACK, false);
                    }
                    7 => {
                        b.frame(0, 6, w, h, draw::DARK, draw::WHITE);
                        b.frame(1, 7, w - 1, h - 1, draw::WHITE, draw::DARK);
                        let tw = text::width(false, &label);
                        b.fill(6, 0, 10 + tw, text::LINE_H, draw::GRAY);
                        b.text(8, 0, &label, draw::BLACK, false);
                    }
                    _ => {}
                }
            });
        }
        "static" => {
            with_buf(hwnd, |px, w, h| {
                let mut b = Buf { px, w, h };
                b.fill(0, 0, w, h, draw::GRAY);
                if style & 0x1F <= 2 {
                    let tw = text::width(false, &label);
                    let x = match style & 0x1F { 1 => (w - tw) / 2, 2 => w - tw, _ => 0 };
                    b.text(x, 0, &label, draw::BLACK, false);
                }
            });
        }
        "listbox" => {
            with_buf(hwnd, |px, w, h| {
                let mut b = Buf { px, w, h };
                listbox::paint(hwnd, style, &mut b, focused);
            });
        }
        "edit" | "combobox" => {
            with_buf(hwnd, |px, w, h| {
                let mut b = Buf { px, w, h };
                b.fill(0, 0, w, h, draw::WHITE);
                b.sunken(0, 0, w, h);
                let tw = b.text(4, text_y.max(2), &label, draw::BLACK, false);
                if focused && class == "edit" {
                    b.fill(4 + tw, 3, 5 + tw, h - 3, draw::BLACK); // text cursor
                }
            });
        }
        _ => {
            with_buf(hwnd, |px, _, _| px.fill(draw::GRAY));
        }
    }
    0
}

fn set_state_bit(hwnd: u32, bit: u32, on: bool) {
    if let Some(w) = STATE.lock().unwrap().windows.get_mut(&hwnd) {
        w.state = if on { w.state | bit } else { w.state & !bit };
        w.need_paint = true;
    }
    touch();
}

fn button_down(hwnd: u32) {
    {
        let mut s = STATE.lock().unwrap();
        let Some(w) = s.windows.get_mut(&hwnd) else { return };
        if !w.enabled {
            return;
        }
        w.state |= 2;
        w.need_paint = true;
        s.capture = hwnd;
    }
    touch();
}

/// Button release: if inside the control, toggles checks and notifies the parent (BN_CLICKED).
fn button_up(hwnd: u32, lparam: u32) {
    let (x, y) = ((lparam & 0xFFFF) as i16 as i32, (lparam >> 16) as i16 as i32);
    let mut notify = None;
    {
        let mut s = STATE.lock().unwrap();
        if s.capture == hwnd {
            s.capture = 0;
        }
        let Some(w) = s.windows.get_mut(&hwnd) else { return };
        let was_pressed = w.state & 2 != 0;
        w.state &= !2;
        w.need_paint = true;
        if was_pressed && x >= 0 && y >= 0 && x < w.cw && y < w.ch {
            let (kind, id, parent) = (w.style & 0xF, w.id, w.parent);
            match kind {
                3 | 5 => w.state ^= 1, // auto checkbox / 3-state
                9 => w.state |= 1,     // auto radio
                _ => {}
            }
            notify = Some((parent, id, kind));
        }
        if let Some((parent, _, 9)) = notify {
            // unchecks the other radios of the same parent
            let others: Vec<u32> = s.windows.iter()
                .filter(|(&h, o)| h != hwnd && o.parent == parent && o.class.eq_ignore_ascii_case("button") && matches!(o.style & 0xF, 4 | 9))
                .map(|(&h, _)| h)
                .collect();
            for h in others {
                if let Some(o) = s.windows.get_mut(&h) {
                    o.state &= !1;
                    o.need_paint = true;
                }
            }
        }
    }
    touch();
    if let Some((parent, id, _)) = notify {
        send(parent, WM_COMMAND, id & 0xFFFF, hwnd); // BN_CLICKED = 0
    }
}

fn edit_char(hwnd: u32, ch: u32) {
    if let Some(w) = STATE.lock().unwrap().windows.get_mut(&hwnd) {
        match ch {
            8 => { w.title.pop(); }
            c if c >= 32 => w.title.push(char::from_u32(c).unwrap_or('?')),
            _ => {}
        }
        w.need_paint = true;
    }
    touch();
}

/// Access to the client-area pixel buffer of a window.
pub fn with_buf<R>(hwnd: u32, f: impl FnOnce(&mut [u32], i32, i32) -> R) -> Option<R> {
    touch();
    let mut s = STATE.lock().unwrap();
    let win = s.windows.get_mut(&hwnd)?;
    let (cw, ch) = (win.cw, win.ch);
    Some(f(&mut win.buf, cw, ch))
}

fn fill_background(hwnd: u32) {
    let (bg, ) = {
        let s = STATE.lock().unwrap();
        let Some(w) = s.windows.get(&hwnd) else { return };
        (s.classes.get(&w.class).map_or(0, |c| c.bg),)
    };
    let color = match bg {
        0 => return,
        1..=31 => gdi32::sys_color(bg - 1),
        h => match gdi32::brush_color(h) {
            Some(c) => c,
            None => return,
        },
    };
    with_buf(hwnd, |px, _, _| px.fill(gdi32::colorref_to_px(color)));
}

// ---- classes and windows ------------------------------------------------------------------

/// WNDCLASSEXA is WNDCLASSA preceded by cbSize (and followed by hIconSm).
#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn RegisterClassExA(wc: *const u32) -> u32 {
    RegisterClassA(wc.add(1)) as u32
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn RegisterClassA(wc: *const u32) -> u32 {
    // WNDCLASSA: style, lpfnWndProc, cbClsExtra, cbWndExtra, hInstance, hIcon, hCursor,
    //            hbrBackground, lpszMenuName, lpszClassName
    let name = cstr(*wc.add(9));
    if trace() { eprintln!("[user32] RegisterClass {name:?} proc={:#x} bg={:#x}", *wc.add(1), *wc.add(7)) }
    let mut s = STATE.lock().unwrap();
    s.classes.insert(name, Class { proc_: *wc.add(1), bg: *wc.add(7), menu: *wc.add(8), raw: std::array::from_fn(|i| *wc.add(i)), is16: false });
    1
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn CreateWindowExA(
    _ex: u32, class: u32, title: u32, style: u32, x: i32, y: i32, w: i32, h: i32,
    parent: u32, menu: u32, inst: u32, param: u32,
) -> u32 {
    create_window(cstr(class), cstr(title), style, x, y, w, h, parent, menu, inst, param, 0)
}

/// `dlgproc != 0` creates a dialog window (no WM_CREATE; WM_INITDIALOG comes later).
#[allow(clippy::too_many_arguments)]
pub unsafe fn create_window(
    class: String, title: String, style: u32, x: i32, y: i32, w: i32, h: i32,
    parent: u32, menu: u32, inst: u32, param: u32, dlgproc: u32,
) -> u32 {
    if trace() { eprintln!("[user32] CreateWindowEx class={class:?} title={title:?} style={style:#x} {x},{y} {w}x{h}") }
    // Top-level windows: `menu` is an HMENU; without it the class menu applies.
    let hmenu = if style & WS_CHILD != 0 || dlgproc != 0 {
        0
    } else if menu != 0 && menu::exists(menu) {
        menu
    } else {
        let (cm, c16) = STATE.lock().unwrap().classes.get(&class).map_or((0, false), |c| (c.menu, c.is16));
        if cm == 0 {
            0
        } else if c16 {
            let h = HOOK16.lock().unwrap().menu;
            h.map_or(0, |f| f(cm))
        } else {
            menu::load(super::kernel32::image_base(), &resources::res_id(cm))
        }
    };
    let (hwnd, cs);
    {
        let mut s = STATE.lock().unwrap();
        let proc_ = s.classes.get(&class).map_or(0, |c| c.proc_);
        let is16 = s.classes.get(&class).is_some_and(|c| c.is16) || (dlgproc != 0 && DLG16.load(std::sync::atomic::Ordering::Relaxed));
        let n = s.windows.len() as i32;
        let (x, y) = if x == CW_USEDEFAULT { (100 + 30 * n, 80 + 30 * n) } else { (x, y) };
        let (w, h) = if w == CW_USEDEFAULT { (400, 300) } else { (w, h) };
        let (cw, ch) = if style & WS_CAPTION == WS_CAPTION {
            (w - 2 * BORDER, h - CAPTION_H - 2 * BORDER - if hmenu != 0 { MENU_H } else { 0 })
        } else {
            (w - 2 * dlg_frame(style), h - 2 * dlg_frame(style))
        };
        let (cw, ch) = (cw.max(0), ch.max(0));
        hwnd = s.next_hwnd;
        s.next_hwnd += 4;
        s.z_next += 1;
        let z = s.z_next;
        s.windows.insert(hwnd, Window {
            class, proc_, style, x, y, w, h, cw, ch, parent, id: menu, is16, z, state: 0, enabled: true, menu: hmenu, longs: BTreeMap::new(), dlgproc, ended: None, title,
            visible: style & WS_VISIBLE != 0,
            need_paint: style & WS_VISIBLE != 0,
            need_erase: true,
            buf: vec![if style & WS_CHILD != 0 { TRANSPARENT } else { 0xFFFF_FFFF }; (cw * ch) as usize],
        });
        if style & WS_CHILD == 0 {
            s.active = hwnd;
            if s.focus == 0 || !s.windows.contains_key(&s.focus) {
                s.focus = hwnd;
            }
        }
        touch();
        // CREATESTRUCT
        cs = Box::leak(Box::new([param, inst, menu, parent, h as u32, w as u32, y as u32, x as u32, style, 0, 0, 0u32]));
    }
    if dlgproc != 0 {
        return hwnd;
    }
    let csp = cs.as_ptr() as u32;
    if send(hwnd, WM_NCCREATE, 0, csp) == 0 && proc_of(hwnd) != 0 {
        destroy(hwnd);
        return 0;
    }
    if send(hwnd, WM_CREATE, 0, csp) == u32::MAX {
        destroy(hwnd);
        return 0;
    }
    let (cw, ch) = with_buf(hwnd, |_, w, h| (w, h)).unwrap_or((0, 0));
    send(hwnd, WM_SIZE, 0, (cw as u32 & 0xFFFF) | ((ch as u32) << 16));
    hwnd
}

pub fn destroy(hwnd: u32) {
    let children: Vec<u32> = STATE.lock().unwrap().windows.iter().filter(|(_, w)| w.parent == hwnd).map(|(&h, _)| h).collect();
    for c in children {
        destroy(c);
    }
    send(hwnd, WM_DESTROY, 0, 0);
    let mut s = STATE.lock().unwrap();
    let parent = s.windows.get(&hwnd).map_or(0, |w| w.parent);
    s.windows.remove(&hwnd);
    listbox::remove(hwnd);
    if s.focus == hwnd { s.focus = if s.windows.contains_key(&parent) { parent } else { 0 } }
    if s.active == hwnd { s.active = s.windows.iter().filter(|(_, w)| w.style & WS_CHILD == 0 && w.visible).max_by_key(|(_, w)| w.z).map_or(0, |(&h, _)| h) }
    if s.capture == hwnd { s.capture = 0 }
    if s.popup.as_ref().is_some_and(|p| p.owner == hwnd) { s.popup = None }
    if s.drag.is_some_and(|d| d.0 == hwnd) { s.drag = None }
    if s.resize.is_some_and(|r| r.hwnd == hwnd) { s.resize = None }
    drop(s);
    touch();
}

#[allow(non_snake_case)]
pub extern "stdcall" fn DefWindowProcA(hwnd: u32, msg: u32, _w: u32, _l: u32) -> u32 {
    match msg {
        WM_NCCREATE => 1,
        0x0C => {
            // WM_SETTEXT
            set_text(hwnd, unsafe { cstr(_l) });
            1
        }
        0x0D => unsafe { copy_title(hwnd, _l as usize as *mut u8, _w as i32) as u32 }, // WM_GETTEXT
        0x0E => STATE.lock().unwrap().windows.get(&hwnd).map_or(0, |w| w.title.len() as u32), // WM_GETTEXTLENGTH
        WM_PAINT => {
            if let Some(w) = STATE.lock().unwrap().windows.get_mut(&hwnd) {
                w.need_paint = false;
            }
            0
        }
        WM_ERASEBKGND => {
            fill_background(hwnd);
            1
        }
        WM_CLOSE => {
            destroy(hwnd);
            0
        }
        _ => 0,
    }
}

#[allow(non_snake_case)]
pub extern "stdcall" fn ShowWindow(hwnd: u32, cmd: u32) -> i32 {
    let mut s = STATE.lock().unwrap();
    let Some(w) = s.windows.get_mut(&hwnd) else { return 0 };
    let was = w.visible;
    w.visible = cmd != 0;
    let erase_now = w.visible && !was && w.is16 && w.need_erase;
    // First top-level window to appear (Win32): Windows activates it and notifies the program.
    // DirectDraw games only draw after receiving WM_ACTIVATEAPP/WM_ACTIVATE.
    let activate_now = w.visible && !was && !w.is16 && w.style & WS_CHILD == 0 && !matches!(cmd, 4 | 7 | 8)
        && !APP_ACTIVATED.swap(true, std::sync::atomic::Ordering::Relaxed);
    if w.visible {
        w.need_paint = true;
    }
    if erase_now {
        w.need_erase = false;
    }
    drop(s);
    if erase_now {
        // Windows erases the background when the window appears; WM_PAINT comes later, without erasing.
        let hdc = gdi32::dc_for_window(hwnd);
        if send(hwnd, WM_ERASEBKGND, hdc, 0) == 0 {
            fill_background(hwnd);
        }
        gdi32::release_dc(hdc);
    }
    if activate_now {
        STATE.lock().unwrap().active = hwnd;
        send(hwnd, WM_ACTIVATEAPP, 1, 0);
        send(hwnd, WM_ACTIVATE, 1, 0); // WA_ACTIVE
    }
    touch();
    was as i32
}

/// The program has already received WM_ACTIVATEAPP (only the first window to appear triggers it).
static APP_ACTIVATED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
const WM_ACTIVATE: u32 = 0x0006;
const WM_ACTIVATEAPP: u32 = 0x001C;

#[allow(non_snake_case)]
pub extern "stdcall" fn UpdateWindow(hwnd: u32) -> i32 {
    let pending = STATE.lock().unwrap().windows.get(&hwnd).is_some_and(|w| w.need_paint);
    if pending {
        send(hwnd, WM_PAINT, 0, 0);
    }
    1
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn InvalidateRect(hwnd: u32, _rect: u32, erase: i32) -> i32 {
    if let Some(w) = STATE.lock().unwrap().windows.get_mut(&hwnd) {
        w.need_paint = true;
        w.need_erase |= erase != 0;
    }
    1
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn GetClientRect(hwnd: u32, rect: *mut i32) -> i32 {
    if hwnd == 1 {
        *rect = 0;
        *rect.add(1) = 0;
        *rect.add(2) = desktop_w() as i32;
        *rect.add(3) = desktop_h() as i32;
        return 1;
    }
    match with_buf(hwnd, |_, w, h| (w, h)) {
        Some((w, h)) => {
            *rect = 0;
            *rect.add(1) = 0;
            *rect.add(2) = w;
            *rect.add(3) = h;
            1
        }
        None => 0,
    }
}

#[allow(non_snake_case)]
pub extern "stdcall" fn MoveWindow(hwnd: u32, x: i32, y: i32, w: i32, h: i32, _repaint: i32) -> i32 {
    let (cw, ch);
    {
        let mut s = STATE.lock().unwrap();
        let nudge = {
            // buttons of Win16 double-frame dialogs (e.g. OK/Go Back in Civilization) move up
            // BUTTON_NUDGE px from what the program requests (adjustment requested by the user)
            let w0 = s.windows.get(&hwnd);
            let parent_framed = w0.is_some_and(|c| c.class.eq_ignore_ascii_case("button") && c.style & WS_CHILD != 0 && s.windows.get(&c.parent).is_some_and(|p| p.is16 && dlg_frame(p.style) > 0));
            if parent_framed { BUTTON_NUDGE } else { 0 }
        };
        let Some(win) = s.windows.get_mut(&hwnd) else { return 0 };
        win.x = x;
        win.y = y - nudge;
        win.w = w;
        win.h = h;
        let cap = win.style & WS_CAPTION == WS_CAPTION;
        let f = dlg_frame(win.style);
        cw = if cap { w - 2 * BORDER } else { w - 2 * f }.max(0);
        ch = if cap { h - CAPTION_H - 2 * BORDER - if win.menu != 0 { MENU_H } else { 0 } } else { h - 2 * f }.max(0);
        win.cw = cw;
        win.ch = ch;
        win.buf = vec![if win.style & WS_CHILD != 0 { TRANSPARENT } else { 0xFFFF_FFFF }; (cw * ch) as usize];
        win.need_paint = true;
        win.need_erase = true;
    }
    send(hwnd, WM_SIZE, 0, (cw as u32 & 0xFFFF) | ((ch as u32) << 16));
    1
}

// ---- message queue ------------------------------------------------------------------------

unsafe fn write_msg(out: *mut u32, m: Msg) {
    *out = m.hwnd;
    *out.add(1) = m.message;
    *out.add(2) = m.wparam;
    *out.add(3) = m.lparam;
    *out.add(4) = now_ms();
    let (cx, cy) = input::cursor();
    *out.add(5) = cx as u32;
    *out.add(6) = cy as u32;
}

/// Next message: queue, then pending WM_PAINT, then WM_QUIT.
pub fn next_message(remove: bool) -> Option<Msg> {
    next_message_filtered(remove, 0, 0, 0)
}

/// Like `next_message`, honoring the window filter (`hwnd`, 0 = any) and the message range filter
/// (`min..=max`, both 0 = all), like `PeekMessage`/`GetMessage`.
fn is_descendant(s: &State, mut h: u32, root: u32) -> bool {
    for _ in 0..16 {
        match s.windows.get(&h) {
            Some(w) if w.parent != 0 => h = w.parent,
            _ => return false,
        }
        if h == root {
            return true;
        }
    }
    false
}

pub fn next_message_filtered(remove: bool, hwnd: u32, min: u32, max: u32) -> Option<Msg> {
    apply_pending_screen();
    input::pump(); // pending input and expired timers become messages
    let in_range = |m: u32| (min == 0 && max == 0) || (m >= min && m <= max);
    let mut s = STATE.lock().unwrap();
    // The window filter includes children (as on Windows).
    let for_window = |s: &State, h: u32| hwnd == 0 || h == hwnd || is_descendant(s, h, hwnd);
    if let Some(i) = s.queue.iter().position(|m| in_range(m.message) && for_window(&s, m.hwnd)) {
        let m = s.queue[i];
        if remove {
            s.queue.remove(i);
        }
        return Some(m);
    }
    if in_range(WM_PAINT) {
        if let Some((&h, _)) = s.windows.iter().find(|(&h, w)| w.need_paint && w.visible && for_window(&s, h)) {
            return Some(Msg { hwnd: h, message: WM_PAINT, wparam: 0, lparam: 0 });
        }
    }
    if let Some(i) = s.timer_msgs.iter().position(|m| in_range(m.message) && for_window(&s, m.hwnd)) {
        let m = s.timer_msgs[i];
        if remove {
            s.timer_msgs.remove(i);
        }
        return Some(m);
    }
    if in_range(WM_QUIT) {
        return s.quit.map(|code| Msg { hwnd: 0, message: WM_QUIT, wparam: code, lparam: 0 });
    }
    None
}

/// Called when there is nothing to do. In capture mode (`--shot`) time is virtual: advances
/// frames (`wait` script, timers) and, once exhausted, saves the PNG and exits. With a viewer, sleeps.
fn idle() {
    if shot_mode() {
        // Yields real time to the program threads (program time is virtual, theirs is not).
        std::thread::sleep(std::time::Duration::from_millis(5));
        if input::take_wait() {
            input::advance_virtual(50);
            return;
        }
        if input::has_pending() {
            return;
        }
        // DirectDraw game: the frame count is what ends the capture (see `ddraw::frame_done`).
        if super::ddraw::active() {
            input::advance_virtual(16);
            return;
        }
        let max_frames: u32 = std::env::var("WINLAYER_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(30);
        {
            let mut s = STATE.lock().unwrap();
            if !s.timers.is_empty() && s.frames < max_frames {
                s.frames += 1;
                drop(s);
                input::advance_virtual(50);
                return;
            }
        }
        if let Ok(path) = std::env::var("WINLAYER_SHOT") {
            dump_screenshot(&path);
        }
        super::kernel32::exit_process(0)
    }
    std::thread::sleep(std::time::Duration::from_millis(8));
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn GetMessageA(out: *mut u32, hwnd: u32, min: u32, max: u32) -> i32 {
    loop {
        if let Some(m) = next_message_filtered(true, hwnd, min, max) {
            write_msg(out, m);
            return (m.message != WM_QUIT) as i32;
        }
        idle();
    }
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn PeekMessageA(out: *mut u32, hwnd: u32, min: u32, max: u32, remove: u32) -> i32 {
    match next_message_filtered(remove & PM_REMOVE != 0, hwnd, min, max) {
        Some(m) => {
            write_msg(out, m);
            1
        }
        None => {
            idle();
            0
        }
    }
}

#[allow(non_snake_case)]
pub extern "stdcall" fn TranslateMessage(_msg: u32) -> i32 {
    0
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn DispatchMessageA(msg: *const u32) -> u32 {
    let (hwnd, m, w, l) = (*msg, *msg.add(1), *msg.add(2), *msg.add(3));
    if m == WM_TIMER && l != 0 {
        return call4(l, hwnd, WM_TIMER, w, now_ms()); // SetTimer with TIMERPROC
    }
    send(hwnd, m, w, l)
}

#[allow(non_snake_case)]
pub extern "stdcall" fn PostMessageA(hwnd: u32, message: u32, wparam: u32, lparam: u32) -> i32 {
    STATE.lock().unwrap().queue.push_back(Msg { hwnd, message, wparam, lparam });
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SendMessageA(hwnd: u32, message: u32, wparam: u32, lparam: u32) -> u32 {
    send(hwnd, message, wparam, lparam)
}

#[allow(non_snake_case)]
pub extern "stdcall" fn PostQuitMessage(code: u32) {
    STATE.lock().unwrap().quit = Some(code);
}

// ---- painting -----------------------------------------------------------------------------

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn BeginPaint(hwnd: u32, ps: *mut u32) -> u32 {
    let hdc = gdi32::dc_for_window(hwnd);
    let (erase, cw, ch) = {
        let mut s = STATE.lock().unwrap();
        let Some(w) = s.windows.get_mut(&hwnd) else { return 0 };
        w.need_paint = false;
        (std::mem::take(&mut w.need_erase), w.cw, w.ch)
    };
    if erase && send(hwnd, WM_ERASEBKGND, hdc, 0) == 0 {
        fill_background(hwnd);
    }
    // PAINTSTRUCT: hdc, fErase, rcPaint(4), fRestore, fIncUpdate, rgbReserved[32]
    std::ptr::write_bytes(ps as *mut u8, 0, 64);
    *ps = hdc;
    *ps.add(1) = erase as u32;
    *ps.add(4) = cw as u32;
    *ps.add(5) = ch as u32;
    hdc
}

#[allow(non_snake_case)]
pub extern "stdcall" fn EndPaint(_hwnd: u32, ps: *const u32) -> i32 {
    gdi32::release_dc(unsafe { *ps });
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetDC(hwnd: u32) -> u32 {
    gdi32::dc_for_window(hwnd)
}

#[allow(non_snake_case)]
pub extern "stdcall" fn ReleaseDC(_hwnd: u32, hdc: u32) -> i32 {
    gdi32::release_dc(hdc);
    1
}

// ---- resources, dialogs, menus (stubs) ----------------------------------------------------

#[allow(non_snake_case)]
pub extern "stdcall" fn LoadCursorA(_inst: u32, _name: u32) -> u32 {
    0x7001
}

#[allow(non_snake_case)]
pub extern "stdcall" fn LoadIconA(_inst: u32, _name: u32) -> u32 {
    0x7002
}

#[allow(non_snake_case)]
pub extern "stdcall" fn FindWindowA(_class: u32, _title: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetLastActivePopup(hwnd: u32) -> u32 {
    hwnd
}

#[allow(non_snake_case)]
pub extern "stdcall" fn IsIconic(_hwnd: u32) -> i32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SendDlgItemMessageA(dlg: u32, id: i32, msg: u32, w: u32, l: u32) -> u32 {
    match find_child(dlg, id as u32) {
        Some(h) => send(h, msg, w, l),
        None => 0,
    }
}

const IDOK: i32 = 1;

/// TODO: a real window. For now, writes to stderr.
#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn MessageBoxA(_hwnd: u32, text: u32, caption: u32, _kind: u32) -> i32 {
    eprintln!("[MessageBox] {}\n{}", cstr(caption), cstr(text));
    if crate::diag::enabled() {
        crate::diag::note(crate::diag::Kind::MessageBox, format!("{}: {}{}", cstr(caption), cstr(text), crate::diag::callers()));
    }
    IDOK
}

// ---- wsprintfA (cdecl, variadic) ----------------------------------------------------------

/// The arguments come as a va_list (a pointer to them on the stack): the same first 8 as wsprintfA.
#[allow(non_snake_case)]
unsafe extern "stdcall" fn wvsprintfA(out: *mut u8, fmt: *const u8, args: *const u32) -> i32 {
    let a = |i: usize| if args.is_null() { 0 } else { *args.add(i) };
    wsprintfA(out, fmt, a(0), a(1), a(2), a(3), a(4), a(5), a(6), a(7))
}

/// We read up to 8 "extra" arguments from the stack; whatever the caller did not push is ignored garbage.
#[allow(non_snake_case)]
unsafe extern "C" fn wsprintfA(out: *mut u8, fmt: *const u8, a0: u32, a1: u32, a2: u32, a3: u32, a4: u32, a5: u32, a6: u32, a7: u32) -> i32 {
    let args = [a0, a1, a2, a3, a4, a5, a6, a7];
    let mut next = 0;
    let mut take = || {
        let v = args.get(next).copied().unwrap_or(0);
        next += 1;
        v
    };
    let f = std::ffi::CStr::from_ptr(fmt as *const i8).to_bytes();
    let mut res = Vec::new();
    let mut i = 0;
    while i < f.len() {
        if f[i] != b'%' {
            res.push(f[i]);
            i += 1;
            continue;
        }
        i += 1;
        let (mut left, mut zero) = (false, false);
        while i < f.len() && matches!(f[i], b'-' | b'0') {
            if f[i] == b'-' { left = true } else { zero = true }
            i += 1;
        }
        let mut width = 0usize;
        while i < f.len() && f[i].is_ascii_digit() {
            width = width * 10 + (f[i] - b'0') as usize;
            i += 1;
        }
        while i < f.len() && matches!(f[i], b'l' | b'h') { i += 1 }
        let Some(&t) = f.get(i) else { break };
        i += 1;
        let text: Vec<u8> = match t {
            b'd' | b'i' => (take() as i32).to_string().into_bytes(),
            b'u' => take().to_string().into_bytes(),
            b'x' => format!("{:x}", take()).into_bytes(),
            b'X' => format!("{:X}", take()).into_bytes(),
            b'c' => vec![take() as u8],
            b's' => {
                let p = take();
                if p == 0 { b"(null)".to_vec() } else { std::ffi::CStr::from_ptr(p as usize as *const i8).to_bytes().to_vec() }
            }
            b'%' => vec![b'%'],
            _ => Vec::new(),
        };
        let pad = width.saturating_sub(text.len());
        if left {
            res.extend_from_slice(&text);
            res.extend(std::iter::repeat(b' ').take(pad));
        } else {
            res.extend(std::iter::repeat(if zero { b'0' } else { b' ' }).take(pad));
            res.extend_from_slice(&text);
        }
    }
    std::ptr::copy_nonoverlapping(res.as_ptr(), out, res.len());
    *out.add(res.len()) = 0;
    res.len() as i32
}

// ---- screen capture -----------------------------------------------------------------------

fn blit(desk: &mut [u32], dx: i32, dy: i32, src: &[u32], sw: i32, sh: i32) {
    for y in 0..sh {
        for x in 0..sw {
            let (tx, ty) = (dx + x, dy + y);
            if tx >= 0 && ty >= 0 && (tx as usize) < desktop_w() && (ty as usize) < desktop_h() {
                desk[ty as usize * desktop_w() + tx as usize] = src[(y * sw + x) as usize];
            }
        }
    }
}

/// Like `blit`, but skips transparent pixels (child windows not painted yet).
fn blit_keyed(desk: &mut [u32], dx: i32, dy: i32, src: &[u32], sw: i32, sh: i32) {
    for y in 0..sh {
        for x in 0..sw {
            let p = src[(y * sw + x) as usize];
            let (tx, ty) = (dx + x, dy + y);
            if p >> 24 != 0 && tx >= 0 && ty >= 0 && (tx as usize) < desktop_w() && (ty as usize) < desktop_h() {
                desk[ty as usize * desktop_w() + tx as usize] = p;
            }
        }
    }
}

fn rect(desk: &mut [u32], x: i32, y: i32, w: i32, h: i32, c: u32) {
    let px = vec![c; (w.max(0) * h.max(0)) as usize];
    blit(desk, x, y, &px, w.max(0), h.max(0));
}

fn draw_window(s: &State, desk: &mut [u32], hwnd: u32, ox: i32, oy: i32) {
    let Some(w) = s.windows.get(&hwnd) else { return };
    if !w.visible { return }
    let (mut cx, mut cy) = (ox + w.x, oy + w.y);
    if w.style & WS_CHILD == 0 && w.style & WS_CAPTION == WS_CAPTION {
        rect(desk, cx, cy, w.w, w.h, 0xFFC0_C0C0);
        let active = s.active == hwnd;
        rect(desk, cx + BORDER, cy + BORDER - 1, w.w - 2 * BORDER, CAPTION_H - 1, if active { 0xFF00_0080 } else { 0xFF80_8080 });
        let title = w.title.bytes().collect::<Vec<u8>>();
        let mut tb = Buf { px: desk, w: desktop_w() as i32, h: desktop_h() as i32 };
        tb.text(cx + BORDER + 3, cy + BORDER + 2, &title, draw::WHITE, true);
        // close button
        let (bx, by) = (cx + w.w - BORDER - 18, cy + BORDER + 1);
        tb.fill(bx, by, bx + 16, by + 14, draw::GRAY);
        tb.raised(bx, by, bx + 16, by + 14);
        tb.text(bx + 5, by + 1, b"x", draw::BLACK, true);
        cx += BORDER;
        cy += BORDER + CAPTION_H - 1;
        if w.menu != 0 {
            let mut b = Buf { px: desk, w: desktop_w() as i32, h: desktop_h() as i32 };
            b.fill(cx, cy, cx + w.cw, cy + MENU_H, draw::GRAY);
            let mut x = cx + 8;
            for it in menu::items(w.menu) {
                let label = text::strip_mnemonic(&it.text);
                let col = if it.flags & (menu::MF_GRAYED | menu::MF_DISABLED) != 0 { draw::DARK } else { draw::BLACK };
                x += b.text(x, cy + 3, &label, col, false) + 14;
            }
            cy += MENU_H;
        }
    }
    let f = dlg_frame(w.style);
    if f > 0 {
        // double frame: black outside, light gray and shadow inside
        rect(desk, cx, cy, w.w, w.h, 0xFF00_0000);
        rect(desk, cx + 1, cy + 1, w.w - 2, w.h - 2, 0xFFC0_C0C0);
        rect(desk, cx + 2, cy + 2, w.w - 4, w.h - 4, 0xFF80_8080);
        cx += f;
        cy += f;
    }
    if w.style & WS_CHILD != 0 {
        blit_keyed(desk, cx, cy, &w.buf, w.cw, w.ch);
    } else {
        blit(desk, cx, cy, &w.buf, w.cw, w.ch);
    }
    for (&child, c) in &s.windows {
        if c.parent == hwnd && c.style & WS_CHILD != 0 {
            draw_window(s, desk, child, cx, cy);
        }
    }
}

pub fn render_desktop() -> Vec<u32> {
    render_desktop_sized().2
}

/// Draws the desktop and returns (width, height, pixels). The size is read under the `STATE` lock, the same one
/// `apply_pending_screen` holds when changing it, so buffer and drawing always agree.
pub fn render_desktop_sized() -> (usize, usize, Vec<u32>) {
    let s = STATE.lock().unwrap();
    let (w, h) = (desktop_w(), desktop_h());
    let mut desk = vec![gdi32::colorref_to_px(gdi32::sys_color(1)); w * h];
    let mut tops: Vec<(&u32, &Window)> = s.windows.iter().filter(|(_, w)| w.style & WS_CHILD == 0).collect();
    tops.sort_by_key(|(_, w)| w.z);
    for (&hwnd, _) in tops {
        draw_window(&s, &mut desk, hwnd, 0, 0);
    }
    input::draw_popups(&s, &mut desk);
    (w, h, desk)
}

pub fn dump_screenshot(path: &str) {
    if std::env::var_os("WINLAYER_TRACE").is_some() {
        for (h, w) in &STATE.lock().unwrap().windows {
            eprintln!("[window {h:#x}] class={:?} title={:?} pos=({}, {}) size={}x{} client={}x{} style={:#x} visible={} parent={:#x}", w.class, w.title, w.x, w.y, w.w, w.h, w.cw, w.ch, w.style, w.visible, w.parent);
        }
    }
    let (w, h, desk) = render_desktop_sized();
    let n = STATE.lock().unwrap().windows.len();
    match crate::png::write(path, w, h, &desk) {
        Ok(()) => eprintln!("[winlayer] capture saved to {path} ({n} windows)"),
        Err(e) => eprintln!("[winlayer] failed to save {path}: {e}"),
    }
}

// ---- dialogs ------------------------------------------------------------------------------

unsafe fn r16(a: u32) -> u32 {
    *(a as usize as *const u16) as u32
}
unsafe fn r16s(a: u32) -> i32 {
    *(a as usize as *const i16) as i32
}
unsafe fn r32(a: u32) -> u32 {
    *(a as usize as *const u32)
}

/// Skips an "ordinal or string" field of the DLGTEMPLATE (menu, class).
unsafe fn skip_ord_or_str(p: u32) -> u32 {
    match r16(p) {
        0 => p + 2,
        0xFFFF => p + 4,
        _ => resources::wide_z(p).1,
    }
}

/// Creates a dialog (window + controls) from a DLGTEMPLATE or DLGTEMPLATEEX and sends WM_INITDIALOG.
unsafe fn create_dialog(tmpl: u32, parent: u32, dlgproc: u32, param: u32) -> u32 {
    // DLGTEMPLATEEX starts with dlgVer = 1 and signature = 0xFFFF; its fields sit at other offsets.
    let ex = r16(tmpl) == 1 && r16(tmpl + 2) == 0xFFFF;
    let h0 = if ex { tmpl + 8 } else { tmpl }; // skips dlgVer, signature, helpID
    let style = if ex { r32(h0 + 4) } else { r32(h0) };
    let cdit = r16(h0 + 8);
    let (dcx, dcy) = (r16s(h0 + 14), r16s(h0 + 16));
    let mut p = skip_ord_or_str(h0 + 18); // menu
    p = skip_ord_or_str(p); // class
    let (title, np) = resources::wide_z(p);
    p = np;
    if style & 0x40 != 0 {
        // DS_SETFONT: size (+ weight, italic, charset in the extended form) + font name
        p = resources::wide_z(p + if ex { 6 } else { 2 }).1;
    }
    let (cw, ch) = (dcx * DLU_X / 4, dcy * DLU_Y / 8);
    let caption = style & WS_CAPTION == WS_CAPTION;
    let (w, h) = if caption { (cw + 2 * BORDER, ch + CAPTION_H + 2 * BORDER) } else { (cw, ch) };
    let (x, y) = ((desktop_w() as i32 - w) / 2, (desktop_h() as i32 - h) / 2);
    let dlg = create_window(DIALOG_CLASS.into(), title, style | WS_VISIBLE, x, y, w, h, parent, 0, 0, param, dlgproc);

    let mut first_tab = 0;
    let mut first = 0;
    for _ in 0..cdit {
        p = tmpl + ((p - tmpl + 3) & !3);
        let (istyle, ix, iy, icx, icy, iid) = if ex {
            // helpID, exStyle, style, x, y, cx, cy, id (32-bit)
            let v = (r32(p + 8), r16s(p + 12), r16s(p + 14), r16s(p + 16), r16s(p + 18), r32(p + 20) as u16 as u32);
            p += 24;
            v
        } else {
            let v = (r32(p), r16s(p + 8), r16s(p + 10), r16s(p + 12), r16s(p + 14), r16(p + 16));
            p += 18;
            v
        };
        let class = if r16(p) == 0xFFFF {
            let atom = r16(p + 2);
            p += 4;
            match atom {
                0x80 => "button", 0x81 => "edit", 0x82 => "static", 0x83 => "listbox",
                0x84 => "scrollbar", 0x85 => "combobox", _ => "static",
            }.to_string()
        } else {
            let (c, np) = resources::wide_z(p);
            p = np;
            c
        };
        let text = if r16(p) == 0xFFFF {
            p += 4;
            String::new() // title by ordinal (icon/bitmap): not supported yet
        } else {
            let (t, np) = resources::wide_z(p);
            p = np;
            t
        };
        p += 2 + r16(p); // extra creation data
        let child = create_window(
            class, text, istyle | WS_CHILD, ix * DLU_X / 4, iy * DLU_Y / 8, icx * DLU_X / 4, icy * DLU_Y / 8,
            dlg, iid, 0, 0, 0,
        );
        if first == 0 { first = child }
        if first_tab == 0 && istyle & WS_TABSTOP != 0 { first_tab = child }
    }
    let focus_target = if first_tab != 0 { first_tab } else { first };
    if send(dlg, WM_INITDIALOG, focus_target, param) != 0 && focus_target != 0 {
        STATE.lock().unwrap().focus = focus_target; // the dialog accepted the default focus
    }
    dlg
}

unsafe fn dialog_template(inst: u32, name: u32) -> Option<u32> {
    let base = if inst == 0 { super::kernel32::image_base() } else { inst };
    resources::find(base, &resources::ResId::Id(resources::RT_DIALOG), &resources::res_id(name)).map(|r| r.1)
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn DialogBoxParamA(inst: u32, name: u32, parent: u32, dlgproc: u32, param: u32) -> i32 {
    let Some(tmpl) = dialog_template(inst, name) else {
        crate::diag::warn("DialogBoxParamA: dialog resource not found".into());
        return -1;
    };
    modal_dialog(tmpl, parent, dlgproc, param)
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn DialogBoxIndirectParamA(_inst: u32, tmpl: u32, parent: u32, dlgproc: u32, param: u32) -> i32 {
    if tmpl == 0 { -1 } else { modal_dialog(tmpl, parent, dlgproc, param) }
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn RedrawWindow(hwnd: u32, _rect: u32, _rgn: u32, flags: u32) -> i32 {
    const RDW_ERASE: u32 = 0x4;
    const RDW_UPDATENOW: u32 = 0x100;
    InvalidateRect(hwnd, 0, (flags & RDW_ERASE != 0) as i32);
    if flags & RDW_UPDATENOW != 0 {
        UpdateWindow(hwnd);
    }
    1
}

/// Only the current mode exists: changing the resolution "works" without doing anything.
#[allow(non_snake_case)]
pub extern "stdcall" fn ChangeDisplaySettingsA(_mode: u32, _flags: u32) -> i32 {
    0
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn GetKeyNameTextA(_lparam: u32, buf: *mut u8, n: i32) -> i32 {
    if !buf.is_null() && n > 0 {
        *buf = 0;
    }
    0
}

/// MAPVK_VK_TO_VSC (0) via the scan code table; the other modes are not supported.
#[allow(non_snake_case)]
pub extern "stdcall" fn MapVirtualKeyA(code: u32, mode: u32) -> u32 {
    if mode == 0 { default_scan(code).0 } else { 0 }
}

unsafe fn modal_dialog(tmpl: u32, parent: u32, dlgproc: u32, param: u32) -> i32 {
    let dlg = create_dialog(tmpl, parent, dlgproc, param);
    loop {
        let ended = STATE.lock().unwrap().windows.get(&dlg).and_then(|w| w.ended);
        if let Some(r) = ended {
            destroy(dlg);
            return r;
        }
        match next_message(true) {
            Some(m) if m.message == WM_QUIT => {
                destroy(dlg);
                return 0; // WM_QUIT stays flagged and ends the main loop
            }
            Some(m) => {
                send(m.hwnd, m.message, m.wparam, m.lparam);
            }
            None => idle(),
        }
    }
}


// ---- file dialog (GetOpenFileName / GetSaveFileName) ------------------------------------------

static FILE_PICK: Mutex<Option<String>> = Mutex::new(None);

fn child_text(dlg: u32, id: u32) -> String {
    find_child(dlg, id).and_then(|c| STATE.lock().unwrap().windows.get(&c).map(|w| w.title.clone())).unwrap_or_default()
}

/// File dialog procedure (native): OK/Cancel, selection and double click on the list.
extern "stdcall" fn filedlg_proc(hwnd: u32, msg: u32, w: u32, _l: u32) -> u32 {
    if msg != WM_COMMAND {
        return 0;
    }
    let (id, code) = (w & 0xFFFF, w >> 16);
    match id {
        1 => {
            let name = child_text(hwnd, 1000).trim().to_string();
            if !name.is_empty() {
                *FILE_PICK.lock().unwrap() = Some(name);
                EndDialog(hwnd, 1);
            }
            1
        }
        2 => {
            EndDialog(hwnd, 0);
            1
        }
        1001 if code == 1 || code == 2 => {
            if let Some(list) = find_child(hwnd, 1001) {
                let sel = listbox::selection(list);
                let mut buf = [0u8; 260];
                if sel >= 0 && send(list, 0x189, sel as u32, buf.as_mut_ptr() as u32) != u32::MAX {
                    let n = buf.iter().position(|&c| c == 0).unwrap_or(0);
                    set_text(find_child(hwnd, 1000).unwrap_or(0), String::from_utf8_lossy(&buf[..n]).into_owned());
                    if code == 2 {
                        return filedlg_proc(hwnd, WM_COMMAND, 1, 0);
                    }
                }
            }
            1
        }
        _ => 0,
    }
}

/// Shows a modal "open/save" dialog. `dir` is a program path (empty = root C:\), `ext` is the
/// filter extension (without dot; empty or "*" = all). Returns the typed/chosen name.
pub fn file_dialog(parent: u32, title: &str, dir: &str, ext: &str, default_name: &str, save: bool) -> Option<String> {
    let (w, h) = (372, 246 + CAPTION_H);
    let (x, y) = ((desktop_w() as i32 - w) / 2, (desktop_h() as i32 - h) / 2);
    let dlg = unsafe { create_window(DIALOG_CLASS.into(), title.to_string(), 0x8000_0000 | WS_CAPTION | 0x80 | WS_VISIBLE, x, y, w, h, parent, 0, 0, 0, filedlg_proc as usize as u32) };
    let child = |class: &str, text: &str, style: u32, x: i32, y: i32, w: i32, h: i32, id: u32| {
        unsafe { create_window(class.into(), text.to_string(), style | WS_CHILD | WS_VISIBLE, x, y, w, h, dlg, id, 0, 0, 0) }
    };
    child("static", "File name:", 0, 10, 8, 120, 16, 0xFFFF);
    child("edit", default_name, 0x0080_0000 | 0x80 | WS_TABSTOP, 10, 26, 250, 22, 1000);
    let listing_dir = crate::win32::files::host_path(dir);
    child("static", if save { "Existing saved games:" } else { "Files:" }, 0, 10, 56, 250, 16, 0xFFFF);
    let list = child("listbox", "", 0x0080_0000 | 0x20_0000 | 0x3 | WS_TABSTOP, 10, 74, 250, 160, 1001);
    let pattern_ext = ext.trim_start_matches("*.").trim_start_matches('*').trim_start_matches('.');
    if let Ok(rd) = std::fs::read_dir(&listing_dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let ok = e.path().is_file() && (pattern_ext.is_empty() || name.rsplit_once('.').is_some_and(|(_, x)| x.eq_ignore_ascii_case(pattern_ext)));
            if ok {
                let c = std::ffi::CString::new(name).unwrap_or_default();
                send(list, 0x180, 0, c.as_ptr() as u32);
            }
        }
    }
    child("button", "OK", 1, 274, 26, 88, 26, 1);
    child("button", "Cancel", 0, 274, 60, 88, 26, 2);
    *FILE_PICK.lock().unwrap() = None;
    STATE.lock().unwrap().focus = find_child(dlg, 1000).unwrap_or(0);
    loop {
        let ended = STATE.lock().unwrap().windows.get(&dlg).and_then(|w| w.ended);
        if let Some(r) = ended {
            destroy(dlg);
            return if r == 1 { FILE_PICK.lock().unwrap().take() } else { None };
        }
        match next_message(true) {
            Some(m) if m.message == WM_QUIT => {
                destroy(dlg);
                return None;
            }
            Some(m) => {
                send(m.hwnd, m.message, m.wparam, m.lparam);
            }
            None => idle(),
        }
    }
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn CreateDialogParamA(inst: u32, name: u32, parent: u32, dlgproc: u32, param: u32) -> u32 {
    match dialog_template(inst, name) {
        Some(t) => create_dialog(t, parent, dlgproc, param),
        None => 0,
    }
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn CreateDialogIndirectParamA(_inst: u32, tmpl: u32, parent: u32, dlgproc: u32, param: u32) -> u32 {
    if tmpl == 0 { 0 } else { create_dialog(tmpl, parent, dlgproc, param) }
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn DrawTextExA(hdc: u32, s: *const u8, n: i32, rect: *mut i32, flags: u32, _params: u32) -> i32 {
    DrawTextA(hdc, s, n, rect, flags)
}

#[allow(non_snake_case)]
pub extern "stdcall" fn ClipCursor(_rect: u32) -> i32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn WindowFromDC(hdc: u32) -> u32 {
    gdi32::dc_hwnd(hdc)
}

#[allow(non_snake_case)]
pub extern "stdcall" fn EndDialog(dlg: u32, result: u32) -> i32 {
    if let Some(w) = STATE.lock().unwrap().windows.get_mut(&dlg) {
        w.ended = Some(result as i32);
    }
    1
}

fn find_child(dlg: u32, id: u32) -> Option<u32> {
    STATE.lock().unwrap().windows.iter().find(|(_, w)| w.parent == dlg && w.id == id).map(|(&h, _)| h)
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetDlgItem(dlg: u32, id: u32) -> u32 {
    find_child(dlg, id).unwrap_or(0)
}

pub fn set_text(hwnd: u32, text: String) {
    touch();
    if let Some(w) = STATE.lock().unwrap().windows.get_mut(&hwnd) {
        w.title = text;
        w.need_paint = true;
    }
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn SetDlgItemTextA(dlg: u32, id: u32, text: u32) -> i32 {
    match find_child(dlg, id) {
        Some(h) => { set_text(h, cstr(text)); 1 }
        None => 0,
    }
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn SetDlgItemInt(dlg: u32, id: u32, value: u32, signed: i32) -> i32 {
    let t = if signed != 0 { (value as i32).to_string() } else { value.to_string() };
    match find_child(dlg, id) {
        Some(h) => { set_text(h, t); 1 }
        None => 0,
    }
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn SetWindowTextA(hwnd: u32, text: u32) -> i32 {
    set_text(hwnd, cstr(text));
    1
}

pub unsafe fn copy_title(hwnd: u32, buf: *mut u8, max: i32) -> i32 {
    let t = STATE.lock().unwrap().windows.get(&hwnd).map(|w| w.title.clone()).unwrap_or_default();
    let n = t.len().min((max.max(1) - 1) as usize);
    std::ptr::copy_nonoverlapping(t.as_ptr(), buf, n);
    *buf.add(n) = 0;
    n as i32
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn GetWindowTextA(hwnd: u32, buf: *mut u8, max: i32) -> i32 {
    copy_title(hwnd, buf, max)
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn GetDlgItemTextA(dlg: u32, id: u32, buf: *mut u8, max: i32) -> u32 {
    match find_child(dlg, id) {
        Some(h) => copy_title(h, buf, max) as u32,
        None => { *buf = 0; 0 }
    }
}

// ---- strings, text and metrics ------------------------------------------------------------

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn LoadStringA(inst: u32, id: u32, buf: *mut u8, max: i32) -> i32 {
    let base = if inst == 0 { super::kernel32::image_base() } else { inst };
    let Some((_, mut p, _)) = resources::find(base, &resources::ResId::Id(resources::RT_STRING), &resources::ResId::Id(id / 16 + 1)) else {
        return 0;
    };
    // Block of 16 strings: each one is [length u16][UTF-16 without NUL].
    for i in 0..16 {
        let len = r16(p);
        if i == id % 16 {
            let n = (len as i32).min(max - 1).max(0) as u32;
            for k in 0..n {
                *buf.add(k as usize) = r16(p + 2 + 2 * k) as u8;
            }
            *buf.add(n as usize) = 0;
            return n as i32;
        }
        p += 2 + 2 * len;
    }
    0
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn FillRect(hdc: u32, rect: *const i32, brush: u32) -> i32 {
    let color = if brush >= 1 && brush <= 31 { Some(gdi32::sys_color(brush - 1)) } else { gdi32::brush_color(brush) };
    let Some(c) = color else { return 1 };
    let (l, t, r, b) = (*rect, *rect.add(1), *rect.add(2), *rect.add(3));
    gdi32::with_dc_target(hdc, |px, w, h| Buf { px, w, h }.fill(l, t, r, b, gdi32::colorref_to_px(c)));
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetSysColor(i: u32) -> u32 {
    gdi32::sys_color(i)
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetSystemMetrics(i: u32) -> i32 {
    match i {
        0 => desktop_w() as i32,             // SM_CXSCREEN
        1 => desktop_h() as i32,             // SM_CYSCREEN
        2 | 3 => 16,                       // SM_CXVSCROLL / SM_CYHSCROLL
        4 => CAPTION_H,                    // SM_CYCAPTION
        5 | 6 => 1,                        // SM_CXBORDER / SM_CYBORDER
        7 | 8 => 3,                        // SM_CXDLGFRAME / SM_CYDLGFRAME
        9 | 10 => 16,                      // SM_CYVTHUMB / SM_CXHTHUMB
        11..=14 => 32,                     // SM_CXICON, CYICON, CXCURSOR, CYCURSOR
        15 => 19,                          // SM_CYMENU
        16 => desktop_w() as i32,            // SM_CXFULLSCREEN
        17 => desktop_h() as i32 - CAPTION_H, // SM_CYFULLSCREEN
        32 | 33 => BORDER,                 // SM_CXFRAME / SM_CYFRAME
        _ => 0,
    }
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn DrawTextA(hdc: u32, s: *const u8, n: i32, rect: *mut i32, flags: u32) -> i32 {
    let Some((fg, bk, bold)) = gdi32::text_style(hdc) else { return 0 };
    let bytes = if n < 0 { std::ffi::CStr::from_ptr(s as *const i8).to_bytes() } else { std::slice::from_raw_parts(s, n as usize) };
    let label = text::strip_mnemonic(&String::from_utf8_lossy(bytes));
    let (l, t, r, b) = (*rect, *rect.add(1), *rect.add(2), *rect.add(3));
    let font = fnt::get(gdi32::font_idx(hdc));
    let (tw, lh) = match &font {
        Some(f) => (f.width(&label), f.height),
        None => (text::width(bold, &label), text::line_h()),
    };
    if flags & 0x400 != 0 { // DT_CALCRECT
        *rect.add(2) = l + tw;
        *rect.add(3) = t + lh;
        return lh;
    }
    let x = if flags & 1 != 0 { l + (r - l - tw) / 2 } else if flags & 2 != 0 { r - tw } else { l }; // DT_CENTER / DT_RIGHT
    let y = if flags & 4 != 0 { t + (b - t - lh) / 2 } else { t }; // DT_VCENTER
    gdi32::with_dc_target(hdc, |px, w, h| {
        let mut bf = Buf { px, w, h };
        if let Some(c) = bk {
            bf.fill(x, y, x + tw, y + lh, gdi32::colorref_to_px(c));
        }
        match &font {
            Some(f) => {
                f.draw(bf.px, bf.w, bf.h, x, y, &label, gdi32::colorref_to_px(fg));
            }
            None => {
                bf.text(x, y, &label, gdi32::colorref_to_px(fg), bold);
            }
        }
    });
    lh
}

// ---- input and miscellaneous (stubs until real input exists) ------------------------------

#[allow(non_snake_case)]
pub extern "stdcall" fn MessageBeep(_kind: u32) -> i32 {
    crate::audio::beep();
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SetCursor(_cursor: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn ShowCursor(_show: i32) -> i32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn WinHelpA(_hwnd: u32, _file: u32, _cmd: u32, _data: u32) -> i32 {
    0
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn LoadBitmapA(inst: u32, name: u32) -> u32 {
    let base = if inst == 0 { super::kernel32::image_base() } else { inst };
    let Some((_, data, _)) = resources::find(base, &resources::ResId::Id(2), &resources::res_id(name)) else { return 0 }; // RT_BITMAP
    match gdi32::decode_dib(data, None) {
        Some((w, h, px)) => gdi32::create_bitmap(w, h, px),
        None => 0,
    }
}

// ---- window geometry and attributes -------------------------------------------------------

/// Origin (in screen coordinates) of the client area of `hwnd`.
pub fn client_origin(s: &State, hwnd: u32) -> (i32, i32) {
    let Some(w) = s.windows.get(&hwnd) else { return (0, 0) };
    if w.style & WS_CHILD != 0 {
        let (px, py) = client_origin(s, w.parent);
        (px + w.x, py + w.y)
    } else if w.style & WS_CAPTION == WS_CAPTION {
        (w.x + BORDER, w.y + BORDER + CAPTION_H - 1 + if w.menu != 0 { MENU_H } else { 0 })
    } else {
        let f = dlg_frame(w.style);
        (w.x + f, w.y + f)
    }
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn GetWindowRect(hwnd: u32, r: *mut i32) -> i32 {
    if hwnd == 1 {
        // desktop
        *r = 0;
        *r.add(1) = 0;
        *r.add(2) = desktop_w() as i32;
        *r.add(3) = desktop_h() as i32;
        return 1;
    }
    let s = STATE.lock().unwrap();
    let Some(w) = s.windows.get(&hwnd) else { return 0 };
    let (x, y) = if w.style & WS_CHILD != 0 { client_origin(&s, w.parent) } else { (0, 0) };
    *r = x + w.x;
    *r.add(1) = y + w.y;
    *r.add(2) = x + w.x + w.w;
    *r.add(3) = y + w.y + w.h;
    1
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn ScreenToClient(hwnd: u32, pt: *mut i32) -> i32 {
    let (ox, oy) = client_origin(&STATE.lock().unwrap(), hwnd);
    *pt -= ox;
    *pt.add(1) -= oy;
    1
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn ClientToScreen(hwnd: u32, pt: *mut i32) -> i32 {
    let (ox, oy) = client_origin(&STATE.lock().unwrap(), hwnd);
    *pt += ox;
    *pt.add(1) += oy;
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn DestroyWindow(hwnd: u32) -> i32 {
    if !STATE.lock().unwrap().windows.contains_key(&hwnd) {
        return 0;
    }
    destroy(hwnd);
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SetWindowPos(hwnd: u32, _after: u32, x: i32, y: i32, cx: i32, cy: i32, flags: u32) -> i32 {
    let (cur, vis) = {
        let s = STATE.lock().unwrap();
        let Some(w) = s.windows.get(&hwnd) else { return 0 };
        ((w.x, w.y, w.w, w.h), w.visible)
    };
    let (nx, ny) = if flags & 2 != 0 { (cur.0, cur.1) } else { (x, y) }; // SWP_NOMOVE
    let (nw, nh) = if flags & 1 != 0 { (cur.2, cur.3) } else { (cx, cy) }; // SWP_NOSIZE
    MoveWindow(hwnd, nx, ny, nw, nh, 1);
    if flags & 0x40 != 0 && !vis { ShowWindow(hwnd, 1); } // SWP_SHOWWINDOW
    if flags & 0x80 != 0 { ShowWindow(hwnd, 0); }         // SWP_HIDEWINDOW
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetWindowLongA(hwnd: u32, idx: i32) -> u32 {
    let s = STATE.lock().unwrap();
    let Some(w) = s.windows.get(&hwnd) else { return 0 };
    match idx {
        -4 => {
            let p = w.dlgproc.max(w.proc_); // GWL_WNDPROC
            if p == 0 { builtin_wndproc as usize as u32 } else { p }
        }
        -16 => w.style,               // GWL_STYLE
        -12 => w.id,                  // GWL_ID
        -8 => w.parent,               // GWL_HWNDPARENT
        i => w.longs.get(&i).copied().unwrap_or(0),
    }
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SetWindowLongA(hwnd: u32, idx: i32, v: u32) -> u32 {
    let mut s = STATE.lock().unwrap();
    let Some(w) = s.windows.get_mut(&hwnd) else { return 0 };
    match idx {
        -4 => std::mem::replace(&mut w.proc_, v),
        -16 => std::mem::replace(&mut w.style, v),
        -12 => std::mem::replace(&mut w.id, v),
        i => w.longs.insert(i, v).unwrap_or(0),
    }
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetParent(hwnd: u32) -> u32 {
    STATE.lock().unwrap().windows.get(&hwnd).map_or(0, |w| w.parent)
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetWindow(hwnd: u32, cmd: u32) -> u32 {
    let s = STATE.lock().unwrap();
    match cmd {
        5 => s.windows.iter().find(|(_, w)| w.parent == hwnd).map_or(0, |(&h, _)| h), // GW_CHILD
        4 => s.windows.get(&hwnd).map_or(0, |w| w.parent),                           // GW_OWNER
        _ => 0,
    }
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetDesktopWindow() -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn EnableWindow(hwnd: u32, enable: i32) -> i32 {
    STATE.lock().unwrap().windows.get_mut(&hwnd).map_or(0, |w| std::mem::replace(&mut w.enabled, enable != 0) as i32 ^ 1)
}

#[allow(non_snake_case)]
pub extern "stdcall" fn IsWindowEnabled(hwnd: u32) -> i32 {
    STATE.lock().unwrap().windows.get(&hwnd).is_some_and(|w| w.enabled) as i32
}

#[allow(non_snake_case)]
pub extern "stdcall" fn CallWindowProcA(proc_: u32, hwnd: u32, msg: u32, w: u32, l: u32) -> u32 {
    if proc_ == 0 { DefWindowProcA(hwnd, msg, w, l) } else { unsafe { call4(proc_, hwnd, msg, w, l) } }
}

#[allow(non_snake_case)]
pub extern "stdcall" fn ValidateRect(hwnd: u32, _rect: u32) -> i32 {
    if let Some(w) = STATE.lock().unwrap().windows.get_mut(&hwnd) {
        w.need_paint = false;
    }
    1
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn InflateRect(r: *mut i32, dx: i32, dy: i32) -> i32 {
    *r -= dx;
    *r.add(1) -= dy;
    *r.add(2) += dx;
    *r.add(3) += dy;
    1
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn GetClassNameA(hwnd: u32, buf: *mut u8, max: i32) -> i32 {
    let c = STATE.lock().unwrap().windows.get(&hwnd).map(|w| w.class.clone()).unwrap_or_default();
    let n = c.len().min((max.max(1) - 1) as usize);
    std::ptr::copy_nonoverlapping(c.as_ptr(), buf, n);
    *buf.add(n) = 0;
    n as i32
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn IsDialogMessageA(dlg: u32, msg: *const u32) -> i32 {
    // If the message belongs to the dialog or a descendant, dispatches it and returns TRUE (as Windows does).
    let (hwnd, m, w, l) = (*msg, *msg.add(1), *msg.add(2), *msg.add(3));
    if hwnd == 0 || (hwnd != dlg && IsChild(dlg, hwnd) == 0) {
        return 0;
    }
    send(hwnd, m, w, l);
    1
}

// ---- menus --------------------------------------------------------------------------------

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn LoadMenuA(inst: u32, name: u32) -> u32 {
    let base = if inst == 0 { super::kernel32::image_base() } else { inst };
    menu::load(base, &resources::res_id(name))
}

#[allow(non_snake_case)]
pub extern "stdcall" fn DestroyMenu(h: u32) -> i32 {
    menu::destroy(h) as i32
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SetMenu(hwnd: u32, hmenu: u32) -> i32 {
    let (x, y, w, h);
    {
        let mut s = STATE.lock().unwrap();
        let Some(win) = s.windows.get_mut(&hwnd) else { return 0 };
        win.menu = hmenu;
        (x, y, w, h) = (win.x, win.y, win.w, win.h);
    }
    MoveWindow(hwnd, x, y, w, h, 1); // recalculates the client area
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetMenu(hwnd: u32) -> u32 {
    STATE.lock().unwrap().windows.get(&hwnd).map_or(0, |w| w.menu)
}

/// Previous state of the item (flags) or -1 if it does not exist.
fn item_flags(menu_h: u32, id: u32, flags: u32, set: u32, mask: u32) -> i32 {
    menu::with_item(menu_h, id, flags & menu::MF_BYPOSITION != 0, |it| {
        let prev = it.flags & mask;
        it.flags = (it.flags & !mask) | (set & mask);
        prev as i32
    })
    .unwrap_or(-1)
}

#[allow(non_snake_case)]
pub extern "stdcall" fn EnableMenuItem(m: u32, id: u32, flags: u32) -> i32 {
    let mask = menu::MF_GRAYED | menu::MF_DISABLED;
    item_flags(m, id, flags, flags, mask)
}

#[allow(non_snake_case)]
pub extern "stdcall" fn CheckMenuItem(m: u32, id: u32, flags: u32) -> i32 {
    item_flags(m, id, flags, flags, menu::MF_CHECKED)
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn ModifyMenuA(m: u32, id: u32, flags: u32, new_id: u32, new_item: u32) -> i32 {
    let text = if flags & (menu::MF_SEPARATOR | 0x100 | 0x4) == 0 && new_item != 0 { Some(cstr(new_item)) } else { None };
    menu::with_item(m, id, flags & menu::MF_BYPOSITION != 0, |it| {
        if let Some(t) = text { it.text = t }
        if it.sub == 0 { it.id = new_id }
    })
    .is_some() as i32
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetSubMenu(m: u32, pos: i32) -> u32 {
    menu::items(m).get(pos as usize).map_or(0, |i| i.sub)
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetMenuItemCount(m: u32) -> i32 {
    if menu::exists(m) { menu::items(m).len() as i32 } else { -1 }
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetMenuItemID(m: u32, pos: i32) -> u32 {
    menu::items(m).get(pos as usize).map_or(u32::MAX, |i| if i.sub != 0 { u32::MAX } else { i.id })
}

// ---- rectangles, props, hooks and miscellaneous -------------------------------------------

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn AdjustWindowRect(r: *mut i32, style: u32, has_menu: i32) -> i32 {
    if style & WS_CAPTION == WS_CAPTION {
        *r -= BORDER;
        *r.add(1) -= BORDER + CAPTION_H + if has_menu != 0 { MENU_H } else { 0 };
        *r.add(2) += BORDER;
        *r.add(3) += BORDER;
    }
    1
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn AdjustWindowRectEx(r: *mut i32, style: u32, has_menu: i32, _ex: u32) -> i32 {
    AdjustWindowRect(r, style, has_menu)
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn SetPropA(hwnd: u32, name: u32, data: u32) -> i32 {
    STATE.lock().unwrap().props.insert((hwnd, cstr(name)), data);
    1
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn GetPropA(hwnd: u32, name: u32) -> u32 {
    STATE.lock().unwrap().props.get(&(hwnd, cstr(name))).copied().unwrap_or(0)
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn RemovePropA(hwnd: u32, name: u32) -> u32 {
    STATE.lock().unwrap().props.remove(&(hwnd, cstr(name))).unwrap_or(0)
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SetWindowsHookExA(_id: i32, _proc: u32, _inst: u32, _tid: u32) -> u32 {
    0x5001 // hooks are not called yet
}

#[allow(non_snake_case)]
pub extern "stdcall" fn UnhookWindowsHookEx(_h: u32) -> i32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn CallNextHookEx(_h: u32, _code: i32, _w: u32, _l: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn OemToCharA(src: *const u8, dst: *mut u8) -> i32 {
    let n = std::ffi::CStr::from_ptr(src as *const i8).to_bytes().len();
    std::ptr::copy(src, dst, n + 1);
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GrayStringA(_hdc: u32, _brush: u32, _cb: u32, _data: u32, _n: i32, _x: i32, _y: i32, _w: i32, _h: i32) -> i32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn TabbedTextOutA(_hdc: u32, _x: i32, _y: i32, _s: u32, _n: i32, _tabs: i32, _pos: u32, _org: i32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn IsWindowVisible(hwnd: u32) -> i32 {
    STATE.lock().unwrap().windows.get(&hwnd).is_some_and(|w| w.visible) as i32
}

#[allow(non_snake_case)]
pub extern "stdcall" fn IsWindow(hwnd: u32) -> i32 {
    STATE.lock().unwrap().windows.contains_key(&hwnd) as i32
}

#[allow(non_snake_case)]
pub extern "stdcall" fn IsChild(parent: u32, child: u32) -> i32 {
    let s = STATE.lock().unwrap();
    let mut cur = child;
    while let Some(w) = s.windows.get(&cur) {
        if w.parent == parent && parent != 0 {
            return 1;
        }
        cur = w.parent;
    }
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetWindowTextLengthA(hwnd: u32) -> i32 {
    STATE.lock().unwrap().windows.get(&hwnd).map_or(0, |w| w.title.len() as i32)
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetDlgCtrlID(hwnd: u32) -> i32 {
    STATE.lock().unwrap().windows.get(&hwnd).map_or(0, |w| w.id as i32)
}

/// WndProc of the native classes (button, edit...), available to the program for subclassing.
extern "stdcall" fn builtin_wndproc(hwnd: u32, msg: u32, w: u32, l: u32) -> u32 {
    builtin_proc(hwnd, msg, w, l)
}

const BUILTIN_CLASSES: [&str; 8] = ["button", "edit", "static", "listbox", "combobox", "scrollbar", "#32770", "mdiclient"];

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn GetClassInfoA(_inst: u32, name: u32, wc: *mut u32) -> i32 {
    let n = cstr(name);
    let raw = {
        let s = STATE.lock().unwrap();
        s.classes.get(&n).map(|c| c.raw)
    };
    if let Some(mut raw) = raw {
        raw[9] = name; // lpszClassName points to the requested name
        std::ptr::copy_nonoverlapping(raw.as_ptr(), wc, 10);
        return 1;
    }
    if BUILTIN_CLASSES.contains(&n.to_ascii_lowercase().as_str()) {
        let raw = [0u32, builtin_wndproc as usize as u32, 0, 0, 0, 0, 0, 6, 0, name]; // hbrBackground = COLOR_WINDOW+1
        std::ptr::copy_nonoverlapping(raw.as_ptr(), wc, 10);
        return 1;
    }
    0
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn CopyRect(dst: *mut i32, src: *const i32) -> i32 {
    std::ptr::copy_nonoverlapping(src, dst, 4);
    1
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn IntersectRect(dst: *mut i32, a: *const i32, b: *const i32) -> i32 {
    let (l, t) = ((*a).max(*b), (*a.add(1)).max(*b.add(1)));
    let (r, bt) = ((*a.add(2)).min(*b.add(2)), (*a.add(3)).min(*b.add(3)));
    if l < r && t < bt {
        *dst = l;
        *dst.add(1) = t;
        *dst.add(2) = r;
        *dst.add(3) = bt;
        1
    } else {
        std::ptr::write_bytes(dst, 0, 4);
        0
    }
}

// ---- ANSI strings, rectangles, dynamic menus ----------------------------------------------

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn CharNextA(p: *const u8) -> *const u8 {
    if *p == 0 { p } else { p.add(1) }
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn CharPrevA(start: *const u8, cur: *const u8) -> *const u8 {
    if cur > start { cur.sub(1) } else { start }
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn CharLowerA(p: u32) -> u32 {
    if p < 0x10000 { return (p as u8).to_ascii_lowercase() as u32 } // single character passed by value
    let mut q = p as usize as *mut u8;
    while *q != 0 { *q = (*q).to_ascii_lowercase(); q = q.add(1) }
    p
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn CharUpperA(p: u32) -> u32 {
    if p < 0x10000 { return (p as u8).to_ascii_uppercase() as u32 }
    let mut q = p as usize as *mut u8;
    while *q != 0 { *q = (*q).to_ascii_uppercase(); q = q.add(1) }
    p
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn CharUpperBuffA(p: *mut u8, n: u32) -> u32 {
    for i in 0..n as usize { *p.add(i) = (*p.add(i)).to_ascii_uppercase() }
    n
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn CharLowerBuffA(p: *mut u8, n: u32) -> u32 {
    for i in 0..n as usize { *p.add(i) = (*p.add(i)).to_ascii_lowercase() }
    n
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn CharUpperBuffW(p: *mut u16, n: u32) -> u32 {
    for i in 0..n as usize {
        let c = *p.add(i);
        if c < 0x80 { *p.add(i) = (c as u8).to_ascii_uppercase() as u16 }
    }
    n
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn CharLowerBuffW(p: *mut u16, n: u32) -> u32 {
    for i in 0..n as usize {
        let c = *p.add(i);
        if c < 0x80 { *p.add(i) = (c as u8).to_ascii_lowercase() as u16 }
    }
    n
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn CharToOemA(src: *const u8, dst: *mut u8) -> i32 {
    OemToCharA(src, dst) // same copy: no code page conversion
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn CharToOemBuffA(src: *const u8, dst: *mut u8, n: u32) -> i32 {
    std::ptr::copy(src, dst, n as usize);
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetWindowDC(hwnd: u32) -> u32 {
    gdi32::dc_for_window(hwnd)
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn SetRect(r: *mut i32, l: i32, t: i32, rt: i32, b: i32) -> i32 {
    *r = l;
    *r.add(1) = t;
    *r.add(2) = rt;
    *r.add(3) = b;
    1
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn IsRectEmpty(r: *const i32) -> i32 {
    (*r >= *r.add(2) || *r.add(1) >= *r.add(3)) as i32
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn OffsetRect(r: *mut i32, dx: i32, dy: i32) -> i32 {
    *r += dx;
    *r.add(2) += dx;
    *r.add(1) += dy;
    *r.add(3) += dy;
    1
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn PtInRect(r: *const i32, x: i32, y: i32) -> i32 {
    (x >= *r && x < *r.add(2) && y >= *r.add(1) && y < *r.add(3)) as i32
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn FrameRect(hdc: u32, r: *const i32, brush: u32) -> i32 {
    let color = if (1..=31).contains(&brush) { Some(gdi32::sys_color(brush - 1)) } else { gdi32::brush_color(brush) };
    let Some(c) = color else { return 1 };
    let (l, t, rt, b) = (*r, *r.add(1), *r.add(2), *r.add(3));
    let px = gdi32::colorref_to_px(c);
    gdi32::with_dc_target(hdc, |buf, w, h| {
        let mut bf = Buf { px: buf, w, h };
        bf.fill(l, t, rt, t + 1, px);
        bf.fill(l, b - 1, rt, b, px);
        bf.fill(l, t, l + 1, b, px);
        bf.fill(rt - 1, t, rt, b, px);
    });
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn CreatePopupMenu() -> u32 {
    menu::create_empty()
}

#[allow(non_snake_case)]
pub extern "stdcall" fn CreateMenu() -> u32 {
    menu::create_empty()
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn AppendMenuA(m: u32, flags: u32, id: u32, text: u32) -> i32 {
    // MF_STRING = 0; for separators/owner-draw `text` is not a string
    let label = if flags & (menu::MF_SEPARATOR | 0x100 | 0x4) == 0 && text != 0 { cstr(text) } else { String::new() };
    menu::append(m, flags, id, label) as i32
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SetParent(hwnd: u32, parent: u32) -> u32 {
    STATE.lock().unwrap().windows.get_mut(&hwnd).map_or(0, |w| std::mem::replace(&mut w.parent, parent))
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn GetWindowThreadProcessId(_hwnd: u32, pid: *mut u32) -> u32 {
    if !pid.is_null() {
        *pid = std::process::id();
    }
    super::sync::current_tid()
}

#[allow(non_snake_case)]
pub extern "stdcall" fn WaitMessage() -> i32 {
    if next_message(false).is_none() {
        idle();
    }
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetTopWindow(hwnd: u32) -> u32 {
    STATE.lock().unwrap().windows.iter().find(|(_, w)| w.parent == hwnd && w.style & WS_CHILD != 0).map_or(0, |(&h, _)| h)
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetDialogBaseUnits() -> u32 {
    let (x, y) = dlu();
    ((y as u32) << 16) | x as u32
}

/// Replaces the WndProc of a window; returns the previous one. `is16` marks a 16-bit far pointer.
pub fn set_wndproc(hwnd: u32, f: u32, is16: bool) -> u32 {
    let mut s = STATE.lock().unwrap();
    match s.windows.get_mut(&hwnd) {
        Some(w) => {
            w.is16 = is16;
            std::mem::replace(&mut w.proc_, f)
        }
        None => 0,
    }
}

/// Class name of a window ("" if it does not exist).
pub fn class_of(hwnd: u32) -> String {
    STATE.lock().unwrap().windows.get(&hwnd).map(|w| w.class.clone()).unwrap_or_default()
}

/// Gives initial focus to a dialog control (without sending messages).
pub fn set_focus_hint(hwnd: u32) {
    STATE.lock().unwrap().focus = hwnd;
}
