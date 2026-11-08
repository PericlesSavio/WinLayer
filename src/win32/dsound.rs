//! dsound.dll — DirectSound (IDirectSound, IDirectSoundBuffer) com mixer em software.
//!
//! Cada buffer secundário é um bloco de PCM (8/16 bits, mono/estéreo, qualquer taxa) que o jogo
//! preenche via `Lock`/`Unlock`. O mixer do `pulse.rs` chama `mix` a cada 10 ms: soma os buffers que
//! estão tocando (com volume, pan e frequência) e avança o cursor de reprodução de cada um.
//! Sem saída de áudio (modo captura, visualizador do navegador), os cursores avançam pelo relógio do
//! programa, para que jogos que esperam o som andar não travem.

use super::{call4, user32};
use std::cell::UnsafeCell;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

pub fn resolve(name: &str) -> Option<usize> {
    exports!(name, DirectSoundCreate, DirectSoundEnumerateA)
}

const DS_OK: u32 = 0;
const E_NOINTERFACE: u32 = 0x8000_4002;
const DSERR_INVALIDPARAM: u32 = 0x8007_0057;
const DSERR_INVALIDCALL: u32 = 0x8878_0032;
const DSERR_BADFORMAT: u32 = 0x8878_0064;

const DSBCAPS_PRIMARYBUFFER: u32 = 0x1;
const DSBPLAY_LOOPING: u32 = 0x1;
const DSBSTATUS_PLAYING: u32 = 0x1;
const DSBSTATUS_LOOPING: u32 = 0x4;
const DSBLOCK_FROMWRITECURSOR: u32 = 0x1;
const DSBLOCK_ENTIREBUFFER: u32 = 0x2;

/// Taxa de saída do mixer (a mesma do `pulse.rs`).
pub const OUT_RATE: u32 = 44_100;

fn trace() -> bool {
    std::env::var_os("WINLAYER_DSOUND_TRACE").is_some()
}

/// Memória de PCM compartilhada entre o jogo (escreve via Lock) e o mixer (lê). As corridas entre os
/// dois são as mesmas de uma placa de som real: o jogo escreve à frente do cursor de reprodução.
struct Pcm(UnsafeCell<Vec<u8>>);
unsafe impl Sync for Pcm {}
unsafe impl Send for Pcm {}

impl Pcm {
    fn ptr(&self) -> *mut u8 {
        unsafe { (*self.0.get()).as_mut_ptr() }
    }
    fn len(&self) -> usize {
        unsafe { (*self.0.get()).len() }
    }
}

#[derive(Clone, Copy)]
struct Wfx {
    ch: u32,
    rate: u32,
    bits: u32,
}

impl Wfx {
    fn align(&self) -> u32 {
        (self.ch * self.bits / 8).max(1)
    }
}

struct Buffer {
    pcm: Arc<Pcm>,
    fmt: Wfx,
    flags: u32,
    /// 0 = frequência original do formato.
    freq: u32,
    /// Atenuação em centésimos de dB (0 = volume máximo, -10000 = silêncio).
    volume: i32,
    /// -10000 (só esquerda) .. 10000 (só direita).
    pan: i32,
    playing: bool,
    looping: bool,
    /// Posição de reprodução em quadros (fracionária por causa da conversão de taxa).
    pos: f64,
    refs: u32,
    primary: bool,
}

impl Buffer {
    fn frames(&self) -> usize {
        self.pcm.len() / self.fmt.align() as usize
    }

    fn play_byte(&self) -> u32 {
        (self.pos as u32 * self.fmt.align()) % self.pcm.len().max(1) as u32
    }
}

struct State {
    ds: u32,
    buffers: BTreeMap<u32, Buffer>,
    /// Último instante (ms do programa) em que os cursores avançaram sem o mixer.
    last_ms: u32,
}

static DS: Mutex<State> = Mutex::new(State { ds: 0, buffers: BTreeMap::new(), last_ms: 0 });

/// O mixer do PulseAudio está rodando e chama `mix` (então ele é quem avança os cursores).
static MIXER_ON: AtomicBool = AtomicBool::new(false);

pub fn set_mixer_on() {
    MIXER_ON.store(true, Ordering::Relaxed);
}

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

