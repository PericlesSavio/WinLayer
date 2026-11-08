//! Windows 3.x USER.EXE (windows, messages, dialogs, menus), on top of `win32::user32`.
//!
//! Differences handled here: 16-bit handles and parameters, structures (RECT, MSG, WNDCLASS,
//! PAINTSTRUCT, CREATESTRUCT...) with 16-bit fields, 16-bit `wParam` and far pointers.

use super::api::{far_to_linear, Call, Impl};
use super::ldt::{self, Kind};
use super::ne::ResKey;
use super::{kernel, kernel_ext, trans};
use crate::win32::{gdi32, menu, user32};

pub fn pack(ax: u16, dx: u16) -> u32 {
    ax as u32 | (dx as u32) << 16
}

unsafe fn rd16(p: u32) -> i16 {
    std::ptr::read_unaligned(p as usize as *const i16)
}
unsafe fn rdu16(p: u32) -> u16 {
    std::ptr::read_unaligned(p as usize as *const u16)
}
unsafe fn rd32(p: u32) -> u32 {
    std::ptr::read_unaligned(p as usize as *const u32)
}
unsafe fn wr16(p: u32, v: u16) {
    std::ptr::write_unaligned(p as usize as *mut u16, v)
}
unsafe fn wr32(p: u32, v: u32) {
    std::ptr::write_unaligned(p as usize as *mut u32, v)
}

pub unsafe fn rect16(p: u32) -> [i32; 4] {
    [rd16(p) as i32, rd16(p + 2) as i32, rd16(p + 4) as i32, rd16(p + 6) as i32]
}

pub unsafe fn put_rect16(p: u32, r: [i32; 4]) {
    for (i, v) in r.iter().enumerate() {
        wr16(p + i as u32 * 2, *v as u16);
    }
}

unsafe fn cstr_at(p: u32) -> String {
    if p == 0 { String::new() } else { std::ffi::CStr::from_ptr(p as usize as *const i8).to_string_lossy().into_owned() }
}

fn dgroup() -> u16 {
    kernel::with_process(|p| p.dgroup_sel)
}

// ---- message conversion between Win32 (user32) and Win16 ------------------------------------

/// Resources allocated for a single call only (global blocks and temporary selectors).
#[derive(Default)]
struct Scratch {
    sels: Vec<u16>,
    bufs: Vec<Box<[u8]>>,
}

