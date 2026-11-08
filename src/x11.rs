//! Janela nativa X11 falando o protocolo X direto pelo socket Unix (só std, sem Xlib/XCB).
//! Mostra o "desktop" do winlayer numa janela de tamanho fixo e converte mouse/teclado do X
//! para o protocolo de entrada de `win32/user32/input.rs` (`m`, `d`, `u`, `kd`, `ku`, `ch`).

use crate::win32::user32;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

struct Conn {
    s: UnixStream,
    max_req_bytes: usize,
}

fn pad4(n: usize) -> usize {
    (n + 3) & !3
}

fn u16le(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// Lê (nome, dados) do cookie de autenticação do ~/.Xauthority para o display `num`.
fn read_xauth(num: &str) -> Option<(Vec<u8>, Vec<u8>)> {
    let path = std::env::var_os("XAUTHORITY").map(std::path::PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| std::path::Path::new(&h).join(".Xauthority")))?;
    let d = std::fs::read(path).ok()?;
    let mut p = 0;
    let host = std::fs::read_to_string("/etc/hostname").unwrap_or_default().trim().to_string();
    let take = |p: &mut usize| -> Option<Vec<u8>> {
        let n = u16::from_be_bytes([*d.get(*p)?, *d.get(*p + 1)?]) as usize;
        let v = d.get(*p + 2..*p + 2 + n)?.to_vec();
        *p += 2 + n;
        Some(v)
    };
    let mut best = None;
    while p + 2 <= d.len() {
        let family = u16::from_be_bytes([d[p], d[p + 1]]);
        p += 2;
        let addr = take(&mut p)?;
        let number = take(&mut p)?;
        let name = take(&mut p)?;
        let data = take(&mut p)?;
        let num_ok = number.is_empty() || number == num.as_bytes();
        let host_ok = family == 0xFFFF || (family == 256 && (addr == host.as_bytes() || host.is_empty()));
        if num_ok && host_ok && name == b"MIT-MAGIC-COOKIE-1" {
            best = Some((name, data));
        }
    }
    best
}

impl Conn {
    fn connect() -> Result<(Conn, Setup), String> {
        let display = std::env::var("DISPLAY").ok().filter(|d| !d.is_empty()).unwrap_or_else(|| ":0".into());
        let after = display.rsplit(':').next().unwrap_or("0");
        let num = after.split('.').next().unwrap_or("0");
        if !(display.starts_with(':') || display.starts_with("unix:")) {
            return Err(format!("DISPLAY={display}: só display local (:N) é suportado"));
        }
        let path = format!("/tmp/.X11-unix/X{num}");
        let mut s = UnixStream::connect(&path).map_err(|e| format!("não consegui conectar em {path}: {e}"))?;
        s.set_read_timeout(Some(Duration::from_secs(5))).ok();
        let (aname, adata) = read_xauth(num).unwrap_or_default();
        let mut req = vec![b'l', 0, 11, 0, 0, 0];
        req.extend_from_slice(&(aname.len() as u16).to_le_bytes());
        req.extend_from_slice(&(adata.len() as u16).to_le_bytes());
        req.extend_from_slice(&[0, 0]);
        req.extend_from_slice(&aname);
        req.resize(req.len() + pad4(aname.len()) - aname.len(), 0);
        req.extend_from_slice(&adata);
        req.resize(req.len() + pad4(adata.len()) - adata.len(), 0);
        s.write_all(&req).map_err(|e| e.to_string())?;
        let mut head = [0u8; 8];
        s.read_exact(&mut head).map_err(|e| format!("sem resposta do servidor X: {e}"))?;
        let extra = u16le(&head, 6) as usize * 4;
        let mut a = vec![0u8; extra];
        s.read_exact(&mut a).map_err(|e| e.to_string())?;
        if head[0] != 1 {
            let n = (head[1] as usize).min(a.len());
            return Err(format!("servidor X recusou a conexão: {}", String::from_utf8_lossy(&a[..n])));
        }
        let setup = Setup::parse(&a)?;
        let max = u16le(&a, 18) as usize * 4;
        Ok((Conn { s, max_req_bytes: max }, setup))
    }

