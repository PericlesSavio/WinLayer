//! winmm.dll — som (sndPlaySound, PlaySound, waveOut), temporizadores de multimídia e stubs de MCI,
//! joystick e MIDI. A saída de áudio vai para o visualizador (ver `audio.rs`).

use super::{call5, user32};
use crate::audio;
use std::collections::BTreeMap;
use std::sync::Mutex;

pub fn resolve(name: &str) -> Option<usize> {
    exports!(
        name,
        sndPlaySoundA, PlaySoundA, timeGetTime, timeBeginPeriod, timeEndPeriod, timeSetEvent, timeKillEvent,
        waveOutGetNumDevs, waveOutGetDevCapsA, waveOutOpen, waveOutClose, waveOutPrepareHeader,
        waveOutUnprepareHeader, waveOutWrite, waveOutReset, waveOutPause, waveOutRestart,
        waveOutGetPosition, waveOutGetVolume, waveOutGetID, waveOutSetVolume, mciSendCommandA, mciSendStringA,
        mciGetErrorStringA, joyGetNumDevs, joyGetPos, joyGetDevCapsA, joySetCapture, joyReleaseCapture,
        joyGetPosEx, midiOutReset, midiOutShortMsg, midiOutLongMsg, timeGetDevCaps, midiOutOpen, midiOutClose,
        midiOutPrepareHeader, midiOutUnprepareHeader, midiOutGetNumDevs, auxGetNumDevs, auxGetDevCapsA, auxGetVolume,
        auxSetVolume, mixerGetNumDevs, mixerOpen, mixerClose, mixerGetLineInfoA, mixerGetLineControlsA,
        mixerGetControlDetailsA, mixerSetControlDetails, waveInOpen, waveInClose, waveInAddBuffer, waveInPrepareHeader,
        waveInUnprepareHeader, waveInReset, waveInStart, waveInGetNumDevs, mmioOpenA, mmioAscend, mmioDescend, mmioRead, mmioClose,
    )
}

const SND_MEMORY: u32 = 0x04;
const SND_LOOP: u32 = 0x08;
const SND_ALIAS: u32 = 0x10000;
const SND_RESOURCE: u32 = 0x40004;