unsafe fn rd16(p: u32, o: u32) -> u32 {
    std::ptr::read_unaligned((p + o) as usize as *const u16) as u32
}

unsafe fn wr(p: u32, o: u32, v: u32) {
    std::ptr::write_unaligned((p + o) as usize as *mut u32, v)
}

unsafe fn read_wfx(p: u32) -> Option<Wfx> {
    if p == 0 {
        return None;
    }
    let (tag, ch, rate, bits) = (rd16(p, 0), rd16(p, 2), rd(p, 4), rd16(p, 14));
    if tag != 1 || !(1..=2).contains(&ch) || rate == 0 || !(bits == 8 || bits == 16) {
        return None;
    }
    Some(Wfx { ch, rate, bits })
}

// ---- mixer ------------------------------------------------------------------------------------

/// Lê o quadro `f` como (esquerda, direita) em escala de 16 bits.
fn sample(p: *const u8, fmt: Wfx, f: usize) -> (i32, i32) {
    unsafe {
        let o = f * fmt.align() as usize;
        let s = |i: usize| -> i32 {
            if fmt.bits == 16 {
                i16::from_le_bytes([*p.add(o + i * 2), *p.add(o + i * 2 + 1)]) as i32
            } else {
                (*p.add(o + i) as i32 - 128) << 8
            }
        };
        if fmt.ch == 2 { (s(0), s(1)) } else { let v = s(0); (v, v) }
    }
}

fn gain(db100: i32) -> f32 {
    if db100 <= -10000 { 0.0 } else { 10f32.powf(db100 as f32 / 2000.0) }
}

/// Soma `frames` quadros estéreo dos buffers tocando em `out` (intercalado E/D) e avança os cursores.
/// Com `out == None` só avança.
fn run_mix(s: &mut State, mut out: Option<&mut [i32]>, frames: usize) {
    for b in s.buffers.values_mut() {
        if !b.playing || b.primary {
            continue;
        }
        let n = b.frames();
        if n == 0 {
            b.playing = false;
            continue;
        }
        let step = (if b.freq != 0 { b.freq } else { b.fmt.rate }) as f64 / OUT_RATE as f64;
        let Some(out) = out.as_deref_mut() else {
            b.pos += step * frames as f64;
            if b.pos >= n as f64 {
                if b.looping {
                    b.pos %= n as f64;
                } else {
                    b.playing = false;
                    b.pos = 0.0;
                }
            }
            continue;
        };
        let v = gain(b.volume);
        let (gl, gr) = (v * if b.pan > 0 { gain(-b.pan) } else { 1.0 }, v * if b.pan < 0 { gain(b.pan) } else { 1.0 });
        let p = b.pcm.ptr();
        for f in 0..frames {
            if b.pos >= n as f64 {
                if b.looping {
                    b.pos -= n as f64;
                } else {
                    b.playing = false;
                    b.pos = 0.0;
                    break;
                }
            }
            let (l, r) = sample(p, b.fmt, (b.pos as usize).min(n - 1));
            out[f * 2] += (l as f32 * gl) as i32;
            out[f * 2 + 1] += (r as f32 * gr) as i32;
            b.pos += step;
        }
    }
}

/// Chamado pelo mixer do PulseAudio a cada bloco.
pub fn mix(out: &mut [i32], frames: usize) {
    let mut s = DS.lock().unwrap();
    if !s.buffers.is_empty() {
        run_mix(&mut s, Some(out), frames);
    }
}

/// Sem mixer, avança os cursores pelo relógio do programa.
fn advance_by_clock(s: &mut State) {
    if MIXER_ON.load(Ordering::Relaxed) {
        return;
    }
    let now = user32::now_ms();
    let dt = now.wrapping_sub(s.last_ms);
    s.last_ms = now;
    if dt > 0 && dt < 10_000 {
        run_mix(s, None, (dt as u64 * OUT_RATE as u64 / 1000) as usize);
    }
}

// ---- API exportada ------------------------------------------------------------------------