    /// Envia uma requisição: opcode, byte de dados, corpo (preenchido até 4 bytes).
    fn req(&mut self, opcode: u8, data: u8, body: &[u8]) -> std::io::Result<()> {
        let total = 4 + pad4(body.len());
        let mut v = Vec::with_capacity(total);
        v.push(opcode);
        v.push(data);
        v.extend_from_slice(&((total / 4) as u16).to_le_bytes());
        v.extend_from_slice(body);
        v.resize(total, 0);
        self.s.write_all(&v)
    }

    /// Lê até chegar uma resposta (byte 1); descarta eventos que chegarem antes.
    fn reply(&mut self) -> Result<Vec<u8>, String> {
        loop {
            let mut h = [0u8; 32];
            self.s.read_exact(&mut h).map_err(|e| format!("lendo resposta X: {e}"))?;
            match h[0] {
                1 => {
                    let mut v = h.to_vec();
                    let extra = u32le(&h, 4) as usize * 4;
                    let mut more = vec![0u8; extra];
                    self.s.read_exact(&mut more).map_err(|e| e.to_string())?;
                    v.extend(more);
                    return Ok(v);
                }
                0 => return Err(format!("erro X: código {} opcode {}", h[1], h[10])),
                _ => {}
            }
        }
    }

    fn intern_atom(&mut self, name: &str) -> Result<u32, String> {
        let mut b = (name.len() as u16).to_le_bytes().to_vec();
        b.extend_from_slice(&[0, 0]);
        b.extend_from_slice(name.as_bytes());
        self.req(16, 0, &b).map_err(|e| e.to_string())?;
        Ok(u32le(&self.reply()?, 8))
    }

    fn change_property(&mut self, win: u32, prop: u32, ty: u32, format: u8, data: &[u8]) -> std::io::Result<()> {
        let mut b = Vec::new();
        b.extend_from_slice(&win.to_le_bytes());
        b.extend_from_slice(&prop.to_le_bytes());
        b.extend_from_slice(&ty.to_le_bytes());
        b.extend_from_slice(&[format, 0, 0, 0]);
        b.extend_from_slice(&((data.len() / (format as usize / 8)) as u32).to_le_bytes());
        b.extend_from_slice(data);
        self.req(18, 0, &b)
    }

    /// Envia a imagem BGRX (32 bpp) inteira, em faixas que cabem no limite de requisição.
    fn put_image(&mut self, win: u32, gc: u32, depth: u8, w: usize, h: usize, px: &[u8]) -> std::io::Result<()> {
        let row = w * 4;
        let rows_per = ((self.max_req_bytes.saturating_sub(64)) / row).clamp(1, h);
        let mut y = 0;
        while y < h {
            let n = rows_per.min(h - y);
            let mut b = Vec::with_capacity(20 + n * row);
            b.extend_from_slice(&win.to_le_bytes());
            b.extend_from_slice(&gc.to_le_bytes());
            b.extend_from_slice(&(w as u16).to_le_bytes());
            b.extend_from_slice(&(n as u16).to_le_bytes());
            b.extend_from_slice(&0i16.to_le_bytes());
            b.extend_from_slice(&(y as i16).to_le_bytes());
            b.extend_from_slice(&[0, depth, 0, 0]);
            b.extend_from_slice(&px[y * row..(y + n) * row]);
            self.req(72, 2, &b)?; // ZPixmap
            y += n;
        }
        Ok(())
    }
}

struct Setup {
    width: u16,
    height: u16,
    base: u32,
    root: u32,
    black: u32,
    depth: u8,
    min_kc: u8,
    max_kc: u8,
}

impl Setup {
    fn parse(a: &[u8]) -> Result<Setup, String> {
        if a.len() < 40 {
            return Err("resposta de setup curta".into());
        }
        let vendor = u16le(a, 16) as usize;
        let nformats = a[21] as usize;
        let screen = 32 + pad4(vendor) + nformats * 8;
        if a.len() < screen + 40 {
            return Err("resposta de setup sem tela".into());
        }
        let depth = a[screen + 38];
        let ok = (0..nformats).any(|i| {
            let f = 32 + pad4(vendor) + i * 8;
            a[f] == depth && a[f + 1] == 32
        });
        if !ok {
            return Err(format!("o servidor X não oferece 32 bits por pixel na profundidade {depth}"));
        }
        if a[22] != 0 {
            return Err("servidor X big-endian não suportado".into());
        }
        Ok(Setup { width: u16le(a, screen + 20), height: u16le(a, screen + 22), base: u32le(a, 4), root: u32le(a, screen), black: u32le(a, screen + 12), depth, min_kc: a[26], max_kc: a[27] })
    }
}

