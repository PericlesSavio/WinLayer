//! Diagnostics for finding what a program needs that winlayer lacks.
//!
//! - `--check program.exe`: static analysis, nothing runs. Lists the imports of the program and of the DLLs
//!   it brings, telling which ones winlayer implements, and the DLL names it may load at run time.
//! - `--report file.md`: runs the program (missing functions become traps) and, however it ends, writes a
//!   report: why it ended, missing functions (and which were called), problems seen during the run (files
//!   not found, `GetProcAddress` failures, unsupported features, message boxes, exceptions, crash) and the
//!   last API calls.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use crate::pe::{ImportName, Pe32};

// ---- events -------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Kind {
    /// Import that winlayer does not implement (found when loading).
    MissingImport,
    /// The program called one of them: it stops there.
    MissingCalled,
    /// `GetProcAddress` did not find the function (the program may go on without it, or not).
    ProcNotFound,
    /// `LoadLibrary` did not find or could not load the DLL.
    DllNotFound,
    /// A file the program tried to open does not exist or could not be opened.
    FileNotFound,
    /// A feature winlayer does not support (or only partly).
    Unsupported,
    /// Text shown in a message box: programs report their errors this way.
    MessageBox,
    /// Exception raised by the program (`RaiseException`, C++ `throw`).
    Exception,
    /// The program crashed.
    Crash,
}

impl Kind {
    fn title(self) -> &'static str {
        match self {
            Kind::MissingImport => "Missing imports",
            Kind::MissingCalled => "Missing function called",
            Kind::ProcNotFound => "GetProcAddress failed",
            Kind::DllNotFound => "LoadLibrary failed",
            Kind::FileNotFound => "Files that could not be opened",
            Kind::Unsupported => "Unsupported features",
            Kind::MessageBox => "Message boxes",
            Kind::Exception => "Exceptions",
            Kind::Crash => "Crash",
        }
    }
}

struct State {
    path: PathBuf,
    program: String,
    start: Instant,
    /// (kind, text) -> count, in the order first seen.
    events: Vec<(Kind, String, u32)>,
    end: Option<String>,
}

static STATE: OnceLock<Mutex<State>> = OnceLock::new();
static WRITTEN: AtomicBool = AtomicBool::new(false);
const MAX_EVENTS: usize = 2000;

/// True with `--report`: the loader then counts API calls and keeps the last ones.
pub fn enabled() -> bool {
    STATE.get().is_some()
}

extern "C" {
    fn atexit(f: extern "C" fn()) -> i32;
    fn signal(sig: i32, handler: extern "C" fn(i32)) -> usize;
}

/// SIGTERM (`timeout`, `kill`) and SIGINT (Ctrl+C): the report is still written.
extern "C" fn on_terminate(sig: i32) {
    set_end(format!("terminated by signal {sig} (still running: hung, or just not finished)"));
    std::process::exit(128 + sig);
}

extern "C" fn at_exit() {
    write_report();
}

/// Starts collecting; the report is written when the process ends, whichever way.
pub fn start(path: PathBuf, program: &str) {
    let state = State { path, program: program.to_string(), start: Instant::now(), events: Vec::new(), end: None };
    if STATE.set(Mutex::new(state)).is_ok() {
        unsafe {
            atexit(at_exit);
            signal(15, on_terminate);
            signal(2, on_terminate);
        }
    }
}

/// Records an event (repeated ones are counted, not repeated).
pub fn note(kind: Kind, text: impl Into<String>) {
    let Some(s) = STATE.get() else { return };
    let text = text.into();
    let Ok(mut s) = s.try_lock() else { return }; // never block (e.g. inside the crash handler)
    if let Some(e) = s.events.iter_mut().find(|e| e.0 == kind && e.1 == text) {
        e.2 += 1;
    } else if s.events.len() < MAX_EVENTS {
        s.events.push((kind, text, 1));
    }
}

/// Likely return addresses into the program's code found on the current stack, innermost first:
/// where a message box or exception came from (to look up in a disassembly).
pub fn callers() -> String {
    let base = crate::win32::kernel32::image_base();
    if base == 0 {
        return String::new();
    }
    let size = unsafe {
        let pe = base + *((base + 0x3C) as usize as *const u32);
        *((pe + 0x50) as usize as *const u32)
    };
    let esp: u32;
    unsafe { core::arch::asm!("mov {0}, esp", out(reg) esp) };
    // read page by page: a read that crosses the end of the stack fails whole
    let mut words = Vec::new();
    for page in 0..8u32 {
        let chunk = crate::loader::peek(esp + page * 0x800, 0x800);
        let full = chunk.len() == 0x800;
        words.extend(chunk);
        if !full {
            break;
        }
    }
    let found: Vec<String> = words
        .chunks_exact(4)
        .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
        .filter(|&v| v > base + 0x1000 && v < base + size && after_call(v))
        .take(12)
        .map(|v| format!("{v:#x}"))
        .collect();
    if found.is_empty() { String::new() } else { format!(" [stack: {}]", found.join(" ")) }
}