#[allow(non_snake_case)]
unsafe extern "stdcall" fn DirectSoundCreate(_guid: u32, out: *mut u32, _outer: u32) -> u32 {
    if out.is_null() {
        return DSERR_INVALIDPARAM;
    }
    let mut s = DS.lock().unwrap();
    if s.ds == 0 {
        s.ds = new_obj(ds_vtbl());
        s.last_ms = user32::now_ms();
    }
    if trace() {
        eprintln!("[dsound] DirectSoundCreate -> {:#x}", s.ds);
    }
    *out = s.ds;
    DS_OK
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn DirectSoundEnumerateA(cb: u32, ctx: u32) -> u32 {
    if cb != 0 {
        call4(cb, 0, c"Primary Sound Driver".as_ptr() as u32, c"".as_ptr() as u32, ctx);
    }
    DS_OK
}

// ---- IDirectSound -------------------------------------------------------------------------

fn ds_vtbl() -> &'static [u32] {
    vtable![
        ds_qi, ds_addref, ds_release, ds_create_buffer, ds_get_caps, ds_duplicate, ds_ok3, ds_ok1,
        ds_get_speaker, ds_ok2, ds_ok2,
    ]
}

unsafe extern "stdcall" fn ds_qi(this: u32, riid: u32, out: *mut u32) -> u32 {
    // IUnknown, IID_IDirectSound {279AFA83-...}
    if riid == 0 || matches!(rd(riid, 0), 0 | 0x279A_FA83) {
        *out = this;
        DS_OK
    } else {
        *out = 0;
        E_NOINTERFACE
    }
}

extern "stdcall" fn ds_addref(_this: u32) -> u32 {
    1
}

extern "stdcall" fn ds_release(_this: u32) -> u32 {
    0
}

extern "stdcall" fn ds_ok1(_this: u32) -> u32 {
    DS_OK
}

extern "stdcall" fn ds_ok2(_this: u32, _a: u32) -> u32 {
    DS_OK
}

extern "stdcall" fn ds_ok3(_this: u32, _a: u32, _b: u32) -> u32 {
    DS_OK
}

unsafe extern "stdcall" fn ds_get_speaker(_this: u32, out: *mut u32) -> u32 {
    if !out.is_null() {
        *out = 4; // DSSPEAKER_STEREO
    }
    DS_OK
}

unsafe extern "stdcall" fn ds_get_caps(_this: u32, caps: u32) -> u32 {
    if caps == 0 {
        return DSERR_INVALIDPARAM;
    }
    let size = rd(caps, 0).clamp(8, 96);
    std::ptr::write_bytes((caps + 4) as usize as *mut u8, 0, size as usize - 4);
    // PRIMARYMONO|STEREO|8BIT|16BIT|CONTINUOUSRATE|EMULDRIVER(não)|SECONDARYMONO|STEREO|8BIT|16BIT
    wr(caps, 4, 0x1 | 0x2 | 0x4 | 0x8 | 0x10 | 0x100 | 0x200 | 0x400 | 0x800);
    if size >= 16 {
        wr(caps, 8, 100); // dwMinSecondarySampleRate
        wr(caps, 12, 100_000);
    }
    DS_OK
}

unsafe extern "stdcall" fn ds_create_buffer(_this: u32, desc: u32, out: *mut u32, _outer: u32) -> u32 {
    if desc == 0 || out.is_null() {
        return DSERR_INVALIDPARAM;
    }
    let flags = rd(desc, 4);
    let bytes = rd(desc, 8);
    let fmt = read_wfx(rd(desc, 16));
    let obj = new_obj(buffer_vtbl());
    let buf = if flags & DSBCAPS_PRIMARYBUFFER != 0 {
        Buffer {
            pcm: Arc::new(Pcm(UnsafeCell::new(vec![0; 4096]))),
            fmt: Wfx { ch: 2, rate: 22050, bits: 16 },
            flags,
            freq: 0,
            volume: 0,
            pan: 0,
            playing: false,
            looping: false,
            pos: 0.0,
            refs: 1,
            primary: true,
        }
    } else {
        let Some(fmt) = fmt else {
            if trace() {
                eprintln!("[dsound] CreateSoundBuffer: formato não suportado");
            }
            return DSERR_BADFORMAT;
        };
        if bytes < 4 || bytes > 64 << 20 {
            return DSERR_INVALIDPARAM;
        }
        let silence = if fmt.bits == 8 { 0x80 } else { 0 };
        Buffer {
            pcm: Arc::new(Pcm(UnsafeCell::new(vec![silence; bytes as usize]))),
            fmt,
            flags,
            freq: 0,
            volume: 0,
            pan: 0,
            playing: false,
            looping: false,
            pos: 0.0,
            refs: 1,
            primary: false,
        }
    };
    if trace() {
        eprintln!(
            "[dsound] CreateSoundBuffer {} {bytes} bytes {} Hz {} bits {} canal(is) -> {obj:#x}",
            if buf.primary { "primário" } else { "secundário" },
            buf.fmt.rate,
            buf.fmt.bits,
            buf.fmt.ch
        );
    }
    DS.lock().unwrap().buffers.insert(obj, buf);
    *out = obj;
    DS_OK
}