/// Tamanho da tela do servidor X (para `--screen auto`).
pub fn root_size() -> Result<(u32, u32), String> {
    let (_, setup) = Conn::connect()?;
    Ok((setup.width as u32, setup.height as u32))
}

// ---- teclado ------------------------------------------------------------------------------

fn vk_of(ks: u32) -> Option<u32> {
    Some(match ks {
        0x61..=0x7a => ks - 0x20,
        0x41..=0x5a | 0x30..=0x39 => ks,
        0x20 => 32,
        0xff08 => 8,
        0xff09 => 9,
        0xff0d | 0xff8d => 13,
        0xff1b => 27,
        0xff13 => 19,
        0xffe1 | 0xffe2 => 16,
        0xffe3 | 0xffe4 => 17,
        0xffe9 | 0xffea | 0xffe7 | 0xffe8 => 18,
        0xffe5 => 20,
        0xff7f => 144,
        0xff50 => 36,
        0xff51 => 37,
        0xff52 => 38,
        0xff53 => 39,
        0xff54 => 40,
        0xff55 => 33,
        0xff56 => 34,
        0xff57 => 35,
        0xff63 => 45,
        0xffff => 46,
        0xffbe..=0xffc9 => 112 + (ks - 0xffbe),
        0xffb0..=0xffb9 => 96 + (ks - 0xffb0),
        // teclado numérico sem NumLock: setas e navegação
        0xff95 => 36,
        0xff96 => 37,
        0xff97 => 38,
        0xff98 => 39,
        0xff99 => 40,
        0xff9a => 33,
        0xff9b => 34,
        0xff9c => 35,
        0xff9d => 12,
        0xff9e => 45,
        0xff9f => 46,
        0xffaa => 106,
        0xffab => 107,
        0xffad => 109,
        0xffae => 110,
        0xffaf => 111,
        0x3b | 0x3a => 186,
        0x3d | 0x2b => 187,
        0x2c | 0x3c => 188,
        0x2d | 0x5f => 189,
        0x2e | 0x3e => 190,
        0x2f | 0x3f => 191,
        0x60 | 0x7e => 192,
        0x5b | 0x7b => 219,
        0x5c | 0x7c => 220,
        0x5d | 0x7d => 221,
        0x27 | 0x22 => 222,
        _ => return None,
    })
}

/// Código de varredura de uma tecla do teclado numérico (não estendida). As setas normais entram como as do
/// keypad porque o Civilization só move unidades com as não estendidas.
fn keypad_scan(ks: u32) -> Option<u32> {
    Some(match ks {
        0xff51 => 0x4B,
        0xff52 => 0x48,
        0xff53 => 0x4D,
        0xff54 => 0x50,
        0xff95 | 0xffb7 => 0x47,
        0xff96 | 0xffb4 => 0x4B,
        0xff97 | 0xffb8 => 0x48,
        0xff98 | 0xffb6 => 0x4D,
        0xff99 | 0xffb2 => 0x50,
        0xff9a | 0xffb9 => 0x49,
        0xff9b | 0xffb3 => 0x51,
        0xff9c | 0xffb1 => 0x4F,
        0xff9d | 0xffb5 => 0x4C,
        0xff9e | 0xffb0 => 0x52,
        0xff9f | 0xffac | 0xffae => 0x53,
        0xffaa => 0x37,
        0xffab => 0x4E,
        0xffad => 0x4A,
        _ => return None,
    })
}

