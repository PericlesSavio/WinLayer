//! Windows API implementation. Each module corresponds to a DLL.

/// Builds `resolve`: function name -> implementation address.
macro_rules! exports {
    ($name:expr, $($f:ident),* $(,)?) => {
        match $name {
            $(stringify!($f) => Some($f as *const () as usize),)*
            _ => None,
        }
    };
}

pub mod advapi32;
pub mod d3d;
pub mod ddraw;
pub mod dinput;
pub mod dplay;
pub mod dsound;
pub mod draw;
pub mod files;
pub mod fnt;
pub mod gdi32;
pub mod kernel32;
pub mod listbox;
pub mod kernel32x;
pub mod kernel32y;
pub mod menu;
pub mod modules;
pub mod msvcrt;
pub mod ole32;
pub mod oleaut32;
pub mod stubs;
pub mod resources;
pub mod sync;
pub mod teb;
pub mod text;
pub mod user32;
pub mod winmm;
pub mod wsock32;

/// Calls a stdcall function of the program with exactly 1 argument (thread start).
pub unsafe fn call1(f: u32, a: u32) -> u32 {
    let f: extern "stdcall" fn(u32) -> u32 = std::mem::transmute(f as usize);
    f(a)
}

/// ... with 3 arguments (DllMain).
pub unsafe fn call3(f: u32, a: u32, b: u32, c: u32) -> u32 {
    let f: extern "stdcall" fn(u32, u32, u32) -> u32 = std::mem::transmute(f as usize);
    f(a, b, c)
}

/// ... with 5 arguments (waveOut and timeSetEvent callbacks).
pub unsafe fn call5(f: u32, a: u32, b: u32, c: u32, d: u32, e: u32) -> u32 {
    let f: extern "stdcall" fn(u32, u32, u32, u32, u32) -> u32 = std::mem::transmute(f as usize);
    f(a, b, c, d, e)
}

/// ... with 4 arguments (WndProc, DlgProc).
pub unsafe fn call4(f: u32, a: u32, b: u32, c: u32, d: u32) -> u32 {
    let f: extern "stdcall" fn(u32, u32, u32, u32) -> u32 = std::mem::transmute(f as usize);
    f(a, b, c, d)
}

/// Name of the function exported by ordinal (builtin DLLs imported by number).
pub fn ordinal_name(dll: &str, ordinal: u32) -> Option<&'static str> {
    match dll {
        "oleaut32.dll" => oleaut32::name_of(ordinal),
        "wsock32.dll" => wsock32::name_of(ordinal),
        "dsound.dll" if ordinal == 1 => Some("DirectSoundCreate"),
        _ => None,
    }
}

/// Looks up the implementation address of `dll!name`. DLL names are
/// case-insensitive on Windows.
pub fn resolve(dll: &str, name: &str) -> Option<usize> {
    match dll.to_ascii_lowercase().as_str() {
        "kernel32.dll" => kernel32::resolve(name)
            .or_else(|| sync::resolve(name))
            .or_else(|| files::resolve(name))
            .or_else(|| kernel32x::resolve(name))
            .or_else(|| kernel32y::resolve(name)),
        "user32.dll" => user32::resolve(name).or_else(|| stubs::resolve_user32(name)),
        "gdi32.dll" => gdi32::resolve(name).or_else(|| stubs::resolve_gdi32(name)),
        "advapi32.dll" => advapi32::resolve(name).or_else(|| stubs::resolve_advapi32(name)),
        "ole32.dll" => ole32::resolve(name),
        "comctl32.dll" => stubs::resolve_comctl32(name),
        "shell32.dll" => stubs::resolve_shell32(name),
        "imm32.dll" => stubs::resolve_imm32(name),
        "msvcrt.dll" => msvcrt::resolve(name),
        "wsock32.dll" => wsock32::resolve(name),
        "winmm.dll" | "_inmm.dll" => winmm::resolve(name),
        "ddraw.dll" => ddraw::resolve(name),
        "dinput.dll" => dinput::resolve(name),
        "dsound.dll" => dsound::resolve(name),
        "oleaut32.dll" => oleaut32::resolve(name),
        _ => None,
    }
}
