//! winmm.dll — sound (sndPlaySound, PlaySound, waveOut), multimedia timers and stubs for MCI,
//! joystick and MIDI. Audio output goes to the viewer (see `audio.rs`).

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
        waveInUnprepareHeader, waveInReset, waveInStart, waveInGetNumDevs, mmioOpenA, mmioAscend, mmioDescend, mmioRead, mmioClose, mmioSeek, mmioGetInfo, mmioSetInfo, mmioSetBuffer, mmioAdvance,
    )
}

const SND_MEMORY: u32 = 0x04;
const SND_LOOP: u32 = 0x08;
const SND_ALIAS: u32 = 0x10000;
const SND_RESOURCE: u32 = 0x40004;

/// Core of sndPlaySound/PlaySound (also used by the Win16 layer).
pub unsafe fn play_sound(name: u32, flags: u32) -> u32 {
    if name == 0 {
        audio::stop();
        return 1;
    }
    let looped = flags & SND_LOOP != 0;
    if flags & SND_RESOURCE == SND_RESOURCE {
        return 0; // sounds embedded in resources: not yet
    }
    if flags & SND_ALIAS == SND_ALIAS {
        audio::beep(); // "SystemDefault" and the like
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
    user32::tick_virtual(1); // see GetTickCount
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

// ---- multimedia timers --------------------------------------------------------------------

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
                user32::tick_virtual(1); // the capture mode virtual clock also runs with the main thread blocked
                fire_timers();
            }
        });
    });
    TIMERS.lock().unwrap().insert(id, MmTimer { due: user32::now_ms() + interval, interval, periodic: flags & 1 != 0, callback, user });
    id
}