impl Scratch {
    fn far_over(&mut self, base: u32, len: u32) -> u32 {
        let lim = len.clamp(1, 0x1_0000) - 1;
        match ldt::alloc(base, lim, Kind::Data16) {
            Ok(sel) => {
                self.sels.push(sel);
                (sel as u32) << 16
            }
            Err(_) => 0,
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        for s in &self.sels {
            ldt::release(*s);
        }
    }
}

const WM_CREATE: u32 = 0x01;
const WM_NCCREATE: u32 = 0x81;
const WM_SETTEXT: u32 = 0x0C;
const WM_GETTEXT: u32 = 0x0D;
const WM_DRAWITEM: u32 = 0x2B;
const WM_COMMAND: u32 = 0x111;
const WM_HSCROLL: u32 = 0x114;
const WM_VSCROLL: u32 = 0x115;
const WM_TIMER: u32 = 0x113;

/// Win32 (as user32 sends it) -> arguments of a 16-bit WndProc: (msg, wParam, lParam).
fn to16(msg: u32, w: u32, l: u32, sc: &mut Scratch) -> (u16, u16, u32) {
    match msg {
        WM_COMMAND => ((msg) as u16, w as u16, ((w >> 16) << 16) | (l & 0xFFFF)),
        WM_VSCROLL | WM_HSCROLL => (msg as u16, w as u16, (l << 16) | (w >> 16)),
        WM_CREATE | WM_NCCREATE if l != 0 => {
            // 16-bit CREATESTRUCT (34 bytes) in a temporary block
            let Some(h) = kernel::global_alloc(64) else { return (msg as u16, w as u16, 0) };
            let (base, _) = kernel::global_info(h).unwrap();
            let s32 = l as usize as *const u32;
            unsafe {
                // lpCreateParams, hInstance, hMenu, hwndParent, cy, cx, y, x, style, lpszName, lpszClass, dwExStyle
                wr32(base, *s32);
                wr16(base + 4, dgroup());
                wr16(base + 6, *s32.add(2) as u16);
                wr16(base + 8, *s32.add(3) as u16);
                wr16(base + 10, *s32.add(4) as u16);
                wr16(base + 12, *s32.add(5) as u16);
                wr16(base + 14, *s32.add(6) as u16);
                wr16(base + 16, *s32.add(7) as u16);
                wr32(base + 18, *s32.add(8));
                wr32(base + 22, 0);
                wr32(base + 26, 0);
                wr32(base + 30, 0);
            }
            sc.sels.push(h); // freed by Drop? (it is a global block: freed by the caller)
            (msg as u16, w as u16, (h as u32) << 16)
        }
        WM_DRAWITEM if l != 0 => {
            // 16-bit DRAWITEMSTRUCT (22 bytes): CtlType, CtlID, itemID, itemAction, itemState, hwndItem, hDC (words), rcItem (4 words), itemData (dword)
            let s32 = l as usize as *const u32;
            let mut b = vec![0u8; 22].into_boxed_slice();
            unsafe {
                let words = [*s32, *s32.add(1), *s32.add(2), *s32.add(3), *s32.add(4), *s32.add(5), *s32.add(6), *s32.add(7), *s32.add(8), *s32.add(9), *s32.add(10)];
                for (i, v) in words.iter().enumerate() {
                    b[i * 2..i * 2 + 2].copy_from_slice(&(*v as u16).to_le_bytes());
                }
                b[18..22].copy_from_slice(&(*s32.add(11)).to_le_bytes());
            }
            let ptr = b.as_ptr() as u32;
            sc.bufs.push(b);
            (msg as u16, w as u16, sc.far_over(ptr, 22))
        }
        WM_GETTEXT if l != 0 => (msg as u16, w as u16, sc.far_over(l, w & 0xFFFF)),
        WM_SETTEXT if l != 0 => {
            let n = unsafe { std::ffi::CStr::from_ptr(l as usize as *const i8).to_bytes().len() } as u32 + 1;
            (msg as u16, w as u16, sc.far_over(l, n))
        }
        _ => (msg as u16, w as u16, l),
    }
}

/// Arguments from a Win16 program (SendMessage, DefWindowProc...) -> user32 semantics.
fn to32(hwnd: u32, msg: u32, w: u16, l: u32) -> (u32, u32, u32) {
    let w = w as u32;
    match msg {
        WM_COMMAND => (msg, ((l >> 16) << 16) | w, l & 0xFFFF),
        WM_VSCROLL | WM_HSCROLL => (msg, ((l & 0xFFFF) << 16) | w, l >> 16),
        WM_GETTEXT | WM_SETTEXT => (msg, w, far_to_linear(l)),
        0x400..=0x403 if user32::class_of(hwnd).eq_ignore_ascii_case("button") => (msg - 0x400 + 0xF0, w, l), // BM_*
        0x401..=0x423 if user32::class_of(hwnd).eq_ignore_ascii_case("listbox") => {
            // LB_*: Win16 WM_USER+n becomes 0x180+n-1; lParam is a far pointer to text/buffer in these messages
            let m32 = msg - 0x401 + 0x180;
            let ptr = matches!(m32, 0x180 | 0x181 | 0x189 | 0x18C | 0x18F | 0x1A2);
            (m32, w, if ptr { far_to_linear(l) } else { l })
        }
        _ => (msg, w, l),
    }
}

/// `user32` hook: calls a 16-bit WndProc/DlgProc.
pub fn call_proc16(far: u32, hwnd: u32, msg: u32, w: u32, l: u32) -> u32 {
    let mut sc = Scratch::default();
    let (m16, w16, l16) = to16(msg, w, l, &mut sc);
    // CREATESTRUCT global block, if any (the selector is in sc.sels as a "block", not temporary)
    let created: Option<u16> = if matches!(msg, WM_CREATE | WM_NCCREATE) && l != 0 { sc.sels.pop() } else { None };
    let r = trans_call(far, &[hwnd as u16, m16, w16, (l16 >> 16) as u16, l16 as u16]);
    if let Some(h) = created {
        kernel::global_free_pub(h);
    }
    r
}

/// Calls a 16-bit procedure given by a far pointer; returns DX:AX.
pub fn trans_call(far: u32, args: &[u16]) -> u32 {
    let rt = super::rt();
    let (ax, dx) = trans::call16(&rt.thunks, &mut rt.stack, (far >> 16) as u16, far as u16, dgroup(), args);
    pack(ax, dx)
}

// ---- 16-bit menus ---------------------------------------------------------------------

fn parse_menu16(b: &[u8], mut p: usize) -> (Vec<menu::Item>, usize) {
    let mut items = Vec::new();
    loop {
        if p + 2 > b.len() {
            break;
        }
        let flags = u16::from_le_bytes([b[p], b[p + 1]]) as u32;
        p += 2;
        let popup = flags & menu::MF_POPUP != 0;
        let mut id = 0;
        if !popup {
            id = u16::from_le_bytes([b[p], b[p + 1]]) as u32;
            p += 2;
        }
        let end = b[p..].iter().position(|&c| c == 0).map_or(b.len(), |i| p + i);
        let text = String::from_utf8_lossy(&b[p..end]).into_owned();
        p = end + 1;
        let item = if popup {
            let (sub, np) = parse_menu16(b, p);
            p = np;
            menu::Item { id: 0, text, flags, sub: menu::add(sub) }
        } else if id == 0 && text.is_empty() {
            menu::Item { id: 0, text, flags: flags | menu::MF_SEPARATOR, sub: 0 }
        } else {
            menu::Item { id, text, flags, sub: 0 }
        };
        items.push(item);
        if flags & menu::MF_END != 0 {
            break;
        }
    }
    (items, p)
}

/// Loads menu `name` (RT_MENU 4) from the NE resources; returns the HMENU.
pub fn load_menu16(raw_name: u32) -> u32 {
    let Some(b) = kernel_ext::resource_bytes(&ResKey::Id(4), &kernel_ext::key_of_far(raw_name)) else { return 0 };
    let skip = if b.len() >= 4 { 4 + u16::from_le_bytes([b[2], b[3]]) as usize } else { return 0 };
    let (items, _) = parse_menu16(&b, skip);
    menu::add(items)
}

// ---- 16-bit dialogs -------------------------------------------------------------------

unsafe fn skip_name(p: u32) -> u32 {
    match *(p as usize as *const u8) {
        0 => p + 1,
        0xFF => p + 3,
        _ => p + std::ffi::CStr::from_ptr(p as usize as *const i8).to_bytes().len() as u32 + 1,
    }
}

unsafe fn create_dialog16(tmpl: u32, parent: u32, dlgproc: u32, param: u32) -> u32 {
    let style = rd32(tmpl);
    let cdit = *((tmpl + 4) as usize as *const u8) as usize;
    let (dcx, dcy) = (rd16(tmpl + 9) as i32, rd16(tmpl + 11) as i32);
    let mut p = skip_name(tmpl + 13); // menu
    p = skip_name(p); // class
    let title = cstr_at(p);
    p += title.len() as u32 + 1;
    if style & 0x40 != 0 {
        p += 2;
        p += cstr_at(p).len() as u32 + 1; // font
    }
    if std::env::var_os("WINLAYER_WIN16_TRACE").is_some() {
        eprintln!("[win16] dialog: style={style:#x} items={cdit} {dcx}x{dcy} title={title:?}");
    }
    let (ux, uy) = user32::dlu();
    let (cw, ch) = (dcx * ux / 4, dcy * uy / 8);
    let caption = style & user32::WS_CAPTION == user32::WS_CAPTION;
    let f = user32::dlg_frame(style);
    let (w, h) = if caption { (cw + 2 * user32::BORDER, ch + user32::CAPTION_H + 2 * user32::BORDER) } else { (cw + 2 * f, ch + 2 * f) };
    let (x, y) = ((user32::desktop_w() as i32 - w) / 2, (user32::desktop_h() as i32 - h) / 2);
    user32::DLG16.store(true, std::sync::atomic::Ordering::Relaxed);
    let dlg = user32::create_window(user32::DIALOG_CLASS.into(), title, style | user32::WS_VISIBLE, x, y, w, h, parent, 0, 0, param, dlgproc);
    user32::DLG16.store(false, std::sync::atomic::Ordering::Relaxed);

    let (mut first, mut first_tab) = (0, 0);
    for _ in 0..cdit {
        let (ix, iy, icx, icy, id) = (rd16(p) as i32, rd16(p + 2) as i32, rd16(p + 4) as i32, rd16(p + 6) as i32, rdu16(p + 8) as u32);
        let istyle = rd32(p + 10);
        p += 14;
        let b0 = *(p as usize as *const u8);
        let class = if b0 & 0x80 != 0 {
            p += 1;
            match b0 {
                0x80 => "button", 0x81 => "edit", 0x82 => "static", 0x83 => "listbox", 0x84 => "scrollbar", 0x85 => "combobox", _ => "static",
            }
            .to_string()
        } else {
            let c = cstr_at(p);
            p += c.len() as u32 + 1;
            c
        };
        let text = if *(p as usize as *const u8) == 0xFF {
            p += 3;
            String::new()
        } else {
            let t = cstr_at(p);
            p += t.len() as u32 + 1;
            t
        };
        p += 1 + *(p as usize as *const u8) as u32; // extra data (1-byte counter)
        if std::env::var_os("WINLAYER_WIN16_TRACE").is_some() {
            eprintln!("[win16]   item id={id} class={class:?} style={istyle:#x} text={text:?} @{ix},{iy} {icx}x{icy}");
        }
        let child = user32::create_window(
            class, text, istyle | user32::WS_CHILD, ix * ux / 4, iy * uy / 8,
            icx * ux / 4, icy * uy / 8, dlg, id, 0, 0, 0,
        );
        if first == 0 { first = child }
        if first_tab == 0 && istyle & user32::WS_TABSTOP != 0 { first_tab = child }
    }
    let target = if first_tab != 0 { first_tab } else { first };
    if user32::send(dlg, 0x110, target, param) != 0 && target != 0 {
        user32::set_focus_hint(target); // WM_INITDIALOG returned TRUE: default focus
    }
    dlg
}

// ---- handlers -----------------------------------------------------------------------------

fn msgbox(c: &mut Call) -> u32 {
    unsafe { user32::MessageBoxA(c.w(0) as u32, c.ptr(1), c.ptr(2), c.w(3) as u32) as u32 }
}

fn get_desktop_window(_c: &mut Call) -> u32 {
    1
}

fn kill_timer(c: &mut Call) -> u32 {
    user32::KillTimer(c.w(0) as u32, c.w(1) as u32) as u32
}

fn set_timer(c: &mut Call) -> u32 {
    user32::SetTimer(c.w(0) as u32, c.w(1) as u32, c.w(2) as u32, c.l(3)) // raw far procedure
}

fn set_focus(c: &mut Call) -> u32 {
    user32::SetFocus(c.w(0) as u32)
}

fn set_capture(c: &mut Call) -> u32 {
    user32::SetCapture(c.w(0) as u32)
}

fn release_capture(_c: &mut Call) -> u32 {
    user32::ReleaseCapture() as u32
}

fn set_cursor(_c: &mut Call) -> u32 {
    0x7001
}

fn load_cursor(_c: &mut Call) -> u32 {
    0x7001
}

fn load_icon(_c: &mut Call) -> u32 {
    0x7002
}

fn message_beep(_c: &mut Call) -> u32 {
    crate::audio::beep();
    1
}

fn win_help(_c: &mut Call) -> u32 {
    1
}

fn get_cursor_pos(c: &mut Call) -> u32 {
    let p = c.ptr(0);
    let mut pt = [0i32; 2];
    unsafe {
        user32::GetCursorPos(pt.as_mut_ptr());
        wr16(p, pt[0] as u16);
        wr16(p + 2, pt[1] as u16);
    }
    0
}

fn screen_to_client(c: &mut Call) -> u32 {
    let (hwnd, p) = (c.w(0) as u32, c.ptr(1));
    unsafe {
        let mut pt = [rd16(p) as i32, rd16(p + 2) as i32];
        user32::ScreenToClient(hwnd, pt.as_mut_ptr());
        wr16(p, pt[0] as u16);
        wr16(p + 2, pt[1] as u16);
    }
    0
}

fn get_window_rect(c: &mut Call) -> u32 {
    let (hwnd, p) = (c.w(0) as u32, c.ptr(1));
    let mut r = [0i32; 4];
    unsafe {
        user32::GetWindowRect(hwnd, r.as_mut_ptr());
        put_rect16(p, r);
    }
    0
}

fn get_client_rect(c: &mut Call) -> u32 {
    let (hwnd, p) = (c.w(0) as u32, c.ptr(1));
    let mut r = [0i32; 4];
    unsafe {
        user32::GetClientRect(hwnd, r.as_mut_ptr());
        put_rect16(p, r);
    }
    0
}

fn enable_window(c: &mut Call) -> u32 {
    user32::EnableWindow(c.w(0) as u32, c.w(1) as i32) as u32
}

fn is_window_enabled(c: &mut Call) -> u32 {
    user32::IsWindowEnabled(c.w(0) as u32) as u32
}

fn get_window_text(c: &mut Call) -> u32 {
    unsafe { user32::GetWindowTextA(c.w(0) as u32, c.ptr(1) as usize as *mut u8, c.w(2) as i32) as u32 }
}

fn set_window_text(c: &mut Call) -> u32 {
    unsafe { user32::SetWindowTextA(c.w(0) as u32, c.ptr(1)) as u32 }
}

fn begin_paint(c: &mut Call) -> u32 {
    let (hwnd, p) = (c.w(0) as u32, c.ptr(1));
    let mut ps = [0u32; 16];
    unsafe {
        let hdc = user32::BeginPaint(hwnd, ps.as_mut_ptr());
        std::ptr::write_bytes(p as usize as *mut u8, 0, 32);
        wr16(p, hdc as u16);
        wr16(p + 2, ps[1] as u16);
        put_rect16(p + 4, [ps[2] as i32, ps[3] as i32, ps[4] as i32, ps[5] as i32]);
        hdc
    }
}

fn end_paint(c: &mut Call) -> u32 {
    let (hwnd, p) = (c.w(0) as u32, c.ptr(1));
    let ps = [unsafe { rdu16(p) } as u32, 0, 0, 0, 0, 0, 0, 0];
    user32::EndPaint(hwnd, ps.as_ptr());
    1
}

fn get_dc(c: &mut Call) -> u32 {
    user32::GetDC(c.w(0) as u32)
}

fn release_dc(c: &mut Call) -> u32 {
    user32::ReleaseDC(c.w(0) as u32, c.w(1) as u32) as u32
}

fn invalidate_rect(c: &mut Call) -> u32 {
    unsafe { user32::InvalidateRect(c.w(0) as u32, 0, c.w(2) as i32) as u32 }
}

fn validate_rect(c: &mut Call) -> u32 {
    user32::ValidateRect(c.w(0) as u32, 0) as u32
}

fn update_window(c: &mut Call) -> u32 {
    user32::UpdateWindow(c.w(0) as u32) as u32
}

fn show_window(c: &mut Call) -> u32 {
    user32::ShowWindow(c.w(0) as u32, c.w(1) as u32) as u32
}

fn move_window(c: &mut Call) -> u32 {
    user32::MoveWindow(c.w(0) as u32, c.s(1) as i32, c.s(2) as i32, c.s(3) as i32, c.s(4) as i32, c.w(5) as i32) as u32
}

fn destroy_window(c: &mut Call) -> u32 {
    user32::DestroyWindow(c.w(0) as u32) as u32
}

fn get_parent(c: &mut Call) -> u32 {
    user32::GetParent(c.w(0) as u32)
}

fn is_child(c: &mut Call) -> u32 {
    user32::IsChild(c.w(0) as u32, c.w(1) as u32) as u32
}

fn get_dlg_item(c: &mut Call) -> u32 {
    user32::GetDlgItem(c.w(0) as u32, c.w(1) as u32)
}

fn get_dlg_ctrl_id(c: &mut Call) -> u32 {
    user32::GetDlgCtrlID(c.w(0) as u32) as u32
}

fn get_system_metrics(c: &mut Call) -> u32 {
    user32::GetSystemMetrics(c.w(0) as u32) as u32
}

fn get_dialog_base_units(_c: &mut Call) -> u32 {
    user32::GetDialogBaseUnits()
}

fn get_free_system_resources(_c: &mut Call) -> u32 {
    80
}

fn zero(_c: &mut Call) -> u32 {
    0
}

fn one(_c: &mut Call) -> u32 {
    1
}

fn set_rect(c: &mut Call) -> u32 {
    let p = c.ptr(0);
    unsafe { put_rect16(p, [c.s(1) as i32, c.s(2) as i32, c.s(3) as i32, c.s(4) as i32]) };
    1
}

fn inflate_rect(c: &mut Call) -> u32 {
    let p = c.ptr(0);
    unsafe {
        let mut r = rect16(p);
        user32::InflateRect(r.as_mut_ptr(), c.s(1) as i32, c.s(2) as i32);
        put_rect16(p, r);
    }
    1
}

fn offset_rect(c: &mut Call) -> u32 {
    let p = c.ptr(0);
    unsafe {
        let mut r = rect16(p);
        user32::OffsetRect(r.as_mut_ptr(), c.s(1) as i32, c.s(2) as i32);
        put_rect16(p, r);
    }
    1
}

fn pt_in_rect(c: &mut Call) -> u32 {
    let (p, pt) = (c.ptr(0), c.l(1));
    if p == 0 {
        return 0; // Civilization actually passes a null rectangle
    }
    let r = unsafe { rect16(p) };
    let (x, y) = ((pt as u16 as i16) as i32, ((pt >> 16) as u16 as i16) as i32);
    (x >= r[0] && x < r[2] && y >= r[1] && y < r[3]) as u32
}

fn fill_rect(c: &mut Call) -> u32 {
    let (hdc, p, brush) = (c.w(0) as u32, c.ptr(1), c.w(2) as u32);
    let r = unsafe { rect16(p) };
    unsafe { user32::FillRect(hdc, r.as_ptr(), brush) as u32 }
}

fn frame_rect(c: &mut Call) -> u32 {
    let (hdc, p, brush) = (c.w(0) as u32, c.ptr(1), c.w(2) as u32);
    let r = unsafe { rect16(p) };
    unsafe { user32::FrameRect(hdc, r.as_ptr(), brush) as u32 }
}

fn draw_text(c: &mut Call) -> u32 {
    let (hdc, text, n, p, flags) = (c.w(0) as u32, c.ptr(1), c.s(2) as i32, c.ptr(3), c.w(4) as u32);
    let mut r = unsafe { rect16(p) };
    let h = unsafe { user32::DrawTextA(hdc, text as usize as *const u8, n, r.as_mut_ptr(), flags) };
    if flags & 0x400 != 0 {
        unsafe { put_rect16(p, r) }; // DT_CALCRECT returns the rectangle
    }
    h as u32
}

fn peek_message(c: &mut Call) -> u32 {
    let (p, remove) = (c.ptr(0), c.w(4) as u32);
    let mut m = [0u32; 7];
    let got = unsafe { user32::PeekMessageA(m.as_mut_ptr(), c.w(1) as u32, c.w(2) as u32, c.w(3) as u32, remove) };
    if got == 0 {
        return 0;
    }
    if std::env::var_os("WINLAYER_WIN16_TRACE").is_some() {
        eprintln!("[win16]   PeekMessage -> hwnd={:#x} msg={:#x} w={:#x} l={:#x}", m[0], m[1], m[2], m[3]);
    }
    let mut sc = Scratch::default();
    let (m16, w16, l16) = to16(m[1], m[2], m[3], &mut sc);
    unsafe {
        wr16(p, m[0] as u16);
        wr16(p + 2, m16);
        wr16(p + 4, w16);
        wr32(p + 6, l16);
        wr32(p + 10, m[4]);
        wr16(p + 14, m[5] as u16);
        wr16(p + 16, m[6] as u16);
    }
    1
}

fn translate_message(_c: &mut Call) -> u32 {
    0
}

fn dispatch_message(c: &mut Call) -> u32 {
    let p = c.ptr(0);
    let (hwnd, msg, w, l, time) = unsafe { (rdu16(p) as u32, rdu16(p + 2) as u32, rdu16(p + 4), rd32(p + 6), rd32(p + 10)) };
    if msg == WM_TIMER && l != 0 {
        // SetTimer with TIMERPROC: calls proc(hwnd, WM_TIMER, id, time)
        return trans_call(l, &[hwnd as u16, WM_TIMER as u16, w, (time >> 16) as u16, time as u16]);
    }
    if hwnd == 0 {
        return 0;
    }
    let (m, w32, l32) = to32(hwnd, msg, w, l);
    user32::send(hwnd, m, w32, l32)
}

fn post_quit(c: &mut Call) -> u32 {
    user32::PostQuitMessage(c.w(0) as u32);
    0
}

fn send_message(c: &mut Call) -> u32 {
    let hwnd = c.w(0) as u32;
    let (m, w, l) = to32(hwnd, c.w(1) as u32, c.w(2), c.l(3));
    user32::send(hwnd, m, w, l)
}

fn send_dlg_item_message(c: &mut Call) -> u32 {
    let dlg = c.w(0) as u32;
    let hwnd = user32::GetDlgItem(dlg, c.w(1) as u32);
    if hwnd == 0 {
        return 0;
    }
    let (m, w, l) = to32(hwnd, c.w(2) as u32, c.w(3), c.l(4));
    user32::send(hwnd, m, w, l)
}

fn def_window_proc(c: &mut Call) -> u32 {
    let hwnd = c.w(0) as u32;
    let (m, w, l) = to32(hwnd, c.w(1) as u32, c.w(2), c.l(3));
    user32::DefWindowProcA(hwnd, m, w, l)
}

fn def_dlg_proc(c: &mut Call) -> u32 {
    let hwnd = c.w(0) as u32;
    let (m, w, l) = to32(hwnd, c.w(1) as u32, c.w(2), c.l(3));
    user32::def_dlg_proc(hwnd, m, w, l)
}

fn register_class(c: &mut Call) -> u32 {
    let p = c.ptr(0);
    unsafe {
        let (style, proc16, bg, menu_name, name) = (rdu16(p) as u32, rd32(p + 2), rdu16(p + 16) as u32, rd32(p + 18), rd32(p + 22));
        let name = cstr_at(far_to_linear(name));
        user32::register_class16(name, proc16, style, bg, menu_name);
    }
    1
}

fn create_window16(c: &mut Call) -> u32 {
    let (class_p, title_p) = (c.ptr(0), c.ptr(1));
    let conv = |v: i16| if v as u16 == 0x8000 { i32::MIN } else { v as i32 };
    unsafe {
        let class = cstr_at(class_p);
        let title = cstr_at(title_p);
        user32::create_window(
            class, title, c.l(2), conv(c.s(3)), conv(c.s(4)), conv(c.s(5)), conv(c.s(6)),
            c.w(7) as u32, c.w(8) as u32, c.w(9) as u32, c.l(10), 0,
        )
    }
}

fn create_dialog(c: &mut Call) -> u32 {
    let (name, parent, proc16) = (kernel_ext::key_of_far(c.l(1)), c.w(2) as u32, c.l(3));
    let Some(b) = kernel_ext::resource_bytes(&ResKey::Id(5), &name) else { return 0 };
    // the template needs stable memory during creation
    let Some(h) = kernel::global_alloc(b.len() as u32) else { return 0 };
    let (base, _) = kernel::global_info(h).unwrap();
    unsafe {
        std::ptr::copy_nonoverlapping(b.as_ptr(), base as usize as *mut u8, b.len());
        let r = create_dialog16(base, parent, proc16, 0);
        kernel::global_free_pub(h);
        r
    }
}

fn create_dialog_indirect(c: &mut Call) -> u32 {
    unsafe { create_dialog16(c.ptr(1), c.w(2) as u32, c.l(3), 0) }
}

/// IsDialogMessage(hDlg, lpMsg): if the message belongs to the dialog box or a child, processes
/// (dispatches) it and returns TRUE; the program must then not dispatch it again.
fn is_dialog_message(c: &mut Call) -> u32 {
    let (dlg, p) = (c.w(0) as u32, c.ptr(1));
    let (hwnd, msg, w, l) = unsafe { (rdu16(p) as u32, rdu16(p + 2) as u32, rdu16(p + 4), rd32(p + 6)) };
    if hwnd == 0 || (hwnd != dlg && user32::IsChild(dlg, hwnd) == 0) {
        return 0;
    }
    let (m, w32, l32) = to32(hwnd, msg, w, l);
    user32::send(hwnd, m, w32, l32);
    1
}

fn get_menu(c: &mut Call) -> u32 {
    user32::GetMenu(c.w(0) as u32)
}

fn check_menu_item(c: &mut Call) -> u32 {
    user32::CheckMenuItem(c.w(0) as u32, c.w(1) as u32, c.w(2) as u32) as u32
}

fn enable_menu_item(c: &mut Call) -> u32 {
    user32::EnableMenuItem(c.w(0) as u32, c.w(1) as u32, c.w(2) as u32) as u32
}

fn draw_menu_bar(_c: &mut Call) -> u32 {
    user32::touch();
    1
}

fn modify_menu(c: &mut Call) -> u32 {
    unsafe { user32::ModifyMenuA(c.w(0) as u32, c.w(1) as u32, c.w(2) as u32, c.w(3) as u32, c.ptr(4)) as u32 }
}

/// Extra window memory (cbWndExtra) in 16-bit words: a long takes two words (idx and idx+2).
fn extra_word(hwnd: u32, idx: i32) -> u32 {
    user32::GetWindowLongA(hwnd, idx) & 0xFFFF
}

fn get_window_word(c: &mut Call) -> u32 {
    let (hwnd, idx) = (c.w(0) as u32, c.s(1) as i32);
    if idx < 0 {
        return user32::GetWindowLongA(hwnd, idx) & 0xFFFF;
    }
    extra_word(hwnd, idx)
}

fn set_window_word(c: &mut Call) -> u32 {
    let (hwnd, idx, v) = (c.w(0) as u32, c.s(1) as i32, c.w(2) as u32);
    if idx < 0 {
        return user32::SetWindowLongA(hwnd, idx, v) & 0xFFFF;
    }
    user32::SetWindowLongA(hwnd, idx, v) & 0xFFFF
}

fn get_window_long(c: &mut Call) -> u32 {
    let (hwnd, idx) = (c.w(0) as u32, c.s(1) as i32);
    if idx < 0 {
        return user32::GetWindowLongA(hwnd, idx);
    }
    extra_word(hwnd, idx) | extra_word(hwnd, idx + 2) << 16
}

fn set_window_long(c: &mut Call) -> u32 {
    let (hwnd, idx, v) = (c.w(0) as u32, c.s(1) as i32, c.l(2));
    if idx == -4 {
        return user32::set_wndproc(hwnd, v, true); // GWL_WNDPROC: new far procedure
    }
    if idx < 0 {
        return user32::SetWindowLongA(hwnd, idx, v);
    }
    let old = extra_word(hwnd, idx) | extra_word(hwnd, idx + 2) << 16;
    user32::SetWindowLongA(hwnd, idx, v & 0xFFFF);
    user32::SetWindowLongA(hwnd, idx + 2, v >> 16);
    old
}

fn get_scroll_range(c: &mut Call) -> u32 {
    unsafe {
        wr16(c.ptr(2), 0);
        wr16(c.ptr(3), 0);
    }
    0
}

fn load_accelerators(c: &mut Call) -> u32 {
    let name = kernel_ext::key_of_far(c.l(1));
    let Some(b) = kernel_ext::resource_bytes(&ResKey::Id(9), &name) else { return 0 };
    let Some(h) = kernel::global_alloc(b.len() as u32) else { return 0 };
    let (base, _) = kernel::global_info(h).unwrap();
    unsafe { std::ptr::copy_nonoverlapping(b.as_ptr(), base as usize as *mut u8, b.len()) };
    if std::env::var_os("WINLAYER_WIN16_TRACE").is_some() {
        for e in b.chunks_exact(5) {
            eprintln!("[win16] accelerator: flags={:#04x} key={:#06x} command={}", e[0], u16::from_le_bytes([e[1], e[2]]), u16::from_le_bytes([e[3], e[4]]));
        }
    }
    h as u32
}

/// TranslateAccelerator(hwnd, haccel, lpmsg): 5-byte entries (flags, key, command).
fn translate_accelerator(c: &mut Call) -> u32 {
    let (hwnd, h, p) = (c.w(0) as u32, c.w(1), c.ptr(2));
    let Some((base, size)) = kernel::global_info(h) else { return 0 };
    let (msg, w) = unsafe { (rdu16(p + 2) as u32, rdu16(p + 4)) };
    if msg != 0x100 && msg != 0x102 {
        return 0;
    }
    let key_state = |vk: i32| user32::input_key_down(vk);
    let (ctrl, shift, alt) = (key_state(17), key_state(16), key_state(18));
    let mut o = 0u32;
    while o + 5 <= size {
        let (flags, key, cmd) = unsafe { (*((base + o) as usize as *const u8), rdu16(base + o + 1), rdu16(base + o + 3)) };
        let virt = flags & 1 != 0;
        let mods = (flags & 8 != 0) == ctrl && (flags & 4 != 0) == shift && (flags & 0x10 != 0) == alt;
        let hit = if virt { msg == 0x100 && key == w && mods } else { msg == 0x102 && key == w };
        if hit {
            user32::send(hwnd, WM_COMMAND, cmd as u32 | (1 << 16), 0);
            return 1;
        }
        if flags & 0x80 != 0 {
            break;
        }
        o += 5;
    }
    0
}

pub static IMPLS: &[Impl] = &[
    Impl { module: "USER", name: "INITAPP", sig: "w", f: one },
    Impl { module: "USER", name: "MESSAGEBOX", sig: "wppw", f: msgbox },
    Impl { module: "USER", name: "GETDESKTOPWINDOW", sig: "", f: get_desktop_window },
    Impl { module: "USER", name: "KILLTIMER", sig: "ww", f: kill_timer },
    Impl { module: "USER", name: "SETTIMER", sig: "wwwl", f: set_timer },
    Impl { module: "USER", name: "SETFOCUS", sig: "w", f: set_focus },
    Impl { module: "USER", name: "SETCAPTURE", sig: "w", f: set_capture },
    Impl { module: "USER", name: "RELEASECAPTURE", sig: "", f: release_capture },
    Impl { module: "USER", name: "SETCURSOR", sig: "w", f: set_cursor },
    Impl { module: "USER", name: "LOADCURSOR", sig: "wl", f: load_cursor },
    Impl { module: "USER", name: "LOADICON", sig: "wl", f: load_icon },
    Impl { module: "USER", name: "MESSAGEBEEP", sig: "w", f: message_beep },
    Impl { module: "USER", name: "WINHELP", sig: "wpwl", f: win_help },
    Impl { module: "USER", name: "GETCURSORPOS", sig: "p", f: get_cursor_pos },
    Impl { module: "USER", name: "SCREENTOCLIENT", sig: "wp", f: screen_to_client },
    Impl { module: "USER", name: "GETWINDOWRECT", sig: "wp", f: get_window_rect },
    Impl { module: "USER", name: "GETCLIENTRECT", sig: "wp", f: get_client_rect },
    Impl { module: "USER", name: "ENABLEWINDOW", sig: "ww", f: enable_window },
    Impl { module: "USER", name: "ISWINDOWENABLED", sig: "w", f: is_window_enabled },
    Impl { module: "USER", name: "GETWINDOWTEXT", sig: "wpw", f: get_window_text },
    Impl { module: "USER", name: "SETWINDOWTEXT", sig: "wp", f: set_window_text },
    Impl { module: "USER", name: "BEGINPAINT", sig: "wp", f: begin_paint },
    Impl { module: "USER", name: "ENDPAINT", sig: "wp", f: end_paint },
    Impl { module: "USER", name: "GETDC", sig: "w", f: get_dc },
    Impl { module: "USER", name: "RELEASEDC", sig: "ww", f: release_dc },
    Impl { module: "USER", name: "INVALIDATERECT", sig: "wpw", f: invalidate_rect },
    Impl { module: "USER", name: "VALIDATERECT", sig: "wp", f: validate_rect },
    Impl { module: "USER", name: "UPDATEWINDOW", sig: "w", f: update_window },
    Impl { module: "USER", name: "SHOWWINDOW", sig: "ww", f: show_window },
    Impl { module: "USER", name: "MOVEWINDOW", sig: "wwwwww", f: move_window },
    Impl { module: "USER", name: "DESTROYWINDOW", sig: "w", f: destroy_window },
    Impl { module: "USER", name: "GETPARENT", sig: "w", f: get_parent },
    Impl { module: "USER", name: "ISCHILD", sig: "ww", f: is_child },
    Impl { module: "USER", name: "GETDLGITEM", sig: "ww", f: get_dlg_item },
    Impl { module: "USER", name: "GETDLGCTRLID", sig: "w", f: get_dlg_ctrl_id },
    Impl { module: "USER", name: "GETSYSTEMMETRICS", sig: "w", f: get_system_metrics },
    Impl { module: "USER", name: "GETDIALOGBASEUNITS", sig: "", f: get_dialog_base_units },
    Impl { module: "USER", name: "GETFREESYSTEMRESOURCES", sig: "w", f: get_free_system_resources },
    Impl { module: "USER", name: "REALIZEPALETTE", sig: "w", f: zero },
    Impl { module: "USER", name: "SELECTPALETTE", sig: "www", f: zero },
    Impl { module: "USER", name: "SETRECT", sig: "pwwww", f: set_rect },
    Impl { module: "USER", name: "INFLATERECT", sig: "pww", f: inflate_rect },
    Impl { module: "USER", name: "OFFSETRECT", sig: "pww", f: offset_rect },
    Impl { module: "USER", name: "PTINRECT", sig: "pl", f: pt_in_rect },
    Impl { module: "USER", name: "FILLRECT", sig: "wpw", f: fill_rect },
    Impl { module: "USER", name: "FRAMERECT", sig: "wpw", f: frame_rect },
    Impl { module: "USER", name: "DRAWTEXT", sig: "wpwpw", f: draw_text },
    Impl { module: "USER", name: "PEEKMESSAGE", sig: "pwwww", f: peek_message },
    Impl { module: "USER", name: "TRANSLATEMESSAGE", sig: "p", f: translate_message },
    Impl { module: "USER", name: "DISPATCHMESSAGE", sig: "p", f: dispatch_message },
    Impl { module: "USER", name: "POSTQUITMESSAGE", sig: "w", f: post_quit },
    Impl { module: "USER", name: "SENDMESSAGE", sig: "wwwl", f: send_message },
    Impl { module: "USER", name: "SENDDLGITEMMESSAGE", sig: "wwwwl", f: send_dlg_item_message },
    Impl { module: "USER", name: "DEFWINDOWPROC", sig: "wwwl", f: def_window_proc },
    Impl { module: "USER", name: "DEFDLGPROC", sig: "wwwl", f: def_dlg_proc },
    Impl { module: "USER", name: "REGISTERCLASS", sig: "p", f: register_class },
    Impl { module: "USER", name: "CREATEWINDOW", sig: "pplwwwwwwwl", f: create_window16 },
    Impl { module: "USER", name: "CREATEDIALOG", sig: "wlwl", f: create_dialog },
    Impl { module: "USER", name: "CREATEDIALOGINDIRECT", sig: "wpwl", f: create_dialog_indirect },
    Impl { module: "USER", name: "ISDIALOGMESSAGE", sig: "wp", f: is_dialog_message },
    Impl { module: "USER", name: "GETMENU", sig: "w", f: get_menu },
    Impl { module: "USER", name: "CHECKMENUITEM", sig: "www", f: check_menu_item },
    Impl { module: "USER", name: "ENABLEMENUITEM", sig: "www", f: enable_menu_item },
    Impl { module: "USER", name: "DRAWMENUBAR", sig: "w", f: draw_menu_bar },
    Impl { module: "USER", name: "MODIFYMENU", sig: "wwwwl", f: modify_menu },
    Impl { module: "USER", name: "GETWINDOWWORD", sig: "ww", f: get_window_word },
    Impl { module: "USER", name: "SETWINDOWWORD", sig: "www", f: set_window_word },
    Impl { module: "USER", name: "SETWINDOWLONG", sig: "wwl", f: set_window_long },
    Impl { module: "USER", name: "GETWINDOWLONG", sig: "ww", f: get_window_long },
    Impl { module: "USER", name: "SETSCROLLPOS", sig: "wwww", f: zero },
    Impl { module: "USER", name: "SETSCROLLRANGE", sig: "wwwww", f: zero },
    Impl { module: "USER", name: "GETSCROLLRANGE", sig: "wwpp", f: get_scroll_range },
    Impl { module: "USER", name: "LOADACCELERATORS", sig: "wl", f: load_accelerators },
    Impl { module: "USER", name: "TRANSLATEACCELERATOR", sig: "wwp", f: translate_accelerator },
];

/// Registers the `user32` hooks (calling 16-bit WndProc, loading NE menus).
pub fn install_hooks() {
    user32::set_win16_hooks(call_proc16, load_menu16);
}
