//! Native audio output: client for PulseAudio's "native" protocol (also served by
//! PipeWire) over the Unix socket, std only. An in-house mixer plays the events from `audio.rs`
//! (whole WAV sounds, looped or queued for waveOut) in a single S16 stereo 44.1 kHz stream.

use crate::audio;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::time::{Duration, Instant};

const RATE: u32 = 44100;
const VERSION: u32 = 13;
const NO_CHANNEL: u32 = 0xFFFF_FFFF;

// ---- protocol -----------------------------------------------------------------------------

#[derive(Default)]
struct Tags(Vec<u8>);

impl Tags {
    fn u32(&mut self, v: u32) -> &mut Self {
        self.0.push(b'L');
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    fn string(&mut self, s: &str) -> &mut Self {
        self.0.push(b't');
        self.0.extend_from_slice(s.as_bytes());
        self.0.push(0);
        self
    }
    fn null(&mut self) -> &mut Self {
        self.0.push(b'N');
        self
    }
    fn bytes(&mut self, d: &[u8]) -> &mut Self {
        self.0.push(b'x');
        self.0.extend_from_slice(&(d.len() as u32).to_be_bytes());
        self.0.extend_from_slice(d);
        self
    }
    fn boolean(&mut self, b: bool) -> &mut Self {
        self.0.push(if b { b'1' } else { b'0' });
        self
    }
    fn sample_spec(&mut self, format: u8, channels: u8, rate: u32) -> &mut Self {
        self.0.extend_from_slice(&[b'a', format, channels]);
        self.0.extend_from_slice(&rate.to_be_bytes());
        self
    }
    fn channel_map(&mut self, pos: &[u8]) -> &mut Self {
        self.0.extend_from_slice(&[b'm', pos.len() as u8]);
        self.0.extend_from_slice(pos);
        self
    }
    fn cvolume(&mut self, channels: u8, vol: u32) -> &mut Self {
        self.0.extend_from_slice(&[b'v', channels]);
        for _ in 0..channels {
            self.0.extend_from_slice(&vol.to_be_bytes());
        }
        self
    }
    fn proplist(&mut self, props: &[(&str, &str)]) -> &mut Self {
        self.0.push(b'P');
        for (k, v) in props {
            self.string(k);
            self.u32(v.len() as u32 + 1);
            let mut val = v.as_bytes().to_vec();
            val.push(0);
            self.bytes(&val);
        }
        self.null();
        self
    }
}

fn send_packet(s: &mut UnixStream, channel: u32, payload: &[u8]) -> std::io::Result<()> {
    let mut v = Vec::with_capacity(20 + payload.len());
    v.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    v.extend_from_slice(&channel.to_be_bytes());
    v.extend_from_slice(&[0u8; 8]); // offset (64 bits)
    v.extend_from_slice(&0u32.to_be_bytes()); // flags
    v.extend_from_slice(payload);
    s.write_all(&v)
}

/// Reads a packet from the server: (channel, payload).
fn read_packet(s: &mut UnixStream) -> std::io::Result<(u32, Vec<u8>)> {
    let mut d = [0u8; 20];
    s.read_exact(&mut d)?;
    let len = u32::from_be_bytes([d[0], d[1], d[2], d[3]]) as usize;
    let ch = u32::from_be_bytes([d[4], d[5], d[6], d[7]]);
    let mut p = vec![0u8; len];
    s.read_exact(&mut p)?;
    Ok((ch, p))
}

/// Waits for the reply to `tag`; returns the payload after (REPLY, tag) or the server's error.
fn reply(s: &mut UnixStream, tag: u32) -> Result<Vec<u8>, String> {
    loop {
        let (ch, p) = read_packet(s).map_err(|e| format!("reading reply: {e}"))?;
        if ch != NO_CHANNEL || p.len() < 10 || p[0] != b'L' || p[5] != b'L' {
            continue;
        }
        let (cmd, t) = (u32::from_be_bytes([p[1], p[2], p[3], p[4]]), u32::from_be_bytes([p[6], p[7], p[8], p[9]]));
        if t != tag {
            continue;
        }
        return match cmd {
            2 => Ok(p[10..].to_vec()),
            0 => Err(format!("the server refused (error {})", p.get(11..15).map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]])))),
            c => Err(format!("unexpected reply {c}")),
        };
    }
}

