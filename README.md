# winlayer

A compatibility layer that runs old Windows games on Linux, written in Rust. It is not an emulator and needs no copy of
Windows: game code runs directly on the CPU, and calls to the Windows API are handled by winlayer.

It ships as a single statically linked 32-bit executable of about 2.4 MB, with no libraries to install.

**Version 0.0.1**, early development. It covers what the tested games use, not Windows as a whole. See
[docs/COMPATIBILITY.md](docs/COMPATIBILITY.md) for per-game results.

## Features

- Win32 (PE32) and Win16 (NE, Windows 3.x) executables
- Native X11 window, spoken directly over the socket without Xlib
- Native PulseAudio/PipeWire audio, with its own mixer
- Software `user32` and `gdi32`: windows, menus, dialogs, controls, bitmap fonts and logical palettes
- DirectDraw (DirectX 5 and 6), including 8-bit palettes and Smacker video through the game's own DLL
- DirectSound and DirectInput
- A browser viewer (`--serve`) and a headless PNG capture mode (`--shot`) with scripted input
- Per-game play time log

Not implemented: OpenGL, Direct3D rendering (the objects exist, but draw nothing), real networking and DOS programs.

## Building

Requires Rust with the `i586-unknown-linux-musl` target:

```sh
rustup target add i586-unknown-linux-musl
cargo build --release
```

The binary is written to `target/i586-unknown-linux-musl/release/winlayer`.

## Requirements

**Only tested on Fedora 43 (x86-64).** Other distributions should work, but have not been tried.

- Linux on x86 or x86-64, with a kernel that can run 32-bit programs
- An X server or XWayland; the X authorization cookie is read from `~/.Xauthority`
- PulseAudio or PipeWire for sound (optional)

Win16 programs also need 16-bit segments through `modify_ldt`; `winlayer --win16-selftest` checks for them.

## Usage

Run it with the game's executable; the folder of the `.exe` is seen by the program as `C:\`:

```sh
winlayer --x11 /path/to/game/GAME.EXE
```

| Option | Description |
|---|---|
| `--x11` | Native window (the default without `--shot` or `--serve`) |
| `--serve PORT [--bind ADDR]` | Viewer at `http://127.0.0.1:PORT/`; only listens on 127.0.0.1 by default |
| `--shot FILE.png` | Headless mode with a virtual clock; saves a PNG and exits |
| `--events SCRIPT` | Scripted input, e.g. `"m 120 140; d 0; wait 1; u 0; kd 13; ku 13"` |
| `--screen WxH` or `auto` | Size of the virtual desktop (default 640x480) |
| `--desktop RRGGBB` | Desktop background color |
| `--dll-path DIR` | Extra folder to search for DLLs (repeatable) |
| `--lazy` | Turn missing imports into traps instead of refusing to start |
| `--playtime` | Show minutes and sessions per game, then exit |
| `--win16-selftest` | Check that 16-bit code can run on this kernel |

### Play time

Time is stored in `~/.local/share/winlayer/tempo-de-jogo.log`, an append-only text file written when a game opens, every
minute and on exit. After a crash, at most the last minute is lost. It counts the time a game stays open, and capture
mode is not counted.

### Debugging

| Variable | Effect |
|---|---|
| `WINLAYER_API_TRACE=1` | Log every imported API call |
| `WINLAYER_TRACE=1` | Log window messages and missing imports |
| `WINLAYER_WIN16_TRACE=1` | Log every Win16 API call |
| `WINLAYER_WIN16_REPORT=1` | Report what a Win16 program is missing |
| `WINLAYER_GDI_TRACE=1` | Log GDI blits and text |
| `WINLAYER_DDRAW_TRACE=1` | Log DirectDraw calls |
| `WINLAYER_D3D_TRACE=1` | Log Direct3D calls |
| `WINLAYER_DSOUND_TRACE=1` | Log DirectSound calls |
| `WINLAYER_DINPUT_TRACE=1` | Log DirectInput calls |
| `WINLAYER_DPLAY_TRACE=1` | Log DirectPlay calls |
| `WINLAYER_AUDIO_LOG=1` | Log played sounds |
| `WINLAYER_FRAMES=N` | `--shot`: timer cycles to run before capturing |
| `WINLAYER_DDRAW_FRAMES=N` | `--shot`: presented DirectDraw frames before capturing |

## How it works

The executable is mapped into memory and its import table is filled with the addresses of Rust functions, so a call to
`CreateFileA` or `IDirectDrawSurface::Blt` lands in winlayer. Some calls translate directly to Linux, others are
reimplemented from scratch (window manager, GDI, DirectDraw surfaces, audio mixer), and some only return what the program
expects. DLLs shipped with a game, such as Miles or Smacker, are loaded from disk and run unchanged.

| Path | Contents |
|---|---|
| `src/main.rs` | Entry point and command line |
| `src/pe.rs`, `src/loader.rs` | PE parsing, image mapping and import resolution |
| `src/x11.rs`, `src/pulse.rs`, `src/viewer.rs` | Window, audio and browser viewer |
| `src/playtime.rs` | Play time log |
| `src/win32/` | One module per Windows DLL |
| `src/win16/` | Windows 3.x layer: NE loader, 32↔16 transitions and 16-bit APIs |

The target is `i586-unknown-linux-musl` because game code must run in a 32-bit process, and because code built by old
compilers does not keep the 16-byte stack alignment that SSE code in Rust relies on.

## Author

Péricles S.