/// True when the bytes before `ret` are a CALL (E8 rel32, FF /2 with the usual operand forms):
/// `ret` is then a return address, not data that happens to point into the code.
fn after_call(ret: u32) -> bool {
    let b = crate::loader::peek(ret - 7, 7);
    if b.len() < 7 {
        return false;
    }
    let modrm_call = |m: u8| (m >> 3) & 7 == 2;
    b[2] == 0xE8 // call rel32
        || (b[1] == 0xFF && modrm_call(b[2]) && b[2] & 0xC7 == 0x05) // call [disp32]
        || (b[5] == 0xFF && modrm_call(b[6]) && b[6] >> 6 == 3) // call reg
        || (b[4] == 0xFF && modrm_call(b[5]) && b[5] >> 6 == 1) // call [reg+disp8]
        || (b[1] == 0xFF && modrm_call(b[2]) && b[2] >> 6 == 2) // call [reg+disp32]
        || (b[5] == 0xFF && modrm_call(b[6]) && b[6] >> 6 == 0) // call [reg]
}

/// Prints a "[winlayer] ..." warning and records it as an unsupported feature.
pub fn warn(text: String) {
    eprintln!("[winlayer] {text}");
    note(Kind::Unsupported, text);
}

/// Why the program ended (the first reason given wins: the most specific one is set first).
pub fn set_end(reason: impl Into<String>) {
    let Some(s) = STATE.get() else { return };
    if let Ok(mut s) = s.try_lock() {
        if s.end.is_none() {
            s.end = Some(reason.into());
        }
    }
}

// ---- API calls ------------------------------------------------------------------------------

const MAX_FUNCS: usize = 16384;
const RING: usize = 64;

static NAMES: Mutex<Vec<String>> = Mutex::new(Vec::new());
static COUNTS: [AtomicU32; MAX_FUNCS] = [const { AtomicU32::new(0) }; MAX_FUNCS];
/// Last calls: function index + 1 (0 = empty) and the first 4 stack arguments.
static RING_CALLS: [[AtomicU32; 5]; RING] = [const { [const { AtomicU32::new(0) }; 5] }; RING];
static RING_POS: AtomicU32 = AtomicU32::new(0);

/// Registers an import wrapped by the loader; returns its index for `on_call`.
pub fn register(label: &str) -> u32 {
    let mut n = NAMES.lock().unwrap();
    n.push(label.to_string());
    (n.len() - 1) as u32
}

pub fn name(idx: u32) -> String {
    NAMES.lock().unwrap().get(idx as usize).cloned().unwrap_or_default()
}

/// Called by the loader's thunk before each wrapped API function. Lock-free: threads may call it at once.
pub extern "C" fn on_call(idx: u32, args: *const u32) {
    if let Some(c) = COUNTS.get(idx as usize) {
        c.fetch_add(1, Ordering::Relaxed);
    }
    let slot = &RING_CALLS[RING_POS.fetch_add(1, Ordering::Relaxed) as usize % RING];
    slot[0].store(idx + 1, Ordering::Relaxed);
    for i in 0..4 {
        slot[i + 1].store(unsafe { *args.add(i) }, Ordering::Relaxed);
    }
}

// ---- report ---------------------------------------------------------------------------------

fn write_report() {
    let Some(s) = STATE.get() else { return };
    if WRITTEN.swap(true, Ordering::SeqCst) {
        return;
    }
    let s = match s.try_lock() {
        Ok(s) => s,
        Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
        Err(_) => return,
    };
    let text = render(&s);
    match std::fs::write(&s.path, text) {
        Ok(()) => eprintln!("[winlayer] report written to {}", s.path.display()),
        Err(e) => eprintln!("[winlayer] could not write the report to {}: {e}", s.path.display()),
    }
}

