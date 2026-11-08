//! Visualizador: servidor HTTP mínimo (só std) que mostra o "desktop" num <canvas> e recebe
//! mouse/teclado do navegador. Sem bibliotecas gráficas: qualquer navegador serve de tela.
//!
//! Rotas: `GET /` (página), `GET /frame?since=N` (long-poll; corpo = RGBA 640x480, cabeçalho
//! X-Version), `POST /event` (linhas do protocolo de entrada; ver `win32/user32/input.rs`).

use crate::audio;
use crate::win32::user32;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

const PAGE: &str = include_str!("viewer.html");

pub fn start(bind: &str, port: u16) -> std::io::Result<()> {
    let listener = TcpListener::bind((bind, port))?;
    eprintln!("[winlayer] visualizador em http://{bind}:{port}/");
    std::thread::spawn(move || {
        for conn in listener.incoming().flatten() {
            std::thread::spawn(move || {
                let _ = handle(conn);
            });
        }
    });
    Ok(())
}

fn respond(s: &mut TcpStream, status: &str, ctype: &str, extra: &str, body: &[u8]) -> std::io::Result<()> {
    write!(
        s,
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n{extra}\r\n",
        body.len()
    )?;
    s.write_all(body)
}

fn handle(mut s: TcpStream) -> std::io::Result<()> {
    s.set_read_timeout(Some(Duration::from_secs(5)))?;
    // Lê até o fim dos cabeçalhos.
    let mut buf = Vec::new();
    let mut tmp = [0u8; 2048];
    let head_end = loop {
        let n = s.read(&mut tmp)?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break p + 4;
        }
        if buf.len() > 64 * 1024 {
            return Ok(());
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.lines();
    let request = lines.next().unwrap_or("");
    let mut parts = request.split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or("/"));
    let length: usize = lines
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.trim().parse().ok())
        .unwrap_or(0);
    let (path, query) = target.split_once('?').unwrap_or((target, ""));

    match (method, path) {
        ("GET", "/") => respond(&mut s, "200 OK", "text/html; charset=utf-8", "", PAGE.as_bytes()),
        ("GET", "/frame") => {
            let since: u64 = query.split('&').find_map(|kv| kv.strip_prefix("since=")).and_then(|v| v.parse().ok()).unwrap_or(u64::MAX);
            let deadline = Instant::now() + Duration::from_millis(1000);
            while user32::frame_version() == since && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(4));
            }
            let v = user32::frame_version();
            if v == since {
                return respond(&mut s, "204 No Content", "text/plain", "", b"");
            }
            let (w, h, rgba) = user32::render_rgba();
            let extra = format!("X-Version: {v}\r\nX-Width: {w}\r\nX-Height: {h}\r\n");
            respond(&mut s, "200 OK", "application/octet-stream", &extra, &rgba)
        }
        ("GET", "/sound") => {
            let id: usize = query.split('&').find_map(|kv| kv.strip_prefix("id=")).and_then(|v| v.parse().ok()).unwrap_or(0);
            match audio::wav_bytes(id) {
                Some(b) => respond(&mut s, "200 OK", "audio/wav", "", &b),
                None => respond(&mut s, "404 Not Found", "text/plain", "", b"sem som"),
            }
        }
        ("GET", "/audio") => {
            let since: u64 = query.split('&').find_map(|kv| kv.strip_prefix("since=")).and_then(|v| v.parse().ok()).unwrap_or(0);
            let deadline = Instant::now() + Duration::from_millis(1000);
            while audio::seq() <= since && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            let body = audio::events_since(since);
            respond(&mut s, "200 OK", "text/plain", &format!("X-Seq: {}\r\n", audio::seq()), body.as_bytes())
        }
        ("POST", "/event") => {
            let mut body = buf[head_end..].to_vec();
            while body.len() < length {
                let n = s.read(&mut tmp)?;
                if n == 0 {
                    break;
                }
                body.extend_from_slice(&tmp[..n]);
            }
            user32::push_events(&String::from_utf8_lossy(&body));
            respond(&mut s, "204 No Content", "text/plain", "", b"")
        }
        _ => respond(&mut s, "404 Not Found", "text/plain", "", b"nao encontrado"),
    }
}