/// Núcleo de sndPlaySound/PlaySound (também usado pela camada Win16).
pub unsafe fn play_sound(name: u32, flags: u32) -> u32 {
    if name == 0 {
        audio::stop();
        return 1;
    }
    let looped = flags & SND_LOOP != 0;
    if flags & SND_RESOURCE == SND_RESOURCE {
        return 0; // sons embutidos em recursos: ainda não
    }
    if flags & SND_ALIAS == SND_ALIAS {
        audio::beep(); // "SystemDefault" e afins
        return 1;
    }
    if flags & SND_MEMORY != 0 {
        let p = name as usize as *const u8;
        let size = u32::from_le_bytes([*p.add(4), *p.add(5), *p.add(6), *p.add(7)]) as usize + 8;
        return audio::play_memory(std::slice::from_raw_parts(p, size).to_vec(), looped) as u32;
    }
    let path = std::ffi::CStr::from_ptr(name as usize as *const i8).to_string_lossy().into_owned();
    audio::play_file(&super::files::host_path(&path), looped) as u32
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn sndPlaySoundA(name: u32, flags: u32) -> u32 {
    play_sound(name, flags & 0xFFFF)
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn PlaySoundA(name: u32, _hmod: u32, flags: u32) -> u32 {
    play_sound(name, flags)
}

#[allow(non_snake_case)]
extern "stdcall" fn timeGetTime() -> u32 {
    user32::tick_virtual(1); // ver GetTickCount
    user32::now_ms()
}

#[allow(non_snake_case)]
extern "stdcall" fn timeBeginPeriod(_ms: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
extern "stdcall" fn timeEndPeriod(_ms: u32) -> u32 {
    0
}

// ---- temporizadores de multimídia ---------------------------------------------------------

struct MmTimer {
    due: u32,
    interval: u32,
    periodic: bool,
    callback: u32,
    user: u32,
}

static TIMERS: Mutex<BTreeMap<u32, MmTimer>> = Mutex::new(BTreeMap::new());
static NEXT_TIMER: Mutex<u32> = Mutex::new(1);
static TIMER_THREAD: std::sync::Once = std::sync::Once::new();

#[allow(non_snake_case)]
extern "stdcall" fn timeSetEvent(delay: u32, _res: u32, callback: u32, user: u32, flags: u32) -> u32 {
    let mut n = NEXT_TIMER.lock().unwrap();
    let id = *n;
    *n += 1;
    let interval = delay.max(1);
    TIMER_THREAD.call_once(|| {
        let base = super::kernel32::image_base();
        let _ = std::thread::Builder::new().spawn(move || {
            if super::teb::install(base).is_err() {
                return;
            }
            loop {
                std::thread::sleep(std::time::Duration::from_millis(1));
                user32::tick_virtual(1); // o relógio virtual do modo captura também anda com a thread principal bloqueada
                fire_timers();
            }
        });
    });
    TIMERS.lock().unwrap().insert(id, MmTimer { due: user32::now_ms() + interval, interval, periodic: flags & 1 != 0, callback, user });
    id
}

#[allow(non_snake_case)]
extern "stdcall" fn timeKillEvent(id: u32) -> u32 {
    TIMERS.lock().unwrap().remove(&id).is_none() as u32 * 97 // MMSYSERR_INVALPARAM se não existir
}

// ---- waveOut ------------------------------------------------------------------------------

struct Wave {
    fmt: Vec<u8>,
    callback: u32,
    instance: u32,
    kind: u32,
}

struct Pending {
    hwo: u32,
    hdr: u32,
    done_at: u32,
}

static WAVES: Mutex<BTreeMap<u32, Wave>> = Mutex::new(BTreeMap::new());
static PENDING: Mutex<Vec<Pending>> = Mutex::new(Vec::new());
/// Instante (ms do relógio do processo) em que o fluxo termina de tocar.
static STREAM_END: Mutex<u32> = Mutex::new(0);
static NEXT_WAVE: Mutex<u32> = Mutex::new(0x6100);

const WHDR_DONE: u32 = 1;
const WHDR_PREPARED: u32 = 2;
const WHDR_INQUEUE: u32 = 0x10;
const MM_WOM_OPEN: u32 = 0x3BB;
const MM_WOM_CLOSE: u32 = 0x3BC;
const MM_WOM_DONE: u32 = 0x3BD;

/// Entrega uma notificação conforme o tipo de callback pedido em waveOutOpen.
fn notify(hwo: u32, msg: u32, p1: u32, p2: u32) {
    let Some((cb, inst, kind)) = WAVES.lock().unwrap().get(&hwo).map(|w| (w.callback, w.instance, w.kind)) else { return };
    match kind & 0x7_0000 {
        0x1_0000 if cb != 0 => {
            user32::PostMessageA(cb, msg, hwo, p1);
        }
        0x3_0000 if cb != 0 => unsafe {
            call5(cb, hwo, msg, inst, p1, p2);
        },
        _ => {}
    }
}

#[allow(non_snake_case)]
extern "stdcall" fn waveOutGetNumDevs() -> u32 {
    1
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn waveOutGetDevCapsA(_dev: u32, caps: *mut u8, cb: u32) -> u32 {
    if cb < 52 {
        return 11; // MMSYSERR_INVALPARAM
    }
    std::ptr::write_bytes(caps, 0, 52);
    *(caps as *mut u16) = 1; // wMid
    *(caps.add(2) as *mut u16) = 1; // wPid
    *(caps.add(4) as *mut u32) = 0x0100; // vDriverVersion
    let name = b"winlayer audio";
    std::ptr::copy_nonoverlapping(name.as_ptr(), caps.add(8), name.len());
    *(caps.add(40) as *mut u32) = 0xFFF; // dwFormats: todos os formatos padrão
    *(caps.add(44) as *mut u16) = 2; // wChannels
    *(caps.add(48) as *mut u32) = 0x0C; // WAVECAPS_VOLUME | WAVECAPS_LRVOLUME
    0
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn waveOutOpen(phwo: *mut u32, _dev: u32, fmt: *const u8, cb: u32, inst: u32, flags: u32) -> u32 {
    if flags & 1 != 0 {
        return 0; // WAVE_FORMAT_QUERY: sim, suportamos
    }
    let mut n = NEXT_WAVE.lock().unwrap();
    let h = *n;
    *n += 4;
    drop(n);
    WAVES.lock().unwrap().insert(h, Wave { fmt: std::slice::from_raw_parts(fmt, 16).to_vec(), callback: cb, instance: inst, kind: flags });
    if !phwo.is_null() {
        *phwo = h;
    }
    notify(h, MM_WOM_OPEN, 0, 0);
    0
}

#[allow(non_snake_case)]
extern "stdcall" fn waveOutClose(hwo: u32) -> u32 {
    if PENDING.lock().unwrap().iter().any(|p| p.hwo == hwo) {
        return 33; // WAVERR_STILLPLAYING
    }
    notify(hwo, MM_WOM_CLOSE, 0, 0);
    WAVES.lock().unwrap().remove(&hwo);
    0
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn waveOutPrepareHeader(_hwo: u32, hdr: *mut u32, _size: u32) -> u32 {
    *hdr.add(4) |= WHDR_PREPARED; // dwFlags (offset 16)
    0
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn waveOutUnprepareHeader(_hwo: u32, hdr: *mut u32, _size: u32) -> u32 {
    *hdr.add(4) &= !WHDR_PREPARED;
    0
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn waveOutWrite(hwo: u32, hdr: *mut u32, _size: u32) -> u32 {
    let Some(fmt) = WAVES.lock().unwrap().get(&hwo).map(|w| w.fmt.clone()) else { return 5 }; // MMSYSERR_INVALHANDLE
    let (data, len) = (*hdr as usize as *const u8, *hdr.add(1) as usize);
    let dur = audio::play_stream(&fmt, std::slice::from_raw_parts(data, len));
    let now = user32::now_ms();
    let mut end = STREAM_END.lock().unwrap();
    *end = (*end).max(now) + dur;
    *hdr.add(4) = (*hdr.add(4) | WHDR_INQUEUE) & !WHDR_DONE;
    PENDING.lock().unwrap().push(Pending { hwo, hdr: hdr as usize as u32, done_at: *end });
    0
}

#[allow(non_snake_case)]
extern "stdcall" fn waveOutReset(hwo: u32) -> u32 {
    audio::stop();
    *STREAM_END.lock().unwrap() = 0;
    let done: Vec<Pending> = {
        let mut p = PENDING.lock().unwrap();
        let (mine, rest): (Vec<_>, Vec<_>) = p.drain(..).partition(|x| x.hwo == hwo);
        *p = rest;
        mine
    };
    for d in done {
        finish(&d);
    }
    0
}

fn finish(p: &Pending) {
    unsafe {
        let flags = (p.hdr + 16) as usize as *mut u32;
        *flags = (*flags & !WHDR_INQUEUE) | WHDR_DONE;
    }
    notify(p.hwo, MM_WOM_DONE, p.hdr, 0);
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn waveOutGetID(_hwo: u32, id: *mut u32) -> u32 {
    if !id.is_null() {
        *id = 0;
    }
    0
}

#[allow(non_snake_case)]
extern "stdcall" fn waveOutPause(_hwo: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
extern "stdcall" fn waveOutRestart(_hwo: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn waveOutGetPosition(_hwo: u32, mmt: *mut u32, _size: u32) -> u32 {
    // MMTIME: wType + união; devolvemos posição 0 no tipo pedido
    *mmt.add(1) = 0;
    0
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn waveOutGetVolume(_hwo: u32, vol: *mut u32) -> u32 {
    if !vol.is_null() {
        *vol = 0xFFFF_FFFF;
    }
    0
}

#[allow(non_snake_case)]
extern "stdcall" fn waveOutSetVolume(_hwo: u32, _vol: u32) -> u32 {
    0
}

/// Chamada a cada ciclo de mensagens: conclui buffers de waveOut e dispara timeSetEvent.
pub fn poll() {
    let now = user32::now_ms();
    let done: Vec<Pending> = {
        let mut p = PENDING.lock().unwrap();
        let (d, rest): (Vec<_>, Vec<_>) = p.drain(..).partition(|x| x.done_at <= now);
        *p = rest;
        d
    };
    for d in done {
        finish(&d);
    }
}

/// Dispara os temporizadores vencidos. Roda na thread própria dos temporizadores, como no Windows: o
/// programa pode bloquear a thread principal esperando um evento que só o callback sinaliza.
fn fire_timers() {
    let now = user32::now_ms();
    let due: Vec<(u32, u32, u32)> = {
        let mut t = TIMERS.lock().unwrap();
        let ids: Vec<u32> = t.iter().filter(|(_, m)| now.wrapping_sub(m.due) < 0x8000_0000).map(|(&i, _)| i).collect();
        let mut out = Vec::new();
        for id in ids {
            let m = t.get_mut(&id).unwrap();
            out.push((id, m.callback, m.user));
            if m.periodic {
                m.due = now + m.interval;
            } else {
                t.remove(&id);
            }
        }
        out
    };
    for (id, cb, user) in due {
        // LPTIMECALLBACK(uTimerID, uMsg, dwUser, dw1, dw2)
        unsafe { call5(cb, id, 0, user, 0, 0) };
    }
}

// ---- MCI, joystick, MIDI, mmio: sem suporte (devolvem erro/“sem dispositivo”) -----------------

#[allow(non_snake_case)]
extern "stdcall" fn mciSendCommandA(_dev: u32, _msg: u32, _flags: u32, _param: u32) -> u32 {
    263 // MCIERR_UNSUPPORTED_FUNCTION
}

#[allow(non_snake_case)]
extern "stdcall" fn mciSendStringA(_cmd: u32, _ret: u32, _cch: u32, _hwnd: u32) -> u32 {
    263
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn mciGetErrorStringA(_err: u32, buf: *mut u8, cch: u32) -> u32 {
    let msg = b"MCI indisponivel\0";
    let n = msg.len().min(cch as usize);
    if n > 0 {
        std::ptr::copy_nonoverlapping(msg.as_ptr(), buf, n);
        *buf.add(n - 1) = 0;
    }
    1
}

#[allow(non_snake_case)]
extern "stdcall" fn joyGetNumDevs() -> u32 {
    0
}

#[allow(non_snake_case)]
extern "stdcall" fn joyGetPos(_id: u32, _info: u32) -> u32 {
    165 // JOYERR_UNPLUGGED
}

#[allow(non_snake_case)]
extern "stdcall" fn joyGetDevCapsA(_id: u32, _caps: u32, _cb: u32) -> u32 {
    165
}

#[allow(non_snake_case)]
extern "stdcall" fn joySetCapture(_hwnd: u32, _id: u32, _period: u32, _changed: u32) -> u32 {
    165
}

#[allow(non_snake_case)]
extern "stdcall" fn joyReleaseCapture(_id: u32) -> u32 {
    165
}

#[allow(non_snake_case)]
extern "stdcall" fn midiOutReset(_h: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
extern "stdcall" fn midiOutShortMsg(_h: u32, _msg: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
extern "stdcall" fn midiOutLongMsg(_h: u32, _hdr: u32, _size: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
extern "stdcall" fn mmioOpenA(_name: u32, _info: u32, _flags: u32) -> u32 {
    0 // falha ao abrir
}

#[allow(non_snake_case)]
extern "stdcall" fn mmioAscend(_h: u32, _ck: u32, _flags: u32) -> u32 {
    264 // MMIOERR_CHUNKNOTFOUND
}

#[allow(non_snake_case)]
extern "stdcall" fn mmioDescend(_h: u32, _ck: u32, _parent: u32, _flags: u32) -> u32 {
    264
}

#[allow(non_snake_case)]
extern "stdcall" fn mmioRead(_h: u32, _buf: u32, _n: u32) -> u32 {
    u32::MAX
}

#[allow(non_snake_case)]
extern "stdcall" fn mmioClose(_h: u32, _flags: u32) -> u32 {
    0
}

// ---- dispositivos que não existem: MIDI, entrada de áudio, mixer, aux ------------------------

const MMSYSERR_BADDEVICEID: u32 = 2;
const MMSYSERR_INVALHANDLE: u32 = 5;
const MMSYSERR_NODRIVER: u32 = 6;

#[allow(non_snake_case)]
unsafe extern "stdcall" fn timeGetDevCaps(caps: *mut u32, cb: u32) -> u32 {
    if caps.is_null() || cb < 8 {
        return 97; // TIMERR_NOCANDO
    }
    *caps = 1; // wPeriodMin
    *caps.add(1) = 1_000_000; // wPeriodMax
    0
}

#[allow(non_snake_case)]
extern "stdcall" fn midiOutGetNumDevs() -> u32 {
    0
}

#[allow(non_snake_case)]
extern "stdcall" fn midiOutOpen(_out: u32, _dev: u32, _cb: u32, _inst: u32, _flags: u32) -> u32 {
    MMSYSERR_BADDEVICEID
}

#[allow(non_snake_case)]
extern "stdcall" fn midiOutClose(_h: u32) -> u32 {
    MMSYSERR_INVALHANDLE
}

#[allow(non_snake_case)]
extern "stdcall" fn midiOutPrepareHeader(_h: u32, _hdr: u32, _size: u32) -> u32 {
    MMSYSERR_INVALHANDLE
}

#[allow(non_snake_case)]
extern "stdcall" fn midiOutUnprepareHeader(_h: u32, _hdr: u32, _size: u32) -> u32 {
    MMSYSERR_INVALHANDLE
}

#[allow(non_snake_case)]
extern "stdcall" fn auxGetNumDevs() -> u32 {
    0
}

#[allow(non_snake_case)]
extern "stdcall" fn auxGetDevCapsA(_dev: u32, _caps: u32, _cb: u32) -> u32 {
    MMSYSERR_BADDEVICEID
}

#[allow(non_snake_case)]
extern "stdcall" fn auxGetVolume(_dev: u32, _vol: u32) -> u32 {
    MMSYSERR_BADDEVICEID
}

#[allow(non_snake_case)]
extern "stdcall" fn auxSetVolume(_dev: u32, _vol: u32) -> u32 {
    MMSYSERR_BADDEVICEID
}

#[allow(non_snake_case)]
extern "stdcall" fn mixerGetNumDevs() -> u32 {
    0
}

#[allow(non_snake_case)]
extern "stdcall" fn mixerOpen(_out: u32, _dev: u32, _cb: u32, _inst: u32, _flags: u32) -> u32 {
    MMSYSERR_NODRIVER
}

#[allow(non_snake_case)]
extern "stdcall" fn mixerClose(_h: u32) -> u32 {
    MMSYSERR_INVALHANDLE
}

#[allow(non_snake_case)]
extern "stdcall" fn mixerGetLineInfoA(_h: u32, _info: u32, _flags: u32) -> u32 {
    MMSYSERR_INVALHANDLE
}

#[allow(non_snake_case)]
extern "stdcall" fn mixerGetLineControlsA(_h: u32, _ctl: u32, _flags: u32) -> u32 {
    MMSYSERR_INVALHANDLE
}

#[allow(non_snake_case)]
extern "stdcall" fn mixerGetControlDetailsA(_h: u32, _det: u32, _flags: u32) -> u32 {
    MMSYSERR_INVALHANDLE
}

#[allow(non_snake_case)]
extern "stdcall" fn mixerSetControlDetails(_h: u32, _det: u32, _flags: u32) -> u32 {
    MMSYSERR_INVALHANDLE
}

#[allow(non_snake_case)]
extern "stdcall" fn waveInGetNumDevs() -> u32 {
    0
}

#[allow(non_snake_case)]
extern "stdcall" fn waveInOpen(_out: u32, _dev: u32, _fmt: u32, _cb: u32, _inst: u32, _flags: u32) -> u32 {
    MMSYSERR_BADDEVICEID
}

#[allow(non_snake_case)]
extern "stdcall" fn waveInClose(_h: u32) -> u32 {
    MMSYSERR_INVALHANDLE
}

#[allow(non_snake_case)]
extern "stdcall" fn waveInAddBuffer(_h: u32, _hdr: u32, _size: u32) -> u32 {
    MMSYSERR_INVALHANDLE
}

#[allow(non_snake_case)]
extern "stdcall" fn waveInPrepareHeader(_h: u32, _hdr: u32, _size: u32) -> u32 {
    MMSYSERR_INVALHANDLE
}

#[allow(non_snake_case)]
extern "stdcall" fn waveInUnprepareHeader(_h: u32, _hdr: u32, _size: u32) -> u32 {
    MMSYSERR_INVALHANDLE
}

#[allow(non_snake_case)]
extern "stdcall" fn waveInReset(_h: u32) -> u32 {
    MMSYSERR_INVALHANDLE
}

#[allow(non_snake_case)]
extern "stdcall" fn waveInStart(_h: u32) -> u32 {
    MMSYSERR_INVALHANDLE
}

#[allow(non_snake_case)]
extern "stdcall" fn joyGetPosEx(_id: u32, _info: u32) -> u32 {
    167 // JOYERR_UNPLUGGED
}