unsafe extern "stdcall" fn ds_duplicate(_this: u32, orig: u32, out: *mut u32) -> u32 {
    let mut s = DS.lock().unwrap();
    let Some(o) = s.buffers.get(&orig) else { return DSERR_INVALIDPARAM };
    let dup = Buffer {
        pcm: o.pcm.clone(),
        fmt: o.fmt,
        flags: o.flags,
        freq: o.freq,
        volume: o.volume,
        pan: o.pan,
        playing: false,
        looping: false,
        pos: 0.0,
        refs: 1,
        primary: o.primary,
    };
    let obj = new_obj(buffer_vtbl());
    s.buffers.insert(obj, dup);
    *out = obj;
    DS_OK
}

// ---- IDirectSoundBuffer -------------------------------------------------------------------

fn buffer_vtbl() -> &'static [u32] {
    vtable![
        b_qi, b_addref, b_release, b_get_caps, b_get_pos, b_get_format, b_get_volume, b_get_pan,
        b_get_freq, b_get_status, b_init, b_lock, b_play, b_set_pos, b_set_format, b_set_volume,
        b_set_pan, b_set_freq, b_stop, b_unlock, b_restore,
    ]
}

unsafe extern "stdcall" fn b_qi(this: u32, riid: u32, out: *mut u32) -> u32 {
    // IUnknown, IID_IDirectSoundBuffer {279AFA85-...}
    if riid == 0 || matches!(rd(riid, 0), 0 | 0x279A_FA85) {
        b_addref(this);
        *out = this;
        DS_OK
    } else {
        *out = 0;
        E_NOINTERFACE
    }
}

extern "stdcall" fn b_addref(this: u32) -> u32 {
    let mut s = DS.lock().unwrap();
    s.buffers.get_mut(&this).map_or(0, |b| {
        b.refs += 1;
        b.refs
    })
}

extern "stdcall" fn b_release(this: u32) -> u32 {
    let mut s = DS.lock().unwrap();
    let Some(b) = s.buffers.get_mut(&this) else { return 0 };
    b.refs = b.refs.saturating_sub(1);
    let left = b.refs;
    if left == 0 {
        s.buffers.remove(&this);
    }
    left
}

unsafe extern "stdcall" fn b_get_caps(this: u32, caps: u32) -> u32 {
    let s = DS.lock().unwrap();
    let Some(b) = s.buffers.get(&this) else { return DSERR_INVALIDCALL };
    if caps == 0 {
        return DSERR_INVALIDPARAM;
    }
    wr(caps, 4, b.flags | 0x8); // DSBCAPS_LOCSOFTWARE
    wr(caps, 8, b.pcm.len() as u32);
    wr(caps, 12, 0);
    wr(caps, 16, 0);
    DS_OK
}

unsafe extern "stdcall" fn b_get_pos(this: u32, play: *mut u32, write: *mut u32) -> u32 {
    let mut s = DS.lock().unwrap();
    advance_by_clock(&mut s);
    let Some(b) = s.buffers.get(&this) else { return DSERR_INVALIDCALL };
    let p = b.play_byte();
    let len = b.pcm.len().max(1) as u32;
    // cursor de escrita ~15 ms à frente do de reprodução (como as placas da época)
    let rate = if b.freq != 0 { b.freq } else { b.fmt.rate };
    let ahead = if b.playing { (rate * 15 / 1000) * b.fmt.align() } else { 0 };
    if !play.is_null() {
        *play = p;
    }
    if !write.is_null() {
        *write = (p + ahead) % len;
    }
    DS_OK
}

