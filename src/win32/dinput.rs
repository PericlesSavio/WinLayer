//! dinput.dll — DirectInput (DirectX 5): teclado e mouse do sistema.
//!
//! O estado vem do mesmo lugar que as mensagens de janela (teclas virtuais e botões em `user32`,
//! posição do cursor). Teclas viram códigos DIK pela tabela de códigos de varredura do `user32`.
//! Dados "bufferizados" (`GetDeviceData`) são gerados comparando o estado atual com o da consulta
//! anterior; o mouse relata o deslocamento relativo desde a última leitura.

use super::user32;
use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

pub fn resolve(name: &str) -> Option<usize> {
    exports!(name, DirectInputCreateA)
}

const DI_OK: u32 = 0;
const DI_NOEFFECT: u32 = 1;
const E_NOINTERFACE: u32 = 0x8000_4002;
const DIERR_UNSUPPORTED: u32 = 0x8000_4001;
const DIERR_INVALIDPARAM: u32 = 0x8007_0057;
const DIERR_DEVICENOTREG: u32 = 0x8004_0154;
const DIERR_NOTACQUIRED: u32 = 0x8007_000C;

// tipos de dispositivo (DIDEVTYPE_*)
const DIDEVTYPE_MOUSE: u32 = 2;
const DIDEVTYPE_KEYBOARD: u32 = 3;

/// Primeiro dword de GUID_SysMouse e GUID_SysKeyboard ({6F1D2B60/61-D5A0-11CF-BFC7-444553540000}).
const GUID_SYSMOUSE: u32 = 0x6F1D_2B60;
const GUID_SYSKEYBOARD: u32 = 0x6F1D_2B61;

fn trace() -> bool {
    std::env::var_os("WINLAYER_DINPUT_TRACE").is_some()
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Mouse,
    Keyboard,
}

struct Device {
    kind: Kind,
    acquired: bool,
    /// Último estado lido (teclado: 256 bytes DIK; mouse: botões em [0..3]).
    last_keys: [u8; 256],
    last_buttons: [u8; 3],
    /// Posição do cursor na leitura anterior (para os deltas).
    last_pos: (i32, i32),
    /// Deltas e mudanças ainda não entregues por GetDeviceData.
    pending: Vec<(u32, u32)>,
    seq: u32,
    refs: u32,
}

struct State {
    di: u32,
    devices: BTreeMap<u32, Device>,
}

static DI: Mutex<State> = Mutex::new(State { di: 0, devices: BTreeMap::new() });

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

unsafe fn iid(p: u32) -> u32 {
    if p == 0 { 0 } else { rd(p, 0) }
}

// ---- leitura do estado ----------------------------------------------------------------------

/// Teclado atual em códigos DIK (0x80 = pressionada).
fn keyboard_now() -> [u8; 256] {
    let mut k = [0u8; 256];
    for vk in 8u32..0xFF {
        if matches!(vk, 0x10..=0x12) {
            continue; // Shift/Ctrl/Alt genéricos: tratados abaixo como as teclas da esquerda
        }
        if !user32::input_key_down(vk as i32) {
            continue;
        }
        let (scan, ext) = user32::default_scan(vk);
        if scan != 0 {
            k[(scan | if ext != 0 { 0x80 } else { 0 }) as usize & 0xFF] = 0x80;
        }
    }
    if user32::input_key_down(0x10) {
        k[0x2A] = 0x80; // DIK_LSHIFT
    }
    if user32::input_key_down(0x11) {
        k[0x1D] = 0x80; // DIK_LCONTROL
    }
    if user32::input_key_down(0x12) {
        k[0x38] = 0x80; // DIK_LMENU
    }
    k
}

fn buttons_now() -> [u8; 3] {
    [1, 2, 4].map(|vk| if user32::input_key_down(vk) { 0x80 } else { 0 })
}

fn cursor_now() -> (i32, i32) {
    let mut p = [0i32; 2];
    unsafe { user32::GetCursorPos(p.as_mut_ptr()) };
    (p[0], p[1])
}

/// Atualiza o dispositivo e acumula as mudanças desde a última leitura em `pending`.
fn poll(d: &mut Device) {
    match d.kind {
        Kind::Keyboard => {
            let now = keyboard_now();
            for i in 0..256 {
                if now[i] != d.last_keys[i] {
                    d.pending.push((i as u32, now[i] as u32));
                }
            }
            d.last_keys = now;
        }
        Kind::Mouse => {
            let pos = cursor_now();
            let (dx, dy) = (pos.0 - d.last_pos.0, pos.1 - d.last_pos.1);
            if dx != 0 {
                d.pending.push((0, dx as u32)); // DIMOFS_X
            }
            if dy != 0 {
                d.pending.push((4, dy as u32)); // DIMOFS_Y
            }
            d.last_pos = pos;
            let b = buttons_now();
            for i in 0..3 {
                if b[i] != d.last_buttons[i] {
                    d.pending.push((12 + i as u32, b[i] as u32)); // DIMOFS_BUTTON0..2
                }
            }
            d.last_buttons = b;
        }
    }
    if d.pending.len() > 256 {
        let extra = d.pending.len() - 256;
        d.pending.drain(..extra);
    }
}

