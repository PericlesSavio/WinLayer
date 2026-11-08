# Compatibility

Windows games tested with winlayer 0.0.1. Last updated: 2026-11-08.

## Summary

| Status | Meaning | Games |
|---|---|---:|
| **Doesn't run** | Crashes or exits before showing the game | 1 |
| **Unplayable** | Starts, but something essential prevents playing | 2 |
| **Playable** | Can be played, but features are missing or there are visible defects | 3 |
| **Perfect** | No known defects, after extended testing | 0 |
| **Total** | | **6** |

No game is rated Perfect yet: more testing is needed.

## Games

| Game | Year | Platform | Status | Graphics | Sound | Input |
|---|---|---|---|---|---|---|
| [Revenant](#revenant) (GOG 1.22l) | 1999 | Win32, DirectX 6 | Playable: no CD music, no multiplayer | DirectDraw 640x480x16, game's own 3D renderer | See notes | DirectInput and window messages |
| [Nox](#nox) | 2000 | Win32, DirectX | Playable: no multiplayer | DirectDraw 640x480x16 | DirectSound | DirectInput |
| [Hexplore](#hexplore) | 1998 | Win32, DirectX 5 | Playable | DirectDraw 640x480x8 | DirectSound | Mouse and keyboard |
| [3D Pinball: Space Cadet](#3d-pinball-space-cadet) | 1995 | Win32, GDI | Unplayable | GDI, 8-bit palette | Not verified | Keyboard |
| [Civilization for Windows](#civilization-for-windows) | 1992 | Win16 (NE) | Unplayable: units do not move | GDI | `sndPlaySound` | Mouse and keyboard |
| [Planescape: Torment Enhanced Edition](#planescape-torment-enhanced-edition) (GOG 3.1.4.0) | 2017 | Win32, OpenGL | Doesn't run: native version available | — | — | — |

## Revenant

- **Command**, from the game folder: `winlayer --x11 Revenant.exe`
- **Required setting:** `Software3D=Yes` in `Revenant.ini`. winlayer's Direct3D objects do not draw; in this mode the
  game rasterizes by itself and only uses them as helpers.
- **Tested:** main menu and starting a game in capture mode; loading and playing in an X11 window.
- **GOG files:** `_inmm.dll` is treated as the built-in `winmm`, so the original DLL, libvorbis and `MSVCR120` are not
  loaded.
- **Limitations:**
  - CD music (OGG tracks in `Music/`) does not play.
  - Multiplayer does not work: DirectPlay only exists so that the game can start.
  - A capture showed a green rectangle on a torch flame; the cause has not been investigated.

## Nox

- **Command**, from the game folder: `winlayer --x11 Game.exe` (`NOX.EXE` has not been tested).
- **Tested:** main menu in capture mode; running in an X11 window. Gameplay has not been verified in capture mode.
- **Tip:** Esc skips the intro.
- **Limitations:** multiplayer does not work (`wsock32` reports the network as down); no Asian text input.

## Hexplore

- **Command**, from the game folder: `winlayer --x11 hexplore.exe`
- **Tested:** logo, Smacker intro videos, menu, new game and map with mouse and keyboard in capture mode; X11 window.
- **Tip:** Esc skips the intro.
- **Limitations:** none known, but only the start of a game has been tested.

## 3D Pinball: Space Cadet

- **Command**, from the game folder: `winlayer --x11 --screen 1024x768 pinball.exe` (at 640x480 the top of the window
  is cut off).
- **Tested:** table, colors and starting a game in capture mode.
- **Game controls:** F2 starts a game and Space launches the ball.
- **Limitations:** unplayable in a real window, cause not yet identified. Sound not verified. The game creates a
  fixed-size window.

## Civilization for Windows

- **Command**, from the game folder: `winlayer --screen auto --desktop 000000 --x11 CIV.EXE`
- **Tested:** intro with music, main menu, map, founding a city, city screen, end of turn and Civilopedia.
- **Limitations:**
  - Many bugs remain in its windows and dialogs.
  - Unplayable: units do not move with the arrow keys in a real window.
  - `GetBitmapBits`/`SetBitmapBits` are missing; colors are hazy on the map screen after moving units.

## Planescape: Torment Enhanced Edition

- **Native Linux version:** the GOG installer includes `Torment64` (64-bit) and `Torment` (32-bit). Prefer them to
  `Torment.exe`. They have not been tested and are installed without the executable bit (`chmod +x Torment64`).
- **In winlayer:** `Torment.exe` stops at the first missing function (`GetStartupInfoW`). 218 functions are missing;
  support will not be implemented while a native version exists.
- **Main blocker:** the game renders with OpenGL, which winlayer does not have. OpenAL, the Unicode `kernel32`/`user32`
  functions and networking (`WS2_32`) are also missing.