fn socket_path() -> Option<String> {
    if let Ok(s) = std::env::var("PULSE_SERVER") {
        if let Some(p) = s.strip_prefix("unix:") {
            return Some(p.to_string());
        }
    }
    let dir = std::env::var("PULSE_RUNTIME_PATH").ok().or_else(|| std::env::var("XDG_RUNTIME_DIR").ok().map(|d| format!("{d}/pulse"))).or_else(|| {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata("/proc/self").ok().map(|m| format!("/run/user/{}/pulse", m.uid()))
    })?;
    Some(format!("{dir}/native"))
}

fn read_cookie() -> Vec<u8> {
    let home = std::env::var("HOME").unwrap_or_default();
    let cfg = std::env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| format!("{home}/.config"));
    for p in [format!("{cfg}/pulse/cookie"), format!("{home}/.pulse-cookie")] {
        if let Ok(mut c) = std::fs::read(p) {
            c.resize(256, 0);
            return c;
        }
    }
    vec![0; 256]
}

/// Connects, authenticates and creates the playback stream; returns (socket, stream channel).
fn connect() -> Result<(UnixStream, u32), String> {
    let path = socket_path().ok_or("no sound server path")?;
    let mut s = UnixStream::connect(&path).map_err(|e| format!("could not connect to {path}: {e}"))?;
    s.set_read_timeout(Some(Duration::from_secs(3))).ok();
    let mut t = Tags::default();
    t.u32(8).u32(1).u32(VERSION).bytes(&read_cookie()); // AUTH
    send_packet(&mut s, NO_CHANNEL, &t.0).map_err(|e| e.to_string())?;
    reply(&mut s, 1)?;
    let mut t = Tags::default();
    t.u32(9).u32(2).proplist(&[("application.name", "winlayer"), ("media.role", "game")]); // SET_CLIENT_NAME
    send_packet(&mut s, NO_CHANNEL, &t.0).map_err(|e| e.to_string())?;
    reply(&mut s, 2)?;
    let mut t = Tags::default();
    t.u32(3).u32(3); // CREATE_PLAYBACK_STREAM
    t.sample_spec(3, 2, RATE).channel_map(&[1, 2]);
    t.u32(NO_CHANNEL).null(); // sink: the default one
    t.u32(NO_CHANNEL); // maxlength
    t.boolean(false); // corked
    t.u32(RATE / 10 * 4); // tlength: ~100 ms
    t.u32(0); // prebuf: start right away
    t.u32(NO_CHANNEL); // minreq
    t.u32(0); // syncid
    t.cvolume(2, 0x10000);
    for _ in 0..7 {
        t.boolean(false); // no_remap, no_remix, fix_*, no_move, variable_rate
    }
    t.boolean(false).boolean(true); // muted, adjust_latency
    t.proplist(&[("media.name", "winlayer")]);
    send_packet(&mut s, NO_CHANNEL, &t.0).map_err(|e| e.to_string())?;
    let r = reply(&mut s, 3)?;
    let channel = if r.len() >= 5 && r[0] == b'L' { u32::from_be_bytes([r[1], r[2], r[3], r[4]]) } else { return Err("invalid stream reply".into()) };
    Ok((s, channel))
}

// ---- WAV -> S16 stereo 44.1 kHz -----------------------------------------------------------

