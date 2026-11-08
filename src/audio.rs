//! Áudio: o `winlayer` não fala com a placa de som; ele publica eventos ("tocar o som N") e o
//! visualizador do navegador reproduz com Web Audio. Aqui ficam o catálogo de sons (arquivos WAV
//! inteiros), a fila de eventos e utilitários de WAV.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

struct State {
    sounds: BTreeMap<usize, Arc<Vec<u8>>>,
    by_path: BTreeMap<String, usize>,
    next_id: usize,
    /// Eventos pendentes para o visualizador: (sequência, linha).
    events: VecDeque<(u64, String)>,
    seq: u64,
}

static STATE: Mutex<State> = Mutex::new(State { sounds: BTreeMap::new(), by_path: BTreeMap::new(), next_id: 1, events: VecDeque::new(), seq: 0 });

/// Quantos sons "de fluxo" (waveOut) guardar antes de descartar os mais antigos.
const STREAM_KEEP: usize = 64;

fn is_wav(b: &[u8]) -> bool {
    b.len() > 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WAVE"
}

fn log_on() -> bool {
    std::env::var_os("WINLAYER_AUDIO_LOG").is_some()
}

fn push_event(s: &mut State, line: String) {
    s.seq += 1;
    let seq = s.seq;
    s.events.push_back((seq, line));
    while s.events.len() > 512 {
        s.events.pop_front();
    }
}

/// Registra um WAV (arquivo inteiro na memória); `key` evita duplicar o mesmo arquivo.
fn register(bytes: Vec<u8>, key: Option<String>, stream: bool) -> Option<usize> {
    if !is_wav(&bytes) {
        return None;
    }
    let mut s = STATE.lock().unwrap();
    if let Some(k) = &key {
        if let Some(&id) = s.by_path.get(k) {
            return Some(id);
        }
    }
    let id = s.next_id;
    s.next_id += 1;
    s.sounds.insert(id, Arc::new(bytes));
    if let Some(k) = key {
        s.by_path.insert(k, id);
    }
    if stream {
        // descarta os fluxos mais antigos (os sons de arquivo ficam)
        let cached: Vec<usize> = s.by_path.values().copied().collect();
        let old: Vec<usize> = s.sounds.keys().copied().filter(|i| !cached.contains(i)).collect();
        if old.len() > STREAM_KEEP {
            for i in &old[..old.len() - STREAM_KEEP] {
                s.sounds.remove(i);
            }
        }
    }
    Some(id)
}

/// Duração aproximada em ms de um WAV (campo `avgBytesPerSec` e tamanho do chunk `data`).
pub fn duration_ms(wav: &[u8]) -> u32 {
    let (mut avg, mut data) = (0u32, 0u32);
    let mut p = 12;
    while p + 8 <= wav.len() {
        let size = u32::from_le_bytes([wav[p + 4], wav[p + 5], wav[p + 6], wav[p + 7]]) as usize;
        match &wav[p..p + 4] {
            b"fmt " if p + 20 <= wav.len() => avg = u32::from_le_bytes([wav[p + 16], wav[p + 17], wav[p + 18], wav[p + 19]]),
            b"data" => data = size as u32,
            _ => {}
        }
        p += 8 + size + (size & 1);
    }
    if avg == 0 { 0 } else { (data as u64 * 1000 / avg as u64) as u32 }
}

/// Toca um arquivo WAV do disco (caminho do host). `looped` repete até `stop`.
pub fn play_file(path: &std::path::Path, looped: bool) -> bool {
    let key = path.to_string_lossy().into_owned();
    let cached = STATE.lock().unwrap().by_path.get(&key).copied();
    let id = match cached {
        Some(id) => id,
        None => match std::fs::read(path).ok().and_then(|b| register(b, Some(key.clone()), false)) {
            Some(id) => id,
            None => return false,
        },
    };
    emit(id, if looped { "loop" } else { "once" }, &key);
    true
}

/// Toca um WAV que já está na memória (SND_MEMORY).
pub fn play_memory(bytes: Vec<u8>, looped: bool) -> bool {
    match register(bytes, None, true) {
        Some(id) => {
            emit(id, if looped { "loop" } else { "once" }, "(memória)");
            true
        }
        None => false,
    }
}

/// Fluxo de PCM (waveOut): os blocos tocam em sequência. Devolve a duração em ms.
pub fn play_stream(fmt: &[u8], pcm: &[u8]) -> u32 {
    let wav = pcm_to_wav(fmt, pcm);
    let dur = duration_ms(&wav);
    if let Some(id) = register(wav, None, true) {
        emit(id, "stream", "(waveOut)");
    }
    dur
}

fn emit(id: usize, mode: &str, what: &str) {
    let mut s = STATE.lock().unwrap();
    if log_on() {
        let dur = s.sounds.get(&id).map_or(0, |b| duration_ms(b));
        eprintln!("[audio] {mode} #{id} {what} ({dur} ms)");
    }
    push_event(&mut s, format!("play|{id}|{mode}"));
}

pub fn stop() {
    let mut s = STATE.lock().unwrap();
    if log_on() {
        eprintln!("[audio] stop");
    }
    push_event(&mut s, "stop|0|once".into());
}

/// Bipe curto sintetizado (MessageBeep).
pub fn beep() {
    let rate = 22050u32;
    let n = (rate as usize * 120) / 1000;
    let mut pcm = Vec::with_capacity(n * 2);
    for i in 0..n {
        let env = 1.0 - i as f32 / n as f32;
        let v = (f32::sin(i as f32 * 880.0 * std::f32::consts::TAU / rate as f32) * 9000.0 * env) as i16;
        pcm.extend_from_slice(&v.to_le_bytes());
    }
    let mut fmt = Vec::new();
    for v in [1u16, 1] {
        fmt.extend_from_slice(&v.to_le_bytes());
    }
    fmt.extend_from_slice(&rate.to_le_bytes());
    fmt.extend_from_slice(&(rate * 2).to_le_bytes());
    fmt.extend_from_slice(&2u16.to_le_bytes());
    fmt.extend_from_slice(&16u16.to_le_bytes());
    play_memory(pcm_to_wav(&fmt, &pcm), false);
}

/// Monta um WAV a partir de um WAVEFORMAT/WAVEFORMATEX (usa os 16 primeiros bytes) e dados PCM.
pub fn pcm_to_wav(fmt: &[u8], pcm: &[u8]) -> Vec<u8> {
    let fmt = &fmt[..fmt.len().min(16)];
    let mut w = Vec::with_capacity(44 + pcm.len());
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&((36 + pcm.len()) as u32).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
    w.extend_from_slice(fmt);
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    w.extend_from_slice(pcm);
    w
}

// ---- lado do visualizador ----------------------------------------------------------------------

pub fn wav_bytes(id: usize) -> Option<Arc<Vec<u8>>> {
    STATE.lock().unwrap().sounds.get(&id).cloned()
}

pub fn seq() -> u64 {
    STATE.lock().unwrap().seq
}

/// Eventos com sequência > `since`, uma linha por evento no formato `seq|op|id|modo`.
pub fn events_since(since: u64) -> String {
    let s = STATE.lock().unwrap();
    s.events.iter().filter(|(q, _)| *q > since).map(|(q, l)| format!("{q}|{l}\n")).collect()
}