#[allow(non_snake_case)]
extern "stdcall" fn timeKillEvent(id: u32) -> u32 {
    TIMERS.lock().unwrap().remove(&id).is_none() as u32 * 97 // MMSYSERR_INVALPARAM if it does not exist
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
/// Instant (process clock ms) at which the stream finishes playing.
static STREAM_END: Mutex<u32> = Mutex::new(0);
static NEXT_WAVE: Mutex<u32> = Mutex::new(0x6100);

const WHDR_DONE: u32 = 1;
const WHDR_PREPARED: u32 = 2;
const WHDR_INQUEUE: u32 = 0x10;
const MM_WOM_OPEN: u32 = 0x3BB;
const MM_WOM_CLOSE: u32 = 0x3BC;
const MM_WOM_DONE: u32 = 0x3BD;

/// Delivers a notification according to the callback type requested in waveOutOpen.
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
    *(caps.add(40) as *mut u32) = 0xFFF; // dwFormats: all standard formats
    *(caps.add(44) as *mut u16) = 2; // wChannels
    *(caps.add(48) as *mut u32) = 0x0C; // WAVECAPS_VOLUME | WAVECAPS_LRVOLUME
    0
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn waveOutOpen(phwo: *mut u32, _dev: u32, fmt: *const u8, cb: u32, inst: u32, flags: u32) -> u32 {
    if flags & 1 != 0 {
        return 0; // WAVE_FORMAT_QUERY: yes, we support it
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
    // MMTIME: wType + union; we return position 0 in the requested type
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

/// Called on every message cycle: completes waveOut buffers and fires timeSetEvent.
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

/// Fires expired timers. Runs on the timers' own thread, as on Windows: the
/// program may block the main thread waiting for an event only the callback signals.
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

// ---- MCI, joystick, MIDI: unsupported (return error/"no device") ------------------------

#[allow(non_snake_case)]
extern "stdcall" fn mciSendCommandA(_dev: u32, msg: u32, _flags: u32, _param: u32) -> u32 {
    let name = match msg {
        0x803 => "MCI_OPEN",
        0x804 => "MCI_CLOSE",
        0x806 => "MCI_PLAY",
        0x807 => "MCI_SEEK",
        0x808 => "MCI_STOP",
        0x809 => "MCI_PAUSE",
        0x80D => "MCI_SET",
        0x814 => "MCI_STATUS",
        _ => "",
    };
    crate::diag::note(crate::diag::Kind::Unsupported, format!("MCI command {msg:#x} {name} (mciSendCommandA)"));
    263 // MCIERR_UNSUPPORTED_FUNCTION
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn mciSendStringA(cmd: *const i8, _ret: u32, _cch: u32, _hwnd: u32) -> u32 {
    if !cmd.is_null() {
        let c = std::ffi::CStr::from_ptr(cmd).to_string_lossy();
        crate::diag::note(crate::diag::Kind::Unsupported, format!("MCI: \"{c}\" (mciSendStringA)"));
    }
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

// ---- mmio: read-only RIFF files (WAV loading) ---------------------------------------------

/// An open mmio file: read whole into memory (the files games read this way are small).
struct MmioFile {
    data: Vec<u8>,
    pos: usize,
}

static MMIO: Mutex<BTreeMap<u32, MmioFile>> = Mutex::new(BTreeMap::new());
static MMIO_NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x6000);

const MMIO_WRITE: u32 = 0x1;
const MMIO_READWRITE: u32 = 0x2;
const MMIO_CREATE: u32 = 0x1000;
const MMIO_EXIST: u32 = 0x4000;
const MMIO_FINDCHUNK: u32 = 0x10;
const MMIO_FINDRIFF: u32 = 0x20;
const MMIO_FINDLIST: u32 = 0x40;
const MMIOERR_CHUNKNOTFOUND: u32 = 265;
const FOURCC_RIFF: u32 = u32::from_le_bytes(*b"RIFF");
const FOURCC_LIST: u32 = u32::from_le_bytes(*b"LIST");

fn rd32(d: &[u8], o: usize) -> Option<u32> {
    d.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// Only reading named files is supported: writing, memory files and custom I/O procedures fail.
#[allow(non_snake_case)]
unsafe extern "stdcall" fn mmioOpenA(name: *const i8, _info: u32, flags: u32) -> u32 {
    if name.is_null() {
        return 0;
    }
    let win = std::ffi::CStr::from_ptr(name).to_string_lossy().into_owned();
    let path = super::files::host_path(&win);
    if flags & MMIO_EXIST != 0 {
        return path.is_file() as u32;
    }
    if flags & (MMIO_WRITE | MMIO_READWRITE | MMIO_CREATE) != 0 {
        return 0;
    }
    let Ok(data) = std::fs::read(&path) else {
        crate::diag::note(crate::diag::Kind::FileNotFound, format!("{win} -> {} (mmioOpen)", path.display()));
        return 0;
    };
    let h = MMIO_NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    MMIO.lock().unwrap().insert(h, MmioFile { data, pos: 0 });
    h
}

#[allow(non_snake_case)]
extern "stdcall" fn mmioClose(h: u32, _flags: u32) -> u32 {
    if MMIO.lock().unwrap().remove(&h).is_some() { 0 } else { 5 } // MMSYSERR_INVALHANDLE
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn mmioRead(h: u32, buf: *mut u8, n: i32) -> i32 {
    let mut files = MMIO.lock().unwrap();
    let Some(f) = files.get_mut(&h) else { return -1 };
    let n = (n.max(0) as usize).min(f.data.len().saturating_sub(f.pos));
    std::ptr::copy_nonoverlapping(f.data.as_ptr().add(f.pos), buf, n);
    f.pos += n;
    n as i32
}

#[allow(non_snake_case)]
extern "stdcall" fn mmioSeek(h: u32, off: i32, origin: i32) -> i32 {
    let mut files = MMIO.lock().unwrap();
    let Some(f) = files.get_mut(&h) else { return -1 };
    let base = match origin {
        0 => 0,
        1 => f.pos as i64,
        2 => f.data.len() as i64,
        _ => return -1,
    };
    let new = base + off as i64;
    if new < 0 {
        return -1;
    }
    f.pos = new as usize;
    new as i32
}

/// MMCKINFO: ckid, cksize, fccType, dwDataOffset, dwFlags.
#[allow(non_snake_case)]
unsafe extern "stdcall" fn mmioDescend(h: u32, ck: *mut u32, parent: *const u32, flags: u32) -> u32 {
    let mut files = MMIO.lock().unwrap();
    let Some(f) = files.get_mut(&h) else { return 5 };
    let end = if parent.is_null() { f.data.len() } else { (*parent.add(3) + *parent.add(1)) as usize }.min(f.data.len());
    let (want_id, want_type) = (*ck, *ck.add(2));
    let mut pos = f.pos;
    loop {
        let (Some(id), Some(size)) = (rd32(&f.data, pos), rd32(&f.data, pos + 4)) else { return MMIOERR_CHUNKNOTFOUND };
        if pos + 8 > end {
            return MMIOERR_CHUNKNOTFOUND;
        }
        let container = id == FOURCC_RIFF || id == FOURCC_LIST;
        let ftype = if container { rd32(&f.data, pos + 8).unwrap_or(0) } else { 0 };
        let found = match flags & (MMIO_FINDCHUNK | MMIO_FINDRIFF | MMIO_FINDLIST) {
            MMIO_FINDCHUNK => id == want_id,
            MMIO_FINDRIFF => id == FOURCC_RIFF && ftype == want_type,
            MMIO_FINDLIST => id == FOURCC_LIST && ftype == want_type,
            _ => true,
        };
        if found {
            *ck = id;
            *ck.add(1) = size;
            *ck.add(2) = ftype;
            *ck.add(3) = (pos + 8) as u32;
            *ck.add(4) = 0;
            f.pos = pos + 8 + if container { 4 } else { 0 };
            return 0;
        }
        pos += 8 + size as usize + (size & 1) as usize;
    }
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn mmioAscend(h: u32, ck: *const u32, _flags: u32) -> u32 {
    let mut files = MMIO.lock().unwrap();
    let Some(f) = files.get_mut(&h) else { return 5 };
    let size = *ck.add(1);
    f.pos = (*ck.add(3) + size + (size & 1)) as usize;
    0
}

/// Direct access to the I/O buffer is not supported.
#[allow(non_snake_case)]
extern "stdcall" fn mmioGetInfo(_h: u32, _info: u32, _flags: u32) -> u32 {
    5 // MMSYSERR_INVALHANDLE
}

#[allow(non_snake_case)]
extern "stdcall" fn mmioSetInfo(_h: u32, _info: u32, _flags: u32) -> u32 {
    5
}

#[allow(non_snake_case)]
extern "stdcall" fn mmioAdvance(_h: u32, _info: u32, _flags: u32) -> u32 {
    5
}

#[allow(non_snake_case)]
extern "stdcall" fn mmioSetBuffer(_h: u32, _buf: u32, _n: u32, _flags: u32) -> u32 {
    5
}

// ---- devices that do not exist: MIDI, audio input, mixer, aux --------------------------------

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