unsafe extern "stdcall" fn b_get_format(this: u32, wfx: u32, size: u32, written: *mut u32) -> u32 {
    let s = DS.lock().unwrap();
    let Some(b) = s.buffers.get(&this) else { return DSERR_INVALIDCALL };
    let f = b.fmt;
    let mut raw = [0u8; 18];
    raw[0..2].copy_from_slice(&1u16.to_le_bytes());
    raw[2..4].copy_from_slice(&(f.ch as u16).to_le_bytes());
    raw[4..8].copy_from_slice(&f.rate.to_le_bytes());
    raw[8..12].copy_from_slice(&(f.rate * f.align()).to_le_bytes());
    raw[12..14].copy_from_slice(&(f.align() as u16).to_le_bytes());
    raw[14..16].copy_from_slice(&(f.bits as u16).to_le_bytes());
    if wfx != 0 {
        let n = (size as usize).min(18);
        std::ptr::copy_nonoverlapping(raw.as_ptr(), wfx as usize as *mut u8, n);
    }
    if !written.is_null() {
        *written = if wfx != 0 { size.min(18) } else { 18 };
    }
    DS_OK
}

unsafe extern "stdcall" fn b_get_volume(this: u32, out: *mut i32) -> u32 {
    let s = DS.lock().unwrap();
    let Some(b) = s.buffers.get(&this) else { return DSERR_INVALIDCALL };
    *out = b.volume;
    DS_OK
}

unsafe extern "stdcall" fn b_get_pan(this: u32, out: *mut i32) -> u32 {
    let s = DS.lock().unwrap();
    let Some(b) = s.buffers.get(&this) else { return DSERR_INVALIDCALL };
    *out = b.pan;
    DS_OK
}

unsafe extern "stdcall" fn b_get_freq(this: u32, out: *mut u32) -> u32 {
    let s = DS.lock().unwrap();
    let Some(b) = s.buffers.get(&this) else { return DSERR_INVALIDCALL };
    *out = if b.freq != 0 { b.freq } else { b.fmt.rate };
    DS_OK
}

unsafe extern "stdcall" fn b_get_status(this: u32, out: *mut u32) -> u32 {
    let mut s = DS.lock().unwrap();
    advance_by_clock(&mut s);
    let Some(b) = s.buffers.get(&this) else { return DSERR_INVALIDCALL };
    *out = if b.playing || (b.primary && b.looping) {
        DSBSTATUS_PLAYING | if b.looping { DSBSTATUS_LOOPING } else { 0 }
    } else {
        0
    };
    DS_OK
}

extern "stdcall" fn b_init(_this: u32, _ds: u32, _desc: u32) -> u32 {
    DS_OK
}

#[allow(clippy::too_many_arguments)]
unsafe extern "stdcall" fn b_lock(this: u32, offset: u32, bytes: u32, p1: *mut u32, n1: *mut u32, p2: *mut u32, n2: *mut u32, flags: u32) -> u32 {
    let mut s = DS.lock().unwrap();
    advance_by_clock(&mut s);
    let Some(b) = s.buffers.get(&this) else { return DSERR_INVALIDCALL };
    let len = b.pcm.len() as u32;
    let mut off = offset;
    if flags & DSBLOCK_FROMWRITECURSOR != 0 {
        let rate = if b.freq != 0 { b.freq } else { b.fmt.rate };
        off = (b.play_byte() + if b.playing { rate * 15 / 1000 * b.fmt.align() } else { 0 }) % len.max(1);
    }
    let bytes = if flags & DSBLOCK_ENTIREBUFFER != 0 { len } else { bytes.min(len) };
    if off >= len || p1.is_null() || n1.is_null() {
        return DSERR_INVALIDPARAM;
    }
    let base = b.pcm.ptr() as u32;
    let first = bytes.min(len - off);
    *p1 = base + off;
    *n1 = first;
    let rest = bytes - first;
    if !p2.is_null() {
        *p2 = if rest > 0 { base } else { 0 };
    }
    if !n2.is_null() {
        *n2 = rest;
    }
    DS_OK
}