fn render(s: &State) -> String {
    let mut o = String::new();
    let _ = writeln!(o, "# winlayer report\n");
    let _ = writeln!(o, "- **Program:** `{}`", s.program);
    let _ = writeln!(o, "- **winlayer:** {}", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(o, "- **Run time:** {:.1} s", s.start.elapsed().as_secs_f32());
    let _ = writeln!(o, "- **End:** {}", s.end.as_deref().unwrap_or("normal exit"));

    let missing: Vec<&(Kind, String, u32)> = s.events.iter().filter(|e| e.0 == Kind::MissingImport).collect();
    let called: Vec<&str> = s.events.iter().filter(|e| e.0 == Kind::MissingCalled).map(|e| e.1.as_str()).collect();
    let _ = writeln!(o, "\n## Missing imports ({})\n", missing.len());
    if missing.is_empty() {
        let _ = writeln!(o, "None.");
    }
    for (_, name, _) in &missing {
        let mark = if called.contains(&name.as_str()) { " — **called**" } else { "" };
        let _ = writeln!(o, "- `{name}`{mark}");
    }

    for kind in [
        Kind::Crash, Kind::MissingCalled, Kind::Exception, Kind::MessageBox, Kind::ProcNotFound, Kind::DllNotFound,
        Kind::FileNotFound, Kind::Unsupported,
    ] {
        let list: Vec<&(Kind, String, u32)> = s.events.iter().filter(|e| e.0 == kind).collect();
        if list.is_empty() {
            continue;
        }
        let _ = writeln!(o, "\n## {} ({})\n", kind.title(), list.len());
        for (_, text, n) in list {
            let times = if *n > 1 { format!(" (×{n})") } else { String::new() };
            if text.contains('\n') {
                let _ = writeln!(o, "```\n{text}\n```{times}");
            } else {
                let _ = writeln!(o, "- `{text}`{times}");
            }
        }
    }

    let names = match NAMES.try_lock() {
        Ok(n) => n.clone(),
        Err(_) => Vec::new(),
    };
    let mut top: Vec<(u32, &str)> = names
        .iter()
        .enumerate()
        .map(|(i, n)| (COUNTS[i.min(MAX_FUNCS - 1)].load(Ordering::Relaxed), n.as_str()))
        .filter(|c| c.0 > 0)
        .collect();
    top.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(b.1)));
    let _ = writeln!(o, "\n## API functions used ({} of {} imported)\n", top.len(), names.len());
    if !top.is_empty() {
        let _ = writeln!(o, "| Calls | Function |\n|---:|---|");
        for (n, name) in &top {
            let _ = writeln!(o, "| {n} | `{name}` |");
        }
    }

    let pos = RING_POS.load(Ordering::Relaxed) as usize;
    let _ = writeln!(o, "\n## Last API calls (oldest first)\n");
    let _ = writeln!(o, "Arguments are the first 4 values on the stack; text is shown when one points to a string.\n");
    let _ = writeln!(o, "```");
    for k in 0..RING {
        let slot = &RING_CALLS[(pos + k) % RING];
        let idx = slot[0].load(Ordering::Relaxed);
        if idx == 0 {
            continue;
        }
        let name = names.get(idx as usize - 1).map_or("?", |n| n.as_str());
        let args: Vec<String> = (1..5)
            .map(|i| {
                let a = slot[i].load(Ordering::Relaxed);
                match crate::loader::peek_text(a) {
                    Some(t) => format!("{a:#x} \"{t}\""),
                    None => format!("{a:#x}"),
                }
            })
            .collect();
        let _ = writeln!(o, "{name}({})", args.join(", "));
    }
    let _ = writeln!(o, "```");
    o
}

// ---- --check: static analysis ---------------------------------------------------------------

struct DllReport {
    /// "built-in", "file" (a DLL found next to the program) or "not found".
    source: &'static str,
    implemented: usize,
    missing: Vec<String>,
}

fn find_dll(dir: &Path, name: &str) -> Option<PathBuf> {
    let want = name.to_ascii_lowercase();
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| {
        p.file_name().is_some_and(|f| f.to_string_lossy().to_ascii_lowercase() == want)
    })
}

fn norm_dll(name: &str) -> String {
    let n = name.to_ascii_lowercase();
    if n.contains('.') { n } else { format!("{n}.dll") }
}

/// Names of DLLs that appear as text in the file: candidates for `LoadLibrary` at run time.
fn dll_strings(data: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < data.len() {
        let start = i;
        while i < data.len() && (data[i].is_ascii_alphanumeric() || b"_-.".contains(&data[i])) {
            i += 1;
        }
        let word = &data[start..i];
        if word.len() > 4 && word.len() < 40 && word.to_ascii_lowercase().ends_with(b".dll") && word[0].is_ascii_alphabetic() {
            let w = String::from_utf8_lossy(word).to_ascii_lowercase();
            if !out.contains(&w) {
                out.push(w);
            }
        }
        i += 1;
    }
    out
}

