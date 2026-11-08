//! Loaded modules: system DLLs implemented by winlayer ("builtin") and PE DLLs from disk.

use crate::loader;
use crate::pe::{ImportName, Pe32, IMAGE_FILE_DLL};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

/// DLLs whose functions are implemented in Rust (modules of `win32/`).
/// `_inmm.dll` is GOG's "winmm" (emulates CD audio with OGG): it is treated as the internal winmm, without loading the DLL.
const BUILTIN: [&str; 16] = ["msvcrt.dll", "imm32.dll", "wsock32.dll", "dinput.dll", "shell32.dll", "_inmm.dll", "kernel32.dll", "user32.dll", "gdi32.dll", "advapi32.dll", "ole32.dll", "oleaut32.dll", "comctl32.dll", "winmm.dll", "ddraw.dll", "dsound.dll"];
/// Fictitious HMODULEs of the builtins: BUILTIN_BASE + index * 0x10000.
const BUILTIN_BASE: u32 = 0x7800_0000;

struct Module {
    name: String,
    /// Path as the program sees it (`C:\...`; the .exe directory is `C:\`).
    win_path: String,
    base: u32,
    exports: BTreeMap<String, u32>,
    by_ordinal: BTreeMap<u32, u32>,
    /// Range of the exports directory (to detect forwarders).
    export_range: (u32, u32),
}

static MODULES: Mutex<Vec<Module>> = Mutex::new(Vec::new());
static SEARCH: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

pub fn add_search_dir(dir: PathBuf) {
    SEARCH.lock().unwrap().push(dir);
}

/// "C:\WINDOWS\SYSTEM\Vb40032.dll" -> "vb40032.dll"
fn norm(name: &str) -> String {
    let file = name.rsplit(['\\', '/']).next().unwrap_or(name).to_ascii_lowercase();
    if file.contains('.') { file } else { format!("{file}.dll") }
}

fn builtin_index(n: &str) -> Option<usize> {
    BUILTIN.iter().position(|b| *b == n)
}

/// HMODULE of an already loaded module (or of a builtin).
pub fn module_handle(name: &str) -> Option<u32> {
    let n = norm(name);
    if let Some(i) = builtin_index(&n) {
        return Some(BUILTIN_BASE + i as u32 * 0x10000);
    }
    MODULES.lock().unwrap().iter().find(|m| m.name == n).map(|m| m.base)
}

fn find_file(name: &str) -> Option<PathBuf> {
    let n = norm(name);
    let mut dirs = SEARCH.lock().unwrap().clone();
    if let Some(root) = super::files::root() {
        dirs.push(root.clone());
        dirs.push(root.join("system"));
    }
    dirs.into_iter().find_map(|d| {
        std::fs::read_dir(d).ok()?.flatten().find(|e| e.file_name().to_string_lossy().to_ascii_lowercase() == n).map(|e| e.path())
    })
}

unsafe fn read_exports(pe: &Pe32, base: u32) -> Module {
    let mut m = Module {
        name: String::new(), win_path: String::new(), base, exports: BTreeMap::new(), by_ordinal: BTreeMap::new(),
        export_range: (pe.export_dir.0, pe.export_dir.0 + pe.export_dir.1),
    };
    if pe.export_dir.0 == 0 {
        return m;
    }
    let d = base + pe.export_dir.0;
    let r32 = |a: u32| *(a as usize as *const u32);
    let r16 = |a: u32| *(a as usize as *const u16) as u32;
    let (ord_base, nfuncs, nnames) = (r32(d + 16), r32(d + 20), r32(d + 24));
    let (funcs, names, ords) = (base + r32(d + 28), base + r32(d + 32), base + r32(d + 36));
    for i in 0..nfuncs {
        m.by_ordinal.insert(ord_base + i, r32(funcs + i * 4));
    }
    for i in 0..nnames {
        let name_ptr = base + r32(names + i * 4);
        let name = std::ffi::CStr::from_ptr(name_ptr as usize as *const i8).to_string_lossy().into_owned();
        let idx = r16(ords + i * 2);
        m.exports.insert(name, r32(funcs + idx * 4));
    }
    m
}