/// Decodes a PCM WAV (8/16 bits, 1/2 channels, any rate) to S16 stereo frames at 44.1 kHz.
fn decode(wav: &[u8]) -> Vec<i16> {
    let (mut tag, mut ch, mut rate, mut bits) = (0u16, 0usize, 0u32, 0u16);
    let mut data: &[u8] = &[];
    let mut p = 12;
    while p + 8 <= wav.len() {
        let size = u32::from_le_bytes([wav[p + 4], wav[p + 5], wav[p + 6], wav[p + 7]]) as usize;
        let body = &wav[(p + 8).min(wav.len())..(p + 8 + size).min(wav.len())];
        match &wav[p..p + 4] {
            b"fmt " if body.len() >= 16 => {
                tag = u16::from_le_bytes([body[0], body[1]]);
                ch = u16::from_le_bytes([body[2], body[3]]) as usize;
                rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
                bits = u16::from_le_bytes([body[14], body[15]]);
            }
            b"data" => data = body,
            _ => {}
        }
        p += 8 + size + (size & 1);
    }
    if tag != 1 || ch == 0 || ch > 2 || rate == 0 || !(bits == 8 || bits == 16) {
        return Vec::new();
    }
    let bps = bits as usize / 8;
    let frames = data.len() / (bps * ch);
    let sample = |f: usize, c: usize| -> i32 {
        let o = (f * ch + c.min(ch - 1)) * bps;
        if bits == 16 { i16::from_le_bytes([data[o], data[o + 1]]) as i32 } else { (data[o] as i32 - 128) << 8 }
    };
    let out_frames = (frames as u64 * RATE as u64 / rate as u64) as usize;
    let mut out = Vec::with_capacity(out_frames * 2);
    for i in 0..out_frames {
        let pos = i as f64 * rate as f64 / RATE as f64;
        let (a, frac) = (pos as usize, pos.fract());
        let b = (a + 1).min(frames.saturating_sub(1));
        for c in 0..2 {
            let v = sample(a.min(frames - 1), c) as f64 * (1.0 - frac) + sample(b, c) as f64 * frac;
            out.push(v as i16);
        }
    }
    out
}

// ---- mixer --------------------------------------------------------------------------------

struct Voice {
    data: Arc<Vec<i16>>,
    pos: usize,
    looped: bool,
    start: u64,
    /// "single" sounds (sndPlaySound) replace the previous one; waveOut streams are queued.
    single: bool,
}

fn log_on() -> bool {
    std::env::var_os("WINLAYER_AUDIO_LOG").is_some()
}

/// Diagnostics: `WINLAYER_PULSE_TONE=1` plays 2 s of a 440 Hz tone through the same path and exits.
fn tone_test(mut s: UnixStream, channel: u32) {
    let t0 = Instant::now();
    let mut sent = 0u64;
    while sent < RATE as u64 * 2 {
        let want = t0.elapsed().as_micros() as u64 * RATE as u64 / 1_000_000 + 3000;
        while sent < want {
            let mut pcm = Vec::with_capacity(441 * 4);
            for i in 0..441u64 {
                let v = (f32::sin((sent + i) as f32 * 440.0 * std::f32::consts::TAU / RATE as f32) * 12000.0) as i16;
                pcm.extend_from_slice(&v.to_le_bytes());
                pcm.extend_from_slice(&v.to_le_bytes());
            }
            if send_packet(&mut s, channel, &pcm).is_err() {
                return;
            }
            sent += 441;
        }
        std::thread::sleep(Duration::from_millis(3));
    }
    std::thread::sleep(Duration::from_millis(300));
}

pub fn start() -> Result<(), String> {
    let (stream, channel) = connect()?;
    if std::env::var_os("WINLAYER_PULSE_TONE").is_some() {
        tone_test(stream, channel);
        return Err("tone test finished".into());
    }
    eprintln!("[winlayer] audio: PulseAudio/PipeWire connected (stream {channel}, {})", socket_path().unwrap_or_default());
    std::thread::spawn(move || {
        if let Err(e) = run(stream, channel) {
            eprintln!("[winlayer] audio stopped: {e}");
        }
    });
    Ok(())
}

