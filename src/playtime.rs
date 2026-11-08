//! Usage time per game, in an append-only log: `~/.local/share/winlayer/playtime.log`.
//!
//! Each line is `start<TAB>pid<TAB>seconds<TAB>path of the .exe`. A session writes one line on opening, another
//! every minute and another on exit; the one with the longest time counts. If the process dies, at most the last minute is lost.
//! The time is that of the program being open (pauses are not detected) and does not count while the computer is suspended.
//! `winlayer --playtime` shows the total.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const BEAT: Duration = Duration::from_secs(60);

struct Session {
    file: PathBuf,
    game: String,
    start: u64,
    clock: Instant,
}

static SESSION: OnceLock<Session> = OnceLock::new();

extern "C" {
    fn atexit(f: extern "C" fn()) -> i32;
}

/// `$XDG_DATA_HOME/winlayer` or `~/.local/share/winlayer`.
fn log_path() -> Option<PathBuf> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(data.join("winlayer/playtime.log"))
}

fn write_beat() {
    let Some(s) = SESSION.get() else { return };
    let line = format!("{}\t{}\t{}\t{}\n", s.start, std::process::id(), s.clock.elapsed().as_secs(), s.game);
    let written = std::fs::OpenOptions::new().create(true).append(true).open(&s.file).and_then(|mut f| f.write_all(line.as_bytes()));
    if let Err(e) = written {
        eprintln!("[winlayer] play time: could not write to {}: {e}", s.file.display());
    }
}

extern "C" fn at_exit() {
    write_beat();
}

/// Starts counting the game's time in `game` (absolute path of the .exe).
pub fn start(game: &str) {
    let Some(file) = log_path() else { return };
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let start = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let _ = SESSION.set(Session { file, game: game.to_string(), start, clock: Instant::now() });
    write_beat();
    // covers ExitProcess, closed window and the fault handler, which leave through `process::exit`
    unsafe { atexit(at_exit) };
    let _ = std::thread::Builder::new().spawn(|| loop {
        std::thread::sleep(BEAT);
        write_beat();
    });
}

#[derive(Debug, PartialEq)]
struct Total {
    game: String,
    seconds: u64,
    sessions: u32,
}

/// Sums the log: per session the longest time counts; invalid lines are ignored. Longest time first.
fn totals(text: &str) -> Vec<Total> {
    let mut sessions: BTreeMap<(&str, &str, &str), u64> = BTreeMap::new();
    for line in text.lines() {
        let mut f = line.splitn(4, '\t');
        let (Some(start), Some(pid), Some(secs), Some(game)) = (f.next(), f.next(), f.next(), f.next()) else { continue };
        let Ok(secs) = secs.parse::<u64>() else { continue };
        let best = sessions.entry((game, start, pid)).or_insert(0);
        *best = secs.max(*best);
    }
    let mut games: BTreeMap<&str, (u64, u32)> = BTreeMap::new();
    for ((game, _, _), secs) in sessions {
        let t = games.entry(game).or_insert((0, 0));
        t.0 += secs;
        t.1 += 1;
    }
    let mut out: Vec<Total> = games.into_iter().map(|(g, (seconds, sessions))| Total { game: g.to_string(), seconds, sessions }).collect();
    out.sort_by(|a, b| b.seconds.cmp(&a.seconds));
    out
}

/// "folder/GAME.EXE": the last two components of the path.
fn short_name(path: &str) -> String {
    let parts: Vec<&str> = path.rsplit('/').take(2).collect();
    parts.into_iter().rev().collect::<Vec<_>>().join("/")
}

/// `winlayer --playtime`
pub fn report() {
    let Some(file) = log_path() else {
        eprintln!("winlayer: no HOME, cannot tell where the play time log is");
        return;
    };
    let text = String::from_utf8_lossy(&std::fs::read(&file).unwrap_or_default()).into_owned();
    let all = totals(&text);
    if all.is_empty() {
        println!("No play time recorded ({}).", file.display());
        return;
    }
    println!("{:>8}  {:>8}  Game", "Minutes", "Sessions");
    for t in &all {
        println!("{:>8}  {:>8}  {}", t.seconds / 60, t.sessions, short_name(&t.game));
    }
    println!("\nLog: {}", file.display());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sums_longest_time_of_each_session() {
        let log = "100\t1\t0\t/j/a.exe\n100\t1\t60\t/j/a.exe\n100\t1\t75\t/j/a.exe\n200\t2\t0\t/j/b.exe\n200\t2\t600\t/j/b.exe\n300\t3\t60\t/j/a.exe\n";
        assert_eq!(
            totals(log),
            vec![
                Total { game: "/j/b.exe".into(), seconds: 600, sessions: 1 },
                Total { game: "/j/a.exe".into(), seconds: 135, sessions: 2 },
            ]
        );
    }

    #[test]
    fn ignores_line_cut_by_crash() {
        assert_eq!(totals("100\t1\t60\t/j/a.exe\n100\t1\t1"), vec![Total { game: "/j/a.exe".into(), seconds: 60, sessions: 1 }]);
    }

    #[test]
    fn short_name_keeps_last_two_components() {
        assert_eq!(short_name("/mnt/Jogos/Hexplore/hexplore.exe"), "Hexplore/hexplore.exe");
    }
}