// ---- API exportada ------------------------------------------------------------------------

#[allow(non_snake_case)]
unsafe extern "stdcall" fn DirectInputCreateA(_inst: u32, version: u32, out: *mut u32, _outer: u32) -> u32 {
    if out.is_null() {
        return DIERR_INVALIDPARAM;
    }
    let mut s = DI.lock().unwrap();
    if s.di == 0 {
        s.di = new_obj(di_vtbl());
    }
    if trace() {
        eprintln!("[dinput] DirectInputCreateA versão {version:#x} -> {:#x}", s.di);
    }
    *out = s.di;
    DI_OK
}

// ---- IDirectInputA --------------------------------------------------------------------------

fn di_vtbl() -> &'static [u32] {
    vtable![di_qi, di_addref, di_release, di_create_device, di_enum_devices, di_get_status, di_ok3, di_ok3]
}

unsafe extern "stdcall" fn di_qi(this: u32, riid: u32, out: *mut u32) -> u32 {
    // IUnknown, IDirectInputA {89521360-...}, IDirectInput2A {5944E662-...}
    match iid(riid) {
        0 | 0x8952_1360 => {
            *out = this;
            DI_OK
        }
        _ => {
            *out = 0;
            E_NOINTERFACE
        }
    }
}

extern "stdcall" fn di_addref(_this: u32) -> u32 {
    1
}

extern "stdcall" fn di_release(_this: u32) -> u32 {
    0
}

extern "stdcall" fn di_ok3(_this: u32, _a: u32, _b: u32) -> u32 {
    DI_OK
}

unsafe extern "stdcall" fn di_create_device(_this: u32, guid: u32, out: *mut u32, _outer: u32) -> u32 {
    let kind = match iid(guid) {
        GUID_SYSMOUSE => Kind::Mouse,
        GUID_SYSKEYBOARD => Kind::Keyboard,
        other => {
            if trace() {
                eprintln!("[dinput] CreateDevice: dispositivo {other:#010x} não existe");
            }
            return DIERR_DEVICENOTREG;
        }
    };
    let obj = new_obj(dev_vtbl());
    let dev = Device {
        kind,
        acquired: false,
        last_keys: keyboard_now(),
        last_buttons: buttons_now(),
        last_pos: cursor_now(),
        pending: Vec::new(),
        seq: 0,
        refs: 1,
    };
    if trace() {
        eprintln!("[dinput] CreateDevice {} -> {obj:#x}", if kind == Kind::Mouse { "mouse" } else { "teclado" });
    }
    DI.lock().unwrap().devices.insert(obj, dev);
    *out = obj;
    DI_OK
}

/// DIDEVICEINSTANCEA (580 bytes): dwSize, guidInstance, guidProduct, dwDevType, tszInstanceName[260], tszProductName[260].
fn device_instance(kind: Kind) -> Vec<u8> {
    let mut v = vec![0u8; 580];
    v[0..4].copy_from_slice(&580u32.to_le_bytes());
    let guid: [u8; 16] = {
        let mut g = [0u8; 16];
        g[0..4].copy_from_slice(&(if kind == Kind::Mouse { GUID_SYSMOUSE } else { GUID_SYSKEYBOARD }).to_le_bytes());
        g[4..16].copy_from_slice(&[0xA0, 0xD5, 0xCF, 0x11, 0xBF, 0xC7, 0x44, 0x45, 0x53, 0x54, 0x00, 0x00]);
        g
    };
    v[4..20].copy_from_slice(&guid);
    v[20..36].copy_from_slice(&guid);
    let ty = if kind == Kind::Mouse { DIDEVTYPE_MOUSE } else { DIDEVTYPE_KEYBOARD };
    v[36..40].copy_from_slice(&ty.to_le_bytes());
    let name: &[u8] = if kind == Kind::Mouse { b"Mouse" } else { b"Keyboard" };
    v[40..40 + name.len()].copy_from_slice(name);
    v[300..300 + name.len()].copy_from_slice(name);
    v
}

