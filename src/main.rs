mod audio;
mod crash;
mod dbgalloc;
mod loader;
mod png;
mod pulse;
mod viewer;
mod win16;
mod pe;
mod playtime;
mod win32;
mod x11;

use std::process::ExitCode;

#[global_allocator]
static ALLOC: dbgalloc::Dbg = dbgalloc::Dbg;

fn main() -> ExitCode {
    if std::env::var_os("WINLAYER_BUMP").is_some() {
        dbgalloc::enable_bump();
        if std::env::var_os("WINLAYER_GUARD").is_some() {
            dbgalloc::enable_guard();
        }
    }
    if std::env::var_os("WINLAYER_HEAPCHECK").is_some() {
        dbgalloc::enable();
    }
    if std::env::args().nth(1).as_deref() == Some("--playtime") {
        playtime::report();
        return ExitCode::SUCCESS;
    }
    if std::env::args().nth(1).as_deref() == Some("--win16-selftest") {
        match win16::selftest() {
            Ok(()) => println!("win16 selftest: OK (32<->16 transitions work)"),
            Err(e) => {
                eprintln!("win16 selftest: FAILED: {e}");
                return ExitCode::from(1);
            }
        }
        return ExitCode::SUCCESS;
    }
    // usage: winlayer [--x11 | --shot file.png | --serve PORT [--bind ADDR]] [--events SCRIPT] [--screen WxH] [--dll-path DIR]... [--lazy] program.exe
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut serve_port: Option<u16> = None;
    let mut bind = String::from("127.0.0.1");
    let mut want_x11 = false;
    // options (before the .exe): --shot file.png, --dll-path DIR (repeatable), --lazy
    while args.first().is_some_and(|a| a.starts_with("--")) {
        match args[0].as_str() {
            "--screen" if args.len() >= 2 => {
                // --screen 1280x1024
                if args[1] == "auto" {
                    // size of the X screen, leaving room for borders, title bar and the window manager panel
                    match x11::root_size() {
                        Ok((w, h)) => {
                            // fits the screen, but at most 1024x768 (the game was made for small screens)
                            let fit = |v: u32, margin: usize, min: usize, max: usize| (v as usize).saturating_sub(margin).clamp(min.min(v as usize), max);
                            win32::user32::set_screen(fit(w, 16, 640, 1024), fit(h, 104, 480, 768));
                        }
                        Err(e) => {
                            eprintln!("winlayer: --screen auto: {e}");
                            return ExitCode::from(1);
                        }
                    }
                } else if let Some((w, h)) = args[1].split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?))) {
                    win32::user32::set_screen(w, h);
                } else {
                    eprintln!("winlayer: --screen expects WIDTHxHEIGHT (e.g. 1280x1024)");
                    return ExitCode::from(2);
                }
                args.drain(..2);
            }
            // background color of the emulated desktop: RRGGBB in hexadecimal (e.g. 000000)
            "--desktop" if args.len() >= 2 => {
                let Some(rgb) = u32::from_str_radix(args[1].trim_start_matches('#'), 16).ok().filter(|v| *v <= 0xFF_FFFF) else {
                    eprintln!("winlayer: --desktop expects an RRGGBB color (e.g. 000000)");
                    return ExitCode::from(2);
                };
                // COLORREF is 0x00BBGGRR
                let c = ((rgb & 0xFF) << 16) | (rgb & 0xFF00) | (rgb >> 16);
                win32::gdi32::DESKTOP_COLOR.store(c, std::sync::atomic::Ordering::Relaxed);
                args.drain(..2);
            }
            "--lazy" => {
                loader::set_lazy(true);
                args.remove(0);
            }
            "--dll-path" if args.len() >= 2 => {
                let dir = std::fs::canonicalize(&args[1]).unwrap_or_else(|_| args[1].clone().into());
                win32::modules::add_search_dir(dir);
                args.drain(..2);
            }
            "--shot" if args.len() >= 2 => {
                let shot = std::env::current_dir().unwrap_or_default().join(&args[1]);
                std::env::set_var("WINLAYER_SHOT", shot);
                win32::user32::set_shot_mode(true);
                args.drain(..2);
            }
            // input script for tests: "m 60 120; d 0; u 0; wait 3; kd 65; ch 97"
            "--events" if args.len() >= 2 => {
                win32::user32::push_events(&args[1]);
                args.drain(..2);
            }
            "--serve" if args.len() >= 2 => {
                serve_port = args[1].parse().ok();
                args.drain(..2);
            }
            "--fullscreen" => {
                x11::set_fullscreen();
                want_x11 = true;
                args.remove(0);
            }
            "--x11" => {
                want_x11 = true;
                args.remove(0);
            }
            "--bind" if args.len() >= 2 => {
                bind = args[1].clone();
                args.drain(..2);
            }
            other => {
                eprintln!("winlayer: unknown option: {other}");
                return ExitCode::from(2);
            }
        }
    }
    let Some(path) = args.first().map(|p| std::fs::canonicalize(p).map_or(p.clone(), |c| c.to_string_lossy().into_owned())) else {
        eprintln!("usage: winlayer [--shot output.png] [--screen WxH] [--dll-path DIR]... [--lazy] <program.exe> [args...]");
        return ExitCode::from(2);
    };

    if let Some(port) = serve_port {
        if let Err(e) = viewer::start(&bind, port) {
            eprintln!("winlayer: could not open the viewer on {bind}:{port}: {e}");
            return ExitCode::from(1);
        }
    }
    // Without --shot/--serve, the default is the native X11 window.
    if want_x11 || (serve_port.is_none() && !win32::user32::shot_mode()) {
        match x11::start() {
            Ok(()) => {
                // without a browser, the sound goes straight out through PulseAudio/PipeWire
                if let Err(e) = pulse::start() {
                    eprintln!("winlayer: no native audio: {e}");
                }
            }
            Err(e) => {
                eprintln!("winlayer: X11 window unavailable: {e}");
                if want_x11 {
                    return ExitCode::from(1);
                }
            }
        }
    }
    crash::install();
    win32::kernel32::init(&path, &args[1..]);

    let data = match std::fs::read(&path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("winlayer: could not read {path}: {e}");
            return ExitCode::from(1);
        }
    };

    // Capture-mode tests are not play time.
    if !win32::user32::shot_mode() {
        playtime::start(&path);
    }

    // Win16 (NE) executable? Goes on to the Win16 layer.
    if let Some(lfa) = data.get(0x3C..0x40).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize) {
        if data.get(lfa..lfa + 2) == Some(b"NE") {
            return match win16::run(&path, &data, &args[1..]) {
                Ok(code) => ExitCode::from(code as u8),
                Err(e) => {
                    eprintln!("winlayer: {path}: {e}");
                    ExitCode::from(1)
                }
            };
        }
    }

    let image = match pe::Pe32::parse(&data) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("winlayer: {path}: {e}");
            return ExitCode::from(1);
        }
    };

    match loader::load(&image, &data) {
        // SAFETY: the entry point is the .exe's own code, mapped by `load`.
        Ok(loaded) => ExitCode::from(unsafe { loaded.run() } as u8),
        Err(e) => {
            eprintln!("winlayer: {path}: {e}");
            ExitCode::from(1)
        }
    }
}