/// Prints the static report for `exe`. Returns the number of missing imports.
pub fn check(exe: &Path) -> usize {
    let dir = exe.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut dlls: BTreeMap<String, DllReport> = BTreeMap::new();
    let mut queue: Vec<PathBuf> = vec![exe.to_path_buf()];
    let mut seen_files: Vec<PathBuf> = Vec::new();
    let mut referenced: Vec<String> = Vec::new();
    let mut graphics: Vec<&str> = Vec::new();

    while let Some(path) = queue.pop() {
        if seen_files.contains(&path) {
            continue;
        }
        seen_files.push(path.clone());
        let Ok(data) = std::fs::read(&path) else { continue };
        let Ok(pe) = Pe32::parse(&data) else {
            println!("{}: not a 32-bit Windows program (PE)", path.display());
            continue;
        };
        for w in dll_strings(&data) {
            if !referenced.contains(&w) {
                // a DLL next to the program, loaded at run time: its imports count too
                if let Some(p) = find_dll(&dir, &w) {
                    queue.push(p);
                }
                referenced.push(w);
            }
        }
        for imp in &pe.imports {
            let dll = norm_dll(&imp.dll);
            let builtin = crate::win32::modules::is_builtin(&dll);
            let entry = dlls.entry(dll.clone()).or_insert_with(|| DllReport {
                source: if builtin { "built-in" } else if find_dll(&dir, &dll).is_some() { "file" } else { "not found" },
                implemented: 0,
                missing: Vec::new(),
            });
            if !builtin {
                match find_dll(&dir, &dll) {
                    Some(p) => queue.push(p),
                    // nobody provides this DLL: all its functions are missing
                    None => {
                        for name in &imp.names {
                            let label = match name {
                                ImportName::Name(n) => n.clone(),
                                ImportName::Ordinal(o) => format!("#{o}"),
                            };
                            if !entry.missing.contains(&label) {
                                entry.missing.push(label);
                            }
                        }
                    }
                }
                continue;
            }
            for name in &imp.names {
                let label = match name {
                    ImportName::Name(n) => n.clone(),
                    ImportName::Ordinal(o) => match crate::win32::ordinal_name(&dll, *o as u32) {
                        Some(n) => n.to_string(),
                        None => format!("#{o}"),
                    },
                };
                let ok = match name {
                    ImportName::Name(n) => crate::win32::resolve(&dll, n).is_some(),
                    ImportName::Ordinal(o) => crate::win32::ordinal_name(&dll, *o as u32).and_then(|n| crate::win32::resolve(&dll, n)).is_some(),
                };
                if ok {
                    entry.implemented += 1;
                } else if !entry.missing.contains(&label) {
                    entry.missing.push(label);
                }
            }
            if dll.starts_with("d3d") || dll == "opengl32.dll" || dll == "glide2x.dll" || dll == "glide3x.dll" {
                if !graphics.contains(&"3D") {
                    graphics.push("3D");
                }
            }
        }
    }

    let total_missing: usize = dlls.values().map(|d| d.missing.len()).sum();
    println!("# winlayer check: {}\n", exe.display());
    println!("| DLL | Source | Implemented | Missing |\n|---|---|---:|---:|");
    for (name, d) in &dlls {
        println!("| {name} | {} | {} | {} |", d.source, d.implemented, d.missing.len());
    }
    if total_missing > 0 {
        println!("\n## Missing functions ({total_missing})\n");
        for (name, d) in dlls.iter().filter(|d| !d.1.missing.is_empty()) {
            println!("- **{name}:** {}", d.missing.iter().map(|m| format!("`{m}`")).collect::<Vec<_>>().join(", "));
        }
    }
    let unknown: Vec<&String> = dlls.iter().filter(|d| d.1.source == "not found").map(|d| d.0).collect();
    if !unknown.is_empty() {
        println!("\n## DLLs that are neither built in nor next to the program\n");
        for d in unknown {
            println!("- `{d}`");
        }
    }
    let dynamic: Vec<&String> = referenced.iter().filter(|r| !dlls.contains_key(*r)).collect();
    if !dynamic.is_empty() {
        println!("\n## Other DLL names in the files (possible LoadLibrary)\n");
        for d in dynamic {
            let src = if crate::win32::modules::is_builtin(d) {
                "built-in"
            } else if find_dll(&dir, d).is_some() {
                "file"
            } else {
                "not found"
            };
            println!("- `{d}` ({src})");
        }
    }
    if !graphics.is_empty() {
        println!("\n**Note:** imports Direct3D/OpenGL/Glide; winlayer's 3D objects do not draw.");
    }
    total_missing
}
