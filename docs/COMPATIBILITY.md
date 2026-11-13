# Compatibility

Windows games tested with winlayer 0.0.5. Last updated: 2026-11-12.

## Summary

| Status | Meaning | Games |
|---|---|---:|
| **Doesn't run** | Crashes or exits before showing the game | 1 |
| **Unplayable** | Starts, but something essential prevents playing | 3 |
| **Playable** | Can be played, but features are missing or there are visible defects | 6 |
| **Perfect** | No known defects, after extended testing | 0 |
| **Total** | | **10** |

No game is rated Perfect yet: more testing is needed.

## Games

| Game | Year | Platform | Status | Graphics | Sound | Input |
|---|---|---|---|---|---|---|
| [Revenant](#revenant) (GOG 1.22l) | 1999 | Win32, DirectX 6 | Playable: no CD music, no multiplayer | DirectDraw 640x480x16, game's own 3D renderer | See notes | DirectInput and window messages |
| [Nox](#nox) | 2000 | Win32, DirectX | Playable: no multiplayer | DirectDraw 640x480x16 | DirectSound | DirectInput |
| [Hexplore](#hexplore) | 1998 | Win32, DirectX 5 | Playable | DirectDraw 640x480x8 | DirectSound | Mouse and keyboard |
| [Rising Lands](#rising-lands) (Portuguese) | 1997 | Win32, DirectX | Playable: no AVI videos, no multiplayer | DirectDraw 800x600 | Not verified | Mouse and keyboard |
| [Age of Empires](#age-of-empires) (Gold Edition) | 1997 | Win32, DirectX 5 | Playable: graphical glitches, minimap does not work, no music | DirectDraw 8-bit | DirectSound | Mouse and keyboard |
| [Icarus: Sanctuary of the Gods](#icarus-sanctuary-of-the-gods) | 1998 | Win32, DirectX | Playable: no CD music | DirectDraw 640x480x8; Cinepak AVI videos | DirectSound | Mouse and keyboard |
| [The Reap](#the-reap) | 1997 | Win32, DirectX | Unplayable: freezes when the intro is skipped | DirectDraw 320x200x16 or 640x480x16 | MIDAS (`waveOut`) | Mouse and keyboard |
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

## Age of Empires

- **Command**, from the game folder: `winlayer --cd-label AOE EMPIRES.EXE` (`EMPIRESX.EXE`, the Rise of Rome
  expansion, has not been tested).
- **Installation:** the `GAME` folder of the CD copied as is, without running the installer.
- **CD check:** the game reads the CD path from the registry (`CDPath`, written by the installer), then checks that the
  drive is a CD-ROM labeled `AOE`. `winlayer.reg` in the game folder provides the value:

  ```
  REGEDIT4

  [HKEY_LOCAL_MACHINE\Software\Microsoft\Games\Age of Empires\1.00]
  "CDPath"="D:\\"
  ```

- **Tested:** main menu, campaign screen and the start of a scenario in an X11 window.
- **Fixed for this game:** `VirtualAlloc` returns 64 KB aligned regions (the game's allocator relies on it);
  `version.dll` (DirectX version check); `CreateFileA` with `OPEN_ALWAYS` and read-only access (the `.DRS` data
  files); the `C:\TEMP` folder is created; 17 missing functions; `.reg` files for the registry.
- **Limitations:**
  - Graphical glitches; the minimap does not work.
  - No music: it plays CD audio tracks through MCI, which winlayer does not have.

## Icarus: Sanctuary of the Gods

- **Command**, from the game folder: `winlayer --cd-label ICARUS ICARUS.EXE`
- **Installation:** the InstallShield 3 installer was not run. Its archives were extracted by hand: `EXE.Z` and
  `ROOT.Z` to the game folder, `EDATA.Z` to `EDATA\`, `TDATA.Z` to `TDATA\`, plus `MAP\`, `OPEN.AVI` and `ENDING.AVI`
  from the CD. `DROIYAN.INI` holds the CD drive (`D:\`), which `--cd-label` maps to the game folder.
- **Tested:** logos, menu, intro video, prologue and the start of a game in capture mode; playing in an X11 window.
- **Tips:** clicks skip the logos and the video; Esc skips the prologue.
- **Fixed for this game:** `mmio` file reading (the sounds are WAV files loaded with it; without them New Game
  crashed); AVI files (`avifil32`) and a Cinepak decoder (`ICDecompress`), since the game closes if the intro video
  cannot be opened; `wvsprintfA`.
- **Limitations:** the CD audio music (second CD) does not play.

## The Reap

- **Command**, from the game folder: `winlayer --cd-label THEREAP thereap.exe`
- **CD check:** the game looks for a CD-ROM labeled `THEREAP`. `--cd-label` adds a virtual CD drive `D:` that maps to
  the game folder (`--cd DIR` maps it to another folder).
- **Tested:** video mode dialog, loading and the intro videos with sound, in capture mode and in an X11 window.
- **Fixed for this game:** C runtime functions (files, time, `_CIpow`, `_beginthread`); a Microsoft-layout `FILE`,
  since the game reads the end-of-file flag directly; dialogs in the extended `DLGTEMPLATEEX` format; threads created
  by other threads (the `fs` TLS entry ran out); a virtual CD drive.
- **Limitations:**
  - Unplayable: skipping the intro with the mouse or Space freezes the game, so the menu is never reached.
  - Radio buttons in the video mode dialog are drawn as check boxes.
  - Drawing to a DIB section (`CreateDIBSection`) does not show on screen.

## 3D Pinball: Space Cadet

- **Command**, from the game folder: `winlayer --x11 --screen 1024x768 pinball.exe` (at 640x480 the top of the window
  is cut off).
- **Tested:** table, colors and starting a game in capture mode.
- **Game controls:** F2 starts a game and Space launches the ball.
- **Limitations:** unplayable in a real window, cause not yet identified. Sound not verified. The game creates a
  fixed-size window.

## Rising Lands

- **Command**, from the game folder: `winlayer Rising.EXE`
- **Tested:** menus, loading a saved game and selecting units in capture mode; playing in an X11 window.
- **Fixed for this game:** sections with uninitialized data declared with a file size are no longer filled from the
  file; `GetAsyncKeyState` now processes pending input, so loops that wait for a key to be released no longer freeze;
  INI files are now read (the game's unit rules, names and inventions come from `RISING.INI`), which brought back the
  options of units.
- **Limitations:**
  - AVI videos (`MCIWndCreateA`) are not played and there is no file dialog (`GetOpenFileNameA`).
  - Writing INI files has no effect.
  - Multiplayer does not work: DirectPlay has no service providers.

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
