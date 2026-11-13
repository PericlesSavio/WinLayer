# Candidate games

Windows games worth testing next with winlayer. None of them has been tried yet: once a game is tested, it moves to
[COMPATIBILITY.md](COMPATIBILITY.md).

## Selection criteria

- **Windows only.** No console ports (the game, not just the series), and no MS-DOS games.
- **No official modern re-release.** Games sold today on Steam or GOG, already adapted to current Windows, are left for
  later (see [Later: re-released games](#later-re-released-games)). The focus is on games with no other way to run.
- **Not a windowed game.** Games that build their interface out of Windows windows, menus and dialogs (like
  Civilization for Windows or 3D Pinball) are left out for now. Candidates draw their whole screen themselves, usually
  full screen with DirectDraw.
- **Fits what winlayer already has:** DirectDraw (8 and 16-bit), DirectSound, DirectInput, Smacker videos, INI files,
  Win32 (no Win16).
- **Avoids what winlayer lacks:** Direct3D drawing (its objects exist but draw nothing), OpenGL, AVI videos through MCI,
  and networking. Games that need a 3D card are out.

## Candidates

All checked in November 2026: no Steam or GOG release, only the original CD.

| Game | Year | Genre | Developer | Why |
|---|---|---|---|---|
| Atomic Bomberman | 1997 | Action | Interplay | Small and simple; tests input and sound. Multiplayer needs DirectPlay |
| Claw | 1997 | Platformer | Monolith | 2D side-scroller, DirectDraw |
| Gruntz | 1999 | Puzzle/strategy | Monolith | Same 2D engine as Claw |
| Get Medieval | 1998 | Action (Gauntlet-like) | Monolith, Microïds | 2D, DirectDraw |
| Dark Colony | 1997 | RTS | Alternative Reality Technologies (SSI) | 2D, DirectDraw. The Mac version is not a console port |
| Beasts and Bumpkins | 1997 | RTS/village building | Worldweaver (EA) | 2D, Windows only |
| Abomination: The Nemesis Project | 1999 | Real-time tactics | Hothouse Creations (Eidos) | Isometric 2D |

## Later: re-released games

Sold today for current Windows. They may be tested in the future, but other games come first.

| Game | Year | Re-release |
|---|---|---|
| Fallout 2 | 1998 | Steam, GOG |
| Arcanum: Of Steamworks and Magick Obscura | 2001 | Steam, GOG |
| Command & Conquer: Tiberian Sun | 1999 | Steam (Command & Conquer Ultimate Collection) |
| Total Annihilation | 1997 | Steam, GOG |
| Dungeon Keeper | 1997 | Steam, GOG |
| Dark Reign: The Future of War | 1997 | GOG |
| Commandos: Behind Enemy Lines | 1998 | Steam, GOG |
| Caesar III | 1998 | Steam, GOG |
| Pharaoh | 1999 | Steam, GOG |
| Sanitarium | 1998 | Steam, GOG |
| Septerra Core | 1999 | Steam, GOG |
| Rage of Mages (Allods) | 1998 | Steam, GOG |
| The Settlers III | 1998 | Ubisoft (History Edition) |
| Earth 2140 | 1997 | Steam, GOG |
| Tzar: The Burden of the Crown | 2000 | Steam, GOG |
| War Wind | 1996 | Steam, GOG (free) |
| Krush Kill 'N Destroy Xtreme | 1997 | Steam, GOG |
| Fragile Allegiance | 1997 | Steam, GOG |

## Left out

Examples of well-known games that do not meet the criteria, so they are not suggested again:

| Game | Reason |
|---|---|
| Diablo | PlayStation port |
| StarCraft | Nintendo 64 port |
| Warcraft II, Command & Conquer: Red Alert, Dune 2000, Theme Hospital | PlayStation ports |
| Baldur's Gate, Icewind Dale | The Enhanced Editions were ported to consoles |
| RollerCoaster Tycoon | Xbox port |
| Heroes of Might and Magic II | Game Boy Color port |
| Worms Armageddon | Console ports |
| Jagged Alliance 2, Majesty, Gorky 17, SimCity 3000 | Native Linux versions |
| Dungeon Keeper 2 | Needs Direct3D |