/// Caractere digitado (para `WM_CHAR`) a partir do keysym já com Shift/CapsLock aplicados.
fn char_of(ks: u32) -> Option<u32> {
    match ks {
        0x20..=0x7e | 0xa0..=0xff => Some(ks),
        0xff0d | 0xff8d => Some(13),
        0xff08 => Some(8),
        0xff09 => Some(9),
        0xff1b => Some(27),
        0xffb0..=0xffb9 => Some(0x30 + (ks - 0xffb0)),
        _ => None,
    }
}

struct Keymap {
    per: usize,
    min: u8,
    syms: Vec<u32>,
}

impl Keymap {
    fn sym(&self, kc: u8, col: usize) -> u32 {
        if kc < self.min || col >= self.per {
            return 0;
        }
        self.syms.get((kc - self.min) as usize * self.per + col).copied().unwrap_or(0)
    }
}

// ---- janela e laço de eventos ---------------------------------------------------------------

struct Ev {
    code: u8,
    detail: u8,
    time: u32,
    x: i16,
    y: i16,
    state: u16,
}

pub fn start() -> Result<(), String> {
    let (mut c, setup) = Conn::connect()?;
    let (w, h) = (user32::desktop_w(), user32::desktop_h());
    let wm_protocols = c.intern_atom("WM_PROTOCOLS")?;
    let wm_delete = c.intern_atom("WM_DELETE_WINDOW")?;
    let (win, gc) = (setup.base | 1, setup.base | 2);

    // CreateWindow: pai = raiz, profundidade/visual copiados do pai; fundo preto; eventos de entrada.
    let mut b = Vec::new();
    b.extend_from_slice(&win.to_le_bytes());
    b.extend_from_slice(&setup.root.to_le_bytes());
    // centralizada na tela (o gerenciador de janelas respeita a posição pedida com USPosition)
    let cx = ((setup.width as i32 - w as i32) / 2).max(0) as i16;
    let cy = ((setup.height as i32 - h as i32) / 2).max(0) as i16;
    for v in [cx, cy] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(&(w as u16).to_le_bytes());
    b.extend_from_slice(&(h as u16).to_le_bytes());
    b.extend_from_slice(&0u16.to_le_bytes()); // borda
    b.extend_from_slice(&1u16.to_le_bytes()); // InputOutput
    b.extend_from_slice(&0u32.to_le_bytes()); // visual: CopyFromParent
    b.extend_from_slice(&(0x2u32 | 0x800).to_le_bytes()); // BackPixel | EventMask
    b.extend_from_slice(&setup.black.to_le_bytes());
    // KeyPress|KeyRelease|ButtonPress|ButtonRelease|PointerMotion|Exposure|StructureNotify
    b.extend_from_slice(&(1u32 | 2 | 4 | 8 | 64 | 0x8000 | 0x20000).to_le_bytes());
    c.req(1, 0, &b).map_err(|e| e.to_string())?;
    // CreateGC sem atributos.
    let mut g = Vec::new();
    g.extend_from_slice(&gc.to_le_bytes());
    g.extend_from_slice(&win.to_le_bytes());
    g.extend_from_slice(&0u32.to_le_bytes());
    c.req(55, 0, &g).map_err(|e| e.to_string())?;

    // Título, tamanho fixo (WM_NORMAL_HINTS: PMinSize|PMaxSize) e aviso de fechamento.
    c.change_property(win, 39, 31, 8, b"winlayer").map_err(|e| e.to_string())?;
    let mut hints = [0u32; 18];
    hints[0] = 1 | 16; // USPosition | PMinSize (sem tamanho máximo: a janela pode ser redimensionada e maximizada)
    hints[1] = cx as u32;
    hints[2] = cy as u32;
    hints[5] = 640;
    hints[6] = 480;
    let hb: Vec<u8> = hints.iter().flat_map(|v| v.to_le_bytes()).collect();
    c.change_property(win, 40, 41, 32, &hb).map_err(|e| e.to_string())?;
    c.change_property(win, wm_protocols, 4, 32, &wm_delete.to_le_bytes()).map_err(|e| e.to_string())?;

    // Mapa de teclado (keycode -> keysym).
    let count = setup.max_kc - setup.min_kc + 1;
    c.req(101, 0, &[setup.min_kc, count, 0, 0]).map_err(|e| e.to_string())?;
    let r = c.reply()?;
    let per = r[1] as usize;
    let syms: Vec<u32> = r[32..].chunks_exact(4).map(|q| u32le(q, 0)).collect();
    let keymap = Keymap { per: per.max(1), min: setup.min_kc, syms };

    c.req(8, 0, &win.to_le_bytes()).map_err(|e| e.to_string())?; // MapWindow
    let depth = setup.depth;
    c.s.set_read_timeout(Some(Duration::from_millis(8))).ok();

    eprintln!("[winlayer] janela X11 {w}x{h} aberta (binário de {} UTC)", build_stamp());
    std::thread::spawn(move || {
        if let Err(e) = run(c, keymap, win, gc, depth, wm_delete, w, h) {
            eprintln!("[winlayer] X11: {e}");
        }
        std::process::exit(0); // janela fechada ou conexão perdida
    });
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run(mut c: Conn, keymap: Keymap, win: u32, gc: u32, depth: u8, wm_delete: u32, w: usize, h: usize) -> Result<(), String> {
    let mut buf: Vec<u8> = Vec::new();
    let (mut cur_w, mut cur_h) = (w, h); // tamanho atual da janela X
    let mut last_version = u64::MAX;
    let mut dirty = true;
    let mut last_draw = std::time::Instant::now();
    let mut tmp = [0u8; 8192];
    // diagnóstico: WINLAYER_X11_WINRESIZE=1 redimensiona a janela real via ConfigureWindow e conta eventos/quadros
    // o valor é o atraso em segundos antes do primeiro redimensionamento
    let wr_delay: u64 = std::env::var("WINLAYER_X11_WINRESIZE").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    let winresize = std::env::var_os("WINLAYER_X11_WINRESIZE").is_some();
    let stats = std::env::var_os("WINLAYER_X11_STATS").is_some();
    let wr_start = Instant::now();
    let (mut wr_i, mut wr_t, mut cfg_events, mut frames, mut stat_t) = (0u32, Instant::now(), 0u32, 0u32, Instant::now());
    let stress = std::env::var_os("WINLAYER_X11_STRESS").is_some(); // teste: muda o tamanho da tela o tempo todo
    let (mut stress_i, mut stress_t) = (0usize, Instant::now());
    loop {
        if stress && stress_t.elapsed() >= Duration::from_millis(60) {
            stress_i += 1;
            stress_t = Instant::now();
            user32::request_screen(640 + (stress_i * 97) % 700, 480 + (stress_i * 61) % 400);
        }
        if winresize && wr_start.elapsed() >= Duration::from_secs(wr_delay) && wr_t.elapsed() >= Duration::from_millis(2500) && wr_i < 4 {
            wr_i += 1;
            wr_t = Instant::now();
            let (nw, nh) = [(900u32, 600u32), (1300, 700), (800, 560), (1100, 650)][(wr_i as usize - 1) % 4];
            let mut b = Vec::new();
            b.extend_from_slice(&win.to_le_bytes());
            b.extend_from_slice(&(0x04u16 | 0x08).to_le_bytes());
            b.extend_from_slice(&[0, 0]);
            b.extend_from_slice(&nw.to_le_bytes());
            b.extend_from_slice(&nh.to_le_bytes());
            let _ = c.req(12, 0, &b);
            eprintln!("[x11] pedindo janela {nw}x{nh}");
        }
        if (winresize || stats) && stat_t.elapsed() >= Duration::from_secs(1) {
            // com WINLAYER_X11_STATS só aparece quando houve atividade (evento ou quadro desenhado)
            if winresize || cfg_events > 0 || frames > 0 {
                eprintln!("[x11] último segundo: {cfg_events} ConfigureNotify, {frames} quadros, janela {cur_w}x{cur_h}, tela {}x{}", user32::desktop_w(), user32::desktop_h());
            }
            (cfg_events, frames, stat_t) = (0, 0, Instant::now());
        }
        // o tamanho da tela virtual muda quando a janela é redimensionada (ver ConfigureNotify)
        let (w, h) = (user32::desktop_w(), user32::desktop_h());
        match c.s.read(&mut tmp) {
            Ok(0) => return Ok(()),
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(e) => return Err(e.to_string()),
        }
        let mut evs = Vec::new();
        while buf.len() >= 32 {
            let code = buf[0] & 0x7F;
            let ev = &buf[..32];
            match code {
                0 => eprintln!("[winlayer] X11: erro código {} opcode {}", ev[1], ev[10]),
                2..=6 => evs.push(Ev { code, detail: ev[1], time: u32le(ev, 4), x: u16le(ev, 24) as i16, y: u16le(ev, 26) as i16, state: u16le(ev, 28) }),
                12 => dirty = true, // Expose
                22 => {
                    // ConfigureNotify: novo tamanho da janela
                    let (nw, nh) = (u16le(ev, 20) as usize, u16le(ev, 22) as usize);
                    cfg_events += 1;
                    if nw > 0 && nh > 0 && (nw, nh) != (cur_w, cur_h) {
                        (cur_w, cur_h) = (nw, nh);
                        // o jogo ganha espaço de verdade: a tela virtual passa a ter o tamanho da janela
                        user32::request_screen(nw.max(640), nh.max(480));
                        dirty = true;
                    }
                }
                33 if u32le(ev, 12) == wm_delete => return Ok(()), // ClientMessage: fechar
                _ => {}
            }
            let extra = if code == 1 { u32le(ev, 4) as usize * 4 } else { 0 };
            buf.drain(..32 + extra.min(buf.len().saturating_sub(32)));
        }
        let mut lines: Vec<String> = Vec::new();
        // janela X -> coordenadas do desktop (inverso da escala com barras pretas)
        let (dw, dh, offx, offy) = fit(w, h, cur_w, cur_h);
        let to_desk = |x: i16, y: i16| -> (i16, i16) {
            let dx = ((x as i64 - offx as i64) * w as i64 / dw.max(1) as i64).clamp(0, w as i64 - 1);
            let dy = ((y as i64 - offy as i64) * h as i64 / dh.max(1) as i64).clamp(0, h as i64 - 1);
            (dx as i16, dy as i16)
        };
        let mut push_move = |lines: &mut Vec<String>, x: i16, y: i16| {
            let (x, y) = to_desk(x, y);
            let l = format!("m {} {}", x.max(0), y.max(0));
            if lines.last().is_some_and(|p| p.starts_with("m ")) {
                lines.pop();
            }
            lines.push(l);
        };
        let mut i = 0;
        while i < evs.len() {
            let e = &evs[i];
            match e.code {
                6 => push_move(&mut lines, e.x, e.y),
                4 | 5 if (1..=3).contains(&e.detail) => {
                    push_move(&mut lines, e.x, e.y);
                    lines.push(format!("{} {}", if e.code == 4 { "d" } else { "u" }, e.detail - 1));
                }
                2 if keymap.sym(e.detail, 0) == 0xffc9 => save_print(), // F12: captura de tela
                3 if keymap.sym(e.detail, 0) == 0xffc9 => {}
                2 => {
                    let shift = e.state & 1 != 0;
                    let caps = e.state & 2 != 0;
                    let ctrl_alt = e.state & (4 | 8) != 0;
                    let mut base = keymap.sym(e.detail, 0);
                    // keypad com NumLock ligado (Mod2): vale o nível dos números, não o da navegação
                    if (0xff95..=0xff9f).contains(&base) && e.state & 0x10 != 0 {
                        base = keymap.sym(e.detail, 1);
                    }
                    if let Some(vk) = vk_of(base) {
                        match keypad_scan(base) {
                            Some(sc) => lines.push(format!("kd {vk} {sc} 0")),
                            None => lines.push(format!("kd {vk}")),
                        }
                    }
                    if !ctrl_alt {
                        let letter = (0x61..=0x7a).contains(&base);
                        let col = if shift != (caps && letter) { 1 } else { 0 };
                        let mut ks = keymap.sym(e.detail, col);
                        if ks == 0 {
                            ks = base;
                        }
                        if let Some(ch) = char_of(ks) {
                            lines.push(format!("ch {ch}"));
                        }
                    }
                }
                3 => {
                    // Repetição automática do X = Release+Press com o mesmo tempo: ignora o Release.
                    let repeat = evs.get(i + 1).is_some_and(|n| n.code == 2 && n.detail == e.detail && n.time == e.time);
                    if !repeat {
                        let mut base = keymap.sym(e.detail, 0);
                        if (0xff95..=0xff9f).contains(&base) && e.state & 0x10 != 0 {
                            base = keymap.sym(e.detail, 1);
                        }
                        if let Some(vk) = vk_of(base) {
                            match keypad_scan(base) {
                                Some(sc) => lines.push(format!("ku {vk} {sc} 0")),
                                None => lines.push(format!("ku {vk}")),
                            }
                        }
                    }
                }
                _ => {}
            }
            i += 1;
        }
        if !lines.is_empty() {
            user32::push_events(&lines.join(";"));
        }
        let v = user32::frame_version();
        if (v != last_version || dirty) && last_draw.elapsed() >= Duration::from_millis(16) {
            let (w, h, px) = user32::render_bgrx(); // tamanho que realmente foi desenhado
            if (cur_w, cur_h) == (w, h) {
                c.put_image(win, gc, depth, w, h, &px).map_err(|e| e.to_string())?;
            } else {
                let scaled = scale_to(&px, w, h, cur_w, cur_h);
                c.put_image(win, gc, depth, cur_w, cur_h, &scaled).map_err(|e| e.to_string())?;
            }
            last_version = v;
            frames += 1;
            dirty = false;
            last_draw = std::time::Instant::now();
        }
    }
}

/// "AAAAMMDD-HHMMSS" (UTC) de um instante em segundos desde 1970.
fn utc_stamp(secs: u64) -> String {
    let (days, rem) = ((secs / 86400) as i64, secs % 86400);
    // dias desde 1970 -> data civil (algoritmo de Howard Hinnant)
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + (m <= 2) as i64;
    format!("{y:04}{m:02}{d:02}-{:02}{:02}{:02}", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Data (UTC) do arquivo do próprio executável: mostra no log qual versão está rodando.
pub fn build_stamp() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| std::fs::metadata(p).ok())
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or_else(|| "?".into(), |d| utc_stamp(d.as_secs()))
}

/// F12: salva o desktop do programa em `out/prints/print-AAAAMMDD-HHMMSS.png` (hora UTC).
fn save_print() {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let _ = std::fs::create_dir_all("out/prints");
    let path = format!("out/prints/print-{}.png", utc_stamp(secs));
    user32::dump_screenshot(&path);
}

/// Tamanho da imagem do jogo dentro de uma janela `cw`x`ch`, mantendo a proporção: (largura, altura, x, y).
fn fit(w: usize, h: usize, cw: usize, ch: usize) -> (usize, usize, usize, usize) {
    if (cw, ch) == (w, h) || w == 0 || h == 0 {
        return (w, h, 0, 0);
    }
    // escala = min(cw/w, ch/h), em aritmética inteira
    let (dw, dh) = if cw * h <= ch * w { (cw, (h * cw / w).max(1)) } else { ((w * ch / h).max(1), ch) };
    (dw, dh, (cw - dw) / 2, (ch - dh) / 2)
}

/// Pesos de amostragem por coluna/linha destino: (índice origem 0, índice origem 1, peso do 1 em 0..=256).
/// "Bilinear nítida": ao ampliar, a mistura entre dois pixels vizinhos acontece só numa faixa de ~1 pixel
/// de destino, então as bordas (e a fonte bitmap) continuam firmes e sem larguras desiguais;
/// ao reduzir, é bilinear comum. Ampliação por múltiplo inteiro dá exatamente o vizinho mais próximo.
fn axis_weights(src: usize, dst: usize) -> Vec<(usize, usize, u32)> {
    let s = dst as f32 / src as f32;
    (0..dst)
        .map(|d| {
            let f = (d as f32 + 0.5) / s - 0.5;
            let i0 = f.floor();
            let t = f - i0;
            let t = if s > 1.0 { ((t - 0.5) * s + 0.5).clamp(0.0, 1.0) } else { t };
            let last = src as i64 - 1;
            let a = (i0 as i64).clamp(0, last) as usize;
            let b = (i0 as i64 + 1).clamp(0, last) as usize;
            (a, b, (t * 256.0).round() as u32)
        })
        .collect()
}

/// Escala o desktop (BGRX `w`x`h`) para uma janela `cw`x`ch` mantendo a proporção, com barras pretas.
fn scale_to(px: &[u8], w: usize, h: usize, cw: usize, ch: usize) -> Vec<u8> {
    let (dw, dh, ox, oy) = fit(w, h, cw, ch);
    let (xs, ys) = (axis_weights(w, dw), axis_weights(h, dh));
    let mut out = vec![0u8; cw * ch * 4];
    let mut row = vec![0u32; dw * 4];
    for (y, &(y0, y1, wy)) in ys.iter().enumerate() {
        let (r0, r1) = (&px[y0 * w * 4..(y0 + 1) * w * 4], &px[y1 * w * 4..(y1 + 1) * w * 4]);
        // mistura vertical de uma coluna de origem por vez, depois horizontal
        for (x, &(x0, x1, wx)) in xs.iter().enumerate() {
            for c in 0..4 {
                let top = r0[x0 * 4 + c] as u32 * (256 - wx) + r0[x1 * 4 + c] as u32 * wx;
                let bot = r1[x0 * 4 + c] as u32 * (256 - wx) + r1[x1 * 4 + c] as u32 * wx;
                row[x * 4 + c] = (top * (256 - wy) + bot * wy + 32768) >> 16;
            }
        }
        let dst = &mut out[((oy + y) * cw + ox) * 4..((oy + y) * cw + ox + dw) * 4];
        for (d, v) in dst.iter_mut().zip(row.iter()) {
            *d = *v as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_mantem_proporcao_com_barras() {
        assert_eq!(fit(1024, 616, 1024, 616), (1024, 616, 0, 0));
        // janela mais larga: barras laterais
        assert_eq!(fit(1000, 500, 2000, 800), (1600, 800, 200, 0));
        // janela mais alta: barras em cima e embaixo
        assert_eq!(fit(1000, 500, 1000, 800), (1000, 500, 0, 150));
        // dobro exato
        assert_eq!(fit(640, 480, 1280, 960), (1280, 960, 0, 0));
    }

    #[test]
    fn escala_preenche_e_centraliza() {
        // imagem 2x2 (BGRX) numa janela 4x4: cada pixel vira 2x2
        let px: Vec<u8> = (0..4u8).flat_map(|i| [i + 1, 0, 0, 0]).collect();
        let out = scale_to(&px, 2, 2, 4, 4);
        let at = |x: usize, y: usize| out[(y * 4 + x) * 4];
        assert_eq!((at(0, 0), at(1, 1), at(2, 0), at(0, 2), at(3, 3)), (1, 1, 2, 3, 4));
        // janela larga 6x2: imagem 2x2 ocupa 2x2 no centro (barras de 2 px)
        let out = scale_to(&px, 2, 2, 6, 2);
        let at = |x: usize, y: usize| out[(y * 6 + x) * 4];
        assert_eq!((at(0, 0), at(2, 0), at(3, 1), at(5, 1)), (0, 1, 4, 0));
    }

    #[test]
    fn ampliacao_fracionaria_mantem_bordas_firmes() {
        // listras de 1 pixel (o pior caso, como traços de fonte) ampliadas 1,37x: os extremos
        // (preto e branco) têm que continuar existindo, sem virar um cinza uniforme
        let (w, h) = (20usize, 2usize);
        let px: Vec<u8> = (0..h).flat_map(|_| (0..w).flat_map(|x| { let v = if x % 2 == 0 { 255 } else { 0 }; [v, v, v, 0] })).collect();
        let (cw, ch) = (27usize, 3usize);
        let out = scale_to(&px, w, h, cw, ch);
        let (dw, _, ox, _) = fit(w, h, cw, ch);
        let row: Vec<u8> = (0..dw).map(|x| out[(ox + x) * 4]).collect();
        let (hi, lo) = (row.iter().filter(|&&v| v > 200).count(), row.iter().filter(|&&v| v < 55).count());
        assert!(hi >= row.len() / 4 && lo >= row.len() / 4, "contraste perdido: {hi} claros, {lo} escuros de {}", row.len());
    }
}
