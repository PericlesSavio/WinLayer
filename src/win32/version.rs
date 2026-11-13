//! version.dll — file version information. Programs use it to check the version of system DLLs
//! (DirectX, DirectPlay). winlayer's own DLLs report DirectX 6.1 versions; files on disk report the
//! VS_FIXEDFILEINFO of their version resource.

pub fn resolve(name: &str) -> Option<usize> {
    exports!(name, GetFileVersionInfoSizeA, GetFileVersionInfoA, VerQueryValueA)
}

const SIGNATURE: u32 = 0xFEEF_04BD;
/// DirectX 6.1 file version: 4.06.02.0436.
const DX_VERSION: (u32, u32) = (0x0004_0006, 0x0002_01B4);

/// The 52-byte VS_FIXEDFILEINFO of `file`: from its version resource, or made up for a built-in DLL.
fn fixed_info(file: &str) -> Option<[u8; 52]> {
    let host = super::files::host_path(file);
    let fixed = |ms: u32, ls: u32| {
        let mut f = [0u8; 52];
        let vals = [SIGNATURE, 0x0001_0000, ms, ls, ms, ls, 0x3F, 0, 0x0004_0004, 2, 0, 0, 0];
        for (i, v) in vals.iter().enumerate() {
            f[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        f
    };
    if let Ok(data) = std::fs::read(&host) {
        // the signature starts the structure, on a 4-byte boundary inside the resource
        let at = (0..=data.len().saturating_sub(52)).step_by(4).find(|&i| data[i..i + 4] == SIGNATURE.to_le_bytes())?;
        let mut f = [0u8; 52];
        f.copy_from_slice(&data[at..at + 52]);
        return Some(f);
    }
    let name = file.rsplit(['\\', '/']).next().unwrap_or(file);
    super::modules::is_builtin(name).then(|| fixed(DX_VERSION.0, DX_VERSION.1))
}

/// The block GetFileVersionInfoA returns: a VS_VERSIONINFO header with the fixed information and no strings.
fn version_block(f: &[u8; 52]) -> Vec<u8> {
    let key: Vec<u8> = "VS_VERSION_INFO\0".encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
    let mut b = Vec::new();
    b.extend_from_slice(&0u16.to_le_bytes()); // wLength, set below
    b.extend_from_slice(&52u16.to_le_bytes()); // wValueLength
    b.extend_from_slice(&0u16.to_le_bytes()); // wType (binary)
    b.extend_from_slice(&key);
    while b.len() % 4 != 0 {
        b.push(0);
    }
    b.extend_from_slice(f);
    let n = b.len() as u16;
    b[0..2].copy_from_slice(&n.to_le_bytes());
    b
}

unsafe fn cstr(p: *const i8) -> String {
    if p.is_null() { String::new() } else { std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned() }
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn GetFileVersionInfoSizeA(file: *const i8, handle: *mut u32) -> u32 {
    if !handle.is_null() {
        *handle = 0;
    }
    fixed_info(&cstr(file)).map_or(0, |f| version_block(&f).len() as u32)
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn GetFileVersionInfoA(file: *const i8, _handle: u32, len: u32, data: *mut u8) -> i32 {
    let Some(f) = fixed_info(&cstr(file)) else { return 0 };
    let b = version_block(&f);
    if data.is_null() || (len as usize) < b.len() {
        return 0;
    }
    std::ptr::copy_nonoverlapping(b.as_ptr(), data, b.len());
    1
}

/// Only the root ("\\"), which is the VS_FIXEDFILEINFO; the string tables are not provided.
#[allow(non_snake_case)]
unsafe extern "stdcall" fn VerQueryValueA(block: *const u8, sub: *const i8, out: *mut u32, len: *mut u32) -> i32 {
    if block.is_null() || out.is_null() || cstr(sub) != "\\" {
        return 0;
    }
    let total = std::ptr::read_unaligned(block as *const u16) as usize;
    let data = std::slice::from_raw_parts(block, total);
    let Some(at) = (0..=total.saturating_sub(52)).step_by(4).find(|&i| data[i..i + 4] == SIGNATURE.to_le_bytes()) else {
        return 0;
    };
    *out = block.add(at) as u32;
    if !len.is_null() {
        *len = 52;
    }
    1
}