fn run(mut s: UnixStream, channel: u32) -> Result<(), String> {
    crate::win32::dsound::set_mixer_on();
    s.set_read_timeout(Some(Duration::from_millis(1))).ok();
    let mut voices: Vec<Voice> = Vec::new();
    let mut cache: std::collections::BTreeMap<usize, Arc<Vec<i16>>> = std::collections::BTreeMap::new();
    let (mut last_seq, mut gframe, mut stream_end) = (audio::seq(), 0u64, 0u64);
    let (mut t0, mut written): (Option<Instant>, u64) = (None, 0);
    let mut sink = [0u8; 4096];
    loop {
        // discards messages from the server (REQUEST etc.) so as not to fill the socket
        match s.read(&mut sink) {
            Ok(0) => return Err("server closed the connection".into()),
            Ok(n) => {
                if log_on() && n >= 25 && sink[20] == b'L' && u32::from_be_bytes([sink[21], sink[22], sink[23], sink[24]]) != 61 {
                    let cmd = u32::from_be_bytes([sink[21], sink[22], sink[23], sink[24]]);
                    eprintln!("[pulse] server -> command {cmd} ({n} bytes)");
                }
            }
            Err(_) => {}
        }
        // new sound events
        if audio::seq() > last_seq {
            let text = audio::events_since(last_seq);
            for line in text.lines() {
                let f: Vec<&str> = line.split('|').collect();
                if let Some(q) = f.first().and_then(|v| v.parse::<u64>().ok()) {
                    last_seq = last_seq.max(q);
                }
                match f.as_slice() {
                    [_, "stop", ..] => {
                        voices.clear();
                        stream_end = 0;
                    }
                    [_, "play", id, mode] => {
                        let Ok(id) = id.parse::<usize>() else { continue };
                        if log_on() {
                            eprintln!("[pulse] event: play sound #{id} mode {mode}");
                        }
                        let data = match cache.get(&id) {
                            Some(d) => d.clone(),
                            None => {
                                let Some(w) = audio::wav_bytes(id) else { continue };
                                let d = Arc::new(decode(&w));
                                cache.insert(id, d.clone());
                                d
                            }
                        };
                        if data.is_empty() {
                            if log_on() {
                                eprintln!("[pulse] sound #{id}: unsupported WAV format (PCM 8/16 bits only)");
                            }
                            continue;
                        }
                        let frames = (data.len() / 2) as u64;
                        match *mode {
                            "stream" => {
                                let start = stream_end.max(gframe);
                                stream_end = start + frames;
                                voices.push(Voice { data, pos: 0, looped: false, start, single: false });
                            }
                            m => {
                                voices.retain(|v| !v.single);
                                voices.push(Voice { data, pos: 0, looped: m == "loop", start: gframe, single: true });
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        // The stream is always kept fed (silence when there are no sounds): PipeWire does not resume an idle stream well.
        let start = *t0.get_or_insert_with(|| {
            written = 0;
            Instant::now()
        });
        let target = (start.elapsed().as_micros() as u64 * RATE as u64 / 1_000_000) + 3000; // ~70 ms of headroom
        let mut sent = false;
        while written < target {
            const CHUNK: usize = 441; // 10 ms
            let mut mix = vec![0i32; CHUNK * 2];
            for v in voices.iter_mut() {
                for f in 0..CHUNK {
                    if gframe + (f as u64) < v.start {
                        continue;
                    }
                    if v.pos * 2 + 1 >= v.data.len() {
                        if v.looped && !v.data.is_empty() {
                            v.pos = 0;
                        } else {
                            break;
                        }
                    }
                    mix[f * 2] += v.data[v.pos * 2] as i32;
                    mix[f * 2 + 1] += v.data[v.pos * 2 + 1] as i32;
                    v.pos += 1;
                }
            }
            crate::win32::dsound::mix(&mut mix, CHUNK); // DirectSound buffers
            let mut pcm = Vec::with_capacity(CHUNK * 4);
            for m in mix {
                pcm.extend_from_slice(&(m.clamp(-32768, 32767) as i16).to_le_bytes());
            }
            if log_on() && !voices.is_empty() && (written == 0 || written % (RATE as u64 * 2) < CHUNK as u64) {
                let peak = pcm.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]).unsigned_abs()).max().unwrap_or(0);
                eprintln!("[pulse] sending audio to the server (frame {written}, peak {peak}, {} voices)", voices.len());
            }
            send_packet(&mut s, channel, &pcm).map_err(|e| e.to_string())?;
            gframe += CHUNK as u64;
            written += CHUNK as u64;
            sent = true;
            voices.retain(|v| v.looped || v.pos * 2 < v.data.len() || v.start > gframe);
        }
        if !sent {
            std::thread::sleep(Duration::from_millis(3));
        }
    }
}