unsafe extern "stdcall" fn di_enum_devices(_this: u32, ty: u32, cb: u32, ctx: u32, _flags: u32) -> u32 {
    if cb == 0 {
        return DIERR_INVALIDPARAM;
    }
    for kind in [Kind::Mouse, Kind::Keyboard] {
        let t = if kind == Kind::Mouse { DIDEVTYPE_MOUSE } else { DIDEVTYPE_KEYBOARD };
        if ty != 0 && ty != t {
            continue;
        }
        let inst = device_instance(kind);
        let f: extern "stdcall" fn(u32, u32) -> u32 = std::mem::transmute(cb as usize);
        if f(inst.as_ptr() as u32, ctx) == 0 {
            break; // DIENUM_STOP
        }
    }
    DI_OK
}

unsafe extern "stdcall" fn di_get_status(_this: u32, guid: u32) -> u32 {
    match iid(guid) {
        GUID_SYSMOUSE | GUID_SYSKEYBOARD => DI_OK,
        _ => DIERR_DEVICENOTREG,
    }
}

// ---- IDirectInputDeviceA / IDirectInputDevice2A ---------------------------------------------

fn dev_vtbl() -> &'static [u32] {
    vtable![
        dev_qi, dev_addref, dev_release, dev_get_caps, dev_ok4, dev_unsupported3, dev_ok3, dev_acquire,
        dev_unacquire, dev_get_state, dev_get_data, dev_ok2, dev_ok2, dev_ok3, dev_unsupported4,
        dev_get_info, dev_ok3, dev_ok4,
        // IDirectInputDevice2A
        dev_unsupported5, dev_ok4, dev_unsupported3, dev_unsupported2, dev_unsupported2, dev_ok4,
        dev_unsupported2, dev_poll, dev_unsupported5,
    ]
}

unsafe extern "stdcall" fn dev_qi(this: u32, riid: u32, out: *mut u32) -> u32 {
    // IUnknown, IDirectInputDeviceA {5944E680-...}, IDirectInputDevice2A {5944E682-...}
    match iid(riid) {
        0 | 0x5944_E680 | 0x5944_E682 => {
            dev_addref(this);
            *out = this;
            DI_OK
        }
        _ => {
            *out = 0;
            E_NOINTERFACE
        }
    }
}

extern "stdcall" fn dev_addref(this: u32) -> u32 {
    let mut s = DI.lock().unwrap();
    s.devices.get_mut(&this).map_or(0, |d| {
        d.refs += 1;
        d.refs
    })
}

extern "stdcall" fn dev_release(this: u32) -> u32 {
    let mut s = DI.lock().unwrap();
    let Some(d) = s.devices.get_mut(&this) else { return 0 };
    d.refs = d.refs.saturating_sub(1);
    let left = d.refs;
    if left == 0 {
        s.devices.remove(&this);
    }
    left
}

extern "stdcall" fn dev_ok2(_this: u32, _a: u32) -> u32 {
    DI_OK
}

extern "stdcall" fn dev_ok3(_this: u32, _a: u32, _b: u32) -> u32 {
    DI_OK
}

extern "stdcall" fn dev_ok4(_this: u32, _a: u32, _b: u32, _c: u32) -> u32 {
    DI_OK
}

extern "stdcall" fn dev_unsupported2(_this: u32, _a: u32) -> u32 {
    DIERR_UNSUPPORTED
}

extern "stdcall" fn dev_unsupported3(_this: u32, _a: u32, _b: u32) -> u32 {
    DIERR_UNSUPPORTED
}

extern "stdcall" fn dev_unsupported4(_this: u32, _a: u32, _b: u32, _c: u32) -> u32 {
    DIERR_UNSUPPORTED
}

extern "stdcall" fn dev_unsupported5(_this: u32, _a: u32, _b: u32, _c: u32, _d: u32) -> u32 {
    DIERR_UNSUPPORTED
}

extern "stdcall" fn dev_poll(_this: u32) -> u32 {
    DI_NOEFFECT
}

unsafe extern "stdcall" fn dev_get_caps(this: u32, caps: u32) -> u32 {
    let s = DI.lock().unwrap();
    let Some(d) = s.devices.get(&this) else { return DIERR_INVALIDPARAM };
    if caps == 0 {
        return DIERR_INVALIDPARAM;
    }
    // DIDEVCAPS: dwSize, dwFlags, dwDevType, dwAxes, dwButtons, dwPOVs, ...
    wr(caps, 4, 1); // DIDC_ATTACHED
    match d.kind {
        Kind::Mouse => {
            wr(caps, 8, DIDEVTYPE_MOUSE);
            wr(caps, 12, 2);
            wr(caps, 16, 3);
        }
        Kind::Keyboard => {
            wr(caps, 8, DIDEVTYPE_KEYBOARD);
            wr(caps, 12, 0);
            wr(caps, 16, 128);
        }
    }
    wr(caps, 20, 0);
    DI_OK
}