extern "stdcall" fn b_unlock(_this: u32, _p1: u32, _n1: u32, _p2: u32, _n2: u32) -> u32 {
    DS_OK
}

extern "stdcall" fn b_play(this: u32, _r: u32, _prio: u32, flags: u32) -> u32 {
    let mut s = DS.lock().unwrap();
    advance_by_clock(&mut s);
    let Some(b) = s.buffers.get_mut(&this) else { return DSERR_INVALIDCALL };
    b.playing = true;
    b.looping = flags & DSBPLAY_LOOPING != 0;
    if trace() {
        eprintln!("[dsound] Play {this:#x} {}", if b.looping { "em laço" } else { "uma vez" });
    }
    DS_OK
}

extern "stdcall" fn b_stop(this: u32) -> u32 {
    let mut s = DS.lock().unwrap();
    advance_by_clock(&mut s);
    if let Some(b) = s.buffers.get_mut(&this) {
        b.playing = false;
    }
    DS_OK
}

extern "stdcall" fn b_set_pos(this: u32, pos: u32) -> u32 {
    let mut s = DS.lock().unwrap();
    let Some(b) = s.buffers.get_mut(&this) else { return DSERR_INVALIDCALL };
    b.pos = ((pos % b.pcm.len().max(1) as u32) / b.fmt.align()) as f64;
    DS_OK
}

unsafe extern "stdcall" fn b_set_format(this: u32, wfx: u32) -> u32 {
    let mut s = DS.lock().unwrap();
    let Some(b) = s.buffers.get_mut(&this) else { return DSERR_INVALIDCALL };
    if !b.primary {
        return DSERR_INVALIDCALL;
    }
    if let Some(f) = read_wfx(wfx) {
        b.fmt = f;
    }
    DS_OK
}

extern "stdcall" fn b_set_volume(this: u32, v: i32) -> u32 {
    let mut s = DS.lock().unwrap();
    let Some(b) = s.buffers.get_mut(&this) else { return DSERR_INVALIDCALL };
    b.volume = v.clamp(-10000, 0);
    DS_OK
}

extern "stdcall" fn b_set_pan(this: u32, v: i32) -> u32 {
    let mut s = DS.lock().unwrap();
    let Some(b) = s.buffers.get_mut(&this) else { return DSERR_INVALIDCALL };
    b.pan = v.clamp(-10000, 10000);
    DS_OK
}

extern "stdcall" fn b_set_freq(this: u32, f: u32) -> u32 {
    let mut s = DS.lock().unwrap();
    let Some(b) = s.buffers.get_mut(&this) else { return DSERR_INVALIDCALL };
    b.freq = if f == 0 { 0 } else { f.clamp(100, 100_000) };
    DS_OK
}

extern "stdcall" fn b_restore(_this: u32) -> u32 {
    DS_OK
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixa_mono_8_bits_e_para_no_fim() {
        let fmt = Wfx { ch: 1, rate: OUT_RATE, bits: 8 };
        let mut st = State { ds: 0, buffers: BTreeMap::new(), last_ms: 0 };
        st.buffers.insert(1, Buffer {
            pcm: Arc::new(Pcm(UnsafeCell::new(vec![0xFF; 4]))),
            fmt, flags: 0, freq: 0, volume: 0, pan: 0, playing: true, looping: false, pos: 0.0, refs: 1, primary: false,
        });
        let mut out = vec![0i32; 12];
        run_mix(&mut st, Some(&mut out), 6);
        assert_eq!(out[0], 127 << 8);
        assert_eq!(out[1], 127 << 8);
        assert_eq!(out[8], 0); // quinto quadro: o buffer acabou
        assert!(!st.buffers[&1].playing);
    }

    #[test]
    fn pan_total_a_direita_silencia_a_esquerda() {
        assert_eq!(gain(-10000), 0.0);
        assert!((gain(0) - 1.0).abs() < 1e-6);
    }
}