/// Windows-style path of a host file: relative to the .exe directory (which is `C:\`).
fn win_path_of(host: &std::path::Path) -> String {
    let rel = super::files::root().and_then(|r| host.strip_prefix(r).ok().map(|p| p.to_path_buf()));
    let rel = rel.unwrap_or_else(|| host.file_name().map(PathBuf::from).unwrap_or_default());
    format!("C:\\{}", rel.to_string_lossy().replace('/', "\\"))
}

/// File name (Windows-style) of the module loaded at `base`; `None` if it is not a known module.
pub fn module_file_name(base: u32) -> Option<String> {
    if base >= BUILTIN_BASE && base < BUILTIN_BASE + BUILTIN.len() as u32 * 0x10000 {
        let dll = BUILTIN[((base - BUILTIN_BASE) / 0x10000) as usize];
        return Some(format!("C:\\WINDOWS\\SYSTEM\\{}", dll.to_ascii_uppercase()));
    }
    MODULES.lock().unwrap().iter().find(|m| m.base == base).map(|m| m.win_path.clone())
}

/// Loads a DLL (or returns the already loaded one). Builtins return a fictitious HMODULE.
pub fn load_library(name: &str) -> Option<u32> {
    if let Some(h) = module_handle(name) {
        return Some(h);
    }
    let n = norm(name);
    let path = find_file(name)?;
    let file = std::fs::read(&path).ok()?;
    let pe = Pe32::parse(&file).ok()?;
    if pe.characteristics & IMAGE_FILE_DLL == 0 {
        return None;
    }
    let base = loader::map(&pe, &file).map_err(|e| eprintln!("[winlayer] {n}: {e}")).ok()?;
    let mut module = unsafe { read_exports(&pe, base) };
    module.name = n.clone();
    module.win_path = win_path_of(&path);
    // Registers before resolving imports (circular dependencies).
    MODULES.lock().unwrap().push(module);

    let missing = loader::fill_imports(&pe, base);
    if !missing.is_empty() {
        eprintln!("[winlayer] {n}: {} imports not implemented", missing.len());
    }
    if pe.entry_rva != 0 {
        // DllMain(hinst, DLL_PROCESS_ATTACH, NULL)
        unsafe { super::call3(base + pe.entry_rva, base, 1, 0) };
    }
    Some(base)
}

pub enum Proc<'a> {
    Name(&'a str),
    Ordinal(u32),
}

/// Address of a function exported by a module.
pub fn get_proc(module: u32, proc_: Proc) -> Option<u32> {
    if module >= BUILTIN_BASE && module < BUILTIN_BASE + BUILTIN.len() as u32 * 0x10000 {
        let dll = BUILTIN[((module - BUILTIN_BASE) / 0x10000) as usize];
        return match proc_ {
            Proc::Name(n) => super::resolve(dll, n).map(|a| a as u32),
            Proc::Ordinal(o) => super::ordinal_name(dll, o).and_then(|n| super::resolve(dll, n)).map(|a| a as u32),
        };
    }
    let (rva, range, name) = {
        let ms = MODULES.lock().unwrap();
        let m = ms.iter().find(|m| m.base == module)?;
        let rva = match proc_ {
            Proc::Name(n) => *m.exports.get(n)?,
            Proc::Ordinal(o) => *m.by_ordinal.get(&o)?,
        };
        (rva, m.export_range, m.name.clone())
    };
    if rva >= range.0 && rva < range.1 {
        // Forwarder "OTHER.Function": resolves in the indicated module.
        let s = unsafe { std::ffi::CStr::from_ptr((module + rva) as usize as *const i8) }.to_string_lossy().into_owned();
        let (dll, f) = s.split_once('.')?;
        eprintln!("[winlayer] forwarder {name} -> {s}");
        return resolve_import(&format!("{dll}.dll"), &ImportName::Name(f.to_string()));
    }
    Some(module + rva)
}

/// Resolves a `dll!name` import: winlayer builtin or PE DLL loaded from disk.
pub fn resolve_import(dll: &str, name: &ImportName) -> Option<u32> {
    let n = norm(dll);
    let proc_ = match name {
        ImportName::Name(s) => Proc::Name(s),
        ImportName::Ordinal(o) => Proc::Ordinal(*o as u32),
    };
    if builtin_index(&n).is_some() {
        return get_proc(module_handle(&n)?, proc_);
    }
    let h = load_library(&n)?;
    get_proc(h, proc_)
}