unsafe extern "stdcall" fn dev_get_info(this: u32, out: u32) -> u32 {
    let kind = match DI.lock().unwrap().devices.get(&this) {
        Some(d) => d.kind,
        None => return DIERR_INVALIDPARAM,
    };
    if out == 0 {
        return DIERR_INVALIDPARAM;
    }
    let inst = device_instance(kind);
    let n = (rd(out, 0) as usize).clamp(4, inst.len());
    std::ptr::copy_nonoverlapping(inst.as_ptr().add(4), (out + 4) as usize as *mut u8, n - 4);
    DI_OK
}

extern "stdcall" fn dev_acquire(this: u32) -> u32 {
    let mut s = DI.lock().unwrap();
    let Some(d) = s.devices.get_mut(&this) else { return DIERR_INVALIDPARAM };
    if !d.acquired {
        d.acquired = true;
        d.last_pos = cursor_now();
        d.last_keys = keyboard_now();
        d.last_buttons = buttons_now();
        d.pending.clear();
    }
    DI_OK
}

extern "stdcall" fn dev_unacquire(this: u32) -> u32 {
    let mut s = DI.lock().unwrap();
    match s.devices.get_mut(&this) {
        Some(d) if d.acquired => {
            d.acquired = false;
            DI_OK
        }
        _ => DI_NOEFFECT,
    }
}

unsafe extern "stdcall" fn dev_get_state(this: u32, cb: u32, data: u32) -> u32 {
    let mut s = DI.lock().unwrap();
    let Some(d) = s.devices.get_mut(&this) else { return DIERR_INVALIDPARAM };
    if !d.acquired {
        return DIERR_NOTACQUIRED;
    }
    if data == 0 {
        return DIERR_INVALIDPARAM;
    }
    std::ptr::write_bytes(data as usize as *mut u8, 0, cb as usize);
    match d.kind {
        Kind::Keyboard => {
            let k = keyboard_now();
            let n = (cb as usize).min(256);
            std::ptr::copy_nonoverlapping(k.as_ptr(), data as usize as *mut u8, n);
            d.last_keys = k;
        }
        Kind::Mouse => {
            // DIMOUSESTATE: lX, lY, lZ (relativos), rgbButtons[4]
            let pos = cursor_now();
            if cb >= 12 {
                wr(data, 0, (pos.0 - d.last_pos.0) as u32);
                wr(data, 4, (pos.1 - d.last_pos.1) as u32);
            }
            d.last_pos = pos;
            let b = buttons_now();
            for (i, v) in b.iter().enumerate() {
                if 12 + i < cb as usize {
                    *((data + 12 + i as u32) as usize as *mut u8) = *v;
                }
            }
            d.last_buttons = b;
        }
    }
    d.pending.clear();
    DI_OK
}

/// GetDeviceData(cbObjectData, rgdod, *pdwInOut, flags): entrega as mudanças acumuladas como
/// DIDEVICEOBJECTDATA {dwOfs, dwData, dwTimeStamp, dwSequence}.
unsafe extern "stdcall" fn dev_get_data(this: u32, obj_size: u32, out: u32, inout: *mut u32, flags: u32) -> u32 {
    const DIGDD_PEEK: u32 = 1;
    let mut s = DI.lock().unwrap();
    let Some(d) = s.devices.get_mut(&this) else { return DIERR_INVALIDPARAM };
    if !d.acquired {
        return DIERR_NOTACQUIRED;
    }
    if inout.is_null() {
        return DIERR_INVALIDPARAM;
    }
    poll(d);
    let max = *inout as usize;
    let n = d.pending.len().min(max);
    let size = obj_size.max(16);
    if out != 0 {
        let now = user32::now_ms();
        for (i, &(ofs, val)) in d.pending.iter().take(n).enumerate() {
            let p = out + i as u32 * size;
            wr(p, 0, ofs);
            wr(p, 4, val);
            wr(p, 8, now);
            wr(p, 12, d.seq + i as u32);
        }
    }
    let overflow = d.pending.len() > max && out != 0 && max != 0;
    if flags & DIGDD_PEEK == 0 {
        d.pending.drain(..n);
        d.seq += n as u32;
    }
    *inout = n as u32;
    if overflow && trace() {
        eprintln!("[dinput] GetDeviceData: {} eventos ficaram para a próxima chamada", d.pending.len());
    }
    // descarte (out == 0, *inout == INFINITE) já acontece no drain acima
    DI_OK
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instancia_do_mouse_tem_tipo_e_guid() {
        let v = device_instance(Kind::Mouse);
        assert_eq!(u32::from_le_bytes(v[4..8].try_into().unwrap()), GUID_SYSMOUSE);
        assert_eq!(u32::from_le_bytes(v[36..40].try_into().unwrap()), DIDEVTYPE_MOUSE);
        assert_eq!(&v[40..45], b"Mouse");
    }
}
