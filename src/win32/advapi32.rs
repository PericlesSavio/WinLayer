//! advapi32.dll — Windows registry, kept in memory (does not persist between runs).
//! Predefined keys: HKCR 0x80000000, HKCU 0x80000001, HKLM 0x80000002, HKU 0x80000003.

use std::collections::BTreeMap;
use std::sync::Mutex;

pub fn resolve(name: &str) -> Option<usize> {
    exports!(
        name,
        RegOpenKeyA, RegOpenKeyExA, RegCreateKeyA, RegCreateKeyExA, RegCloseKey, RegSetValueA, RegSetValueExA,
        RegQueryValueA, RegQueryValueExA, RegDeleteKeyA, RegDeleteValueA, RegEnumKeyA, RegEnumValueA,
    )
}

const ERROR_SUCCESS: u32 = 0;
const ERROR_FILE_NOT_FOUND: u32 = 2;
const ERROR_MORE_DATA: u32 = 234;
const ERROR_NO_MORE_ITEMS: u32 = 259;
const REG_SZ: u32 = 1;

struct Value {
    name: String,
    ty: u32,
    data: Vec<u8>,
}

#[derive(Default)]
struct Key {
    /// Values by lowercase name ("" = default value).
    values: BTreeMap<String, Value>,
}

struct Registry {
    keys: BTreeMap<String, Key>,
    handles: BTreeMap<u32, String>,
    next: u32,
}

static REG: Mutex<Registry> = Mutex::new(Registry { keys: BTreeMap::new(), handles: BTreeMap::new(), next: 0x8100 });

fn root_name(h: u32) -> Option<&'static str> {
    Some(match h {
        0x8000_0000 => "hkcr",
        0x8000_0001 => "hkcu",
        0x8000_0002 => "hklm",
        0x8000_0003 => "hku",
        _ => return None,
    })
}

/// Normalized path (lowercase, `\`) of `key` + `sub`.
fn full_path(r: &Registry, key: u32, sub: &str) -> Option<String> {
    let base = match root_name(key) {
        Some(n) => n.to_string(),
        None => r.handles.get(&key)?.clone(),
    };
    let sub = sub.replace('/', "\\").trim_matches('\\').to_ascii_lowercase();
    Some(if sub.is_empty() { base } else { format!("{base}\\{sub}") })
}

fn ensure(r: &mut Registry, path: &str) {
    let mut cur = String::new();
    for part in path.split('\\') {
        if !cur.is_empty() {
            cur.push('\\');
        }
        cur.push_str(part);
        r.keys.entry(cur.clone()).or_default();
    }
}

fn new_handle(r: &mut Registry, path: String) -> u32 {
    let h = r.next;
    r.next += 4;
    r.handles.insert(h, path);
    h
}

unsafe fn s(p: u32) -> String {
    if p == 0 { String::new() } else { std::ffi::CStr::from_ptr(p as usize as *const i8).to_string_lossy().into_owned() }
}

fn open(key: u32, sub: &str, create: bool) -> Option<u32> {
    let mut r = REG.lock().unwrap();
    let path = full_path(&r, key, sub)?;
    if !r.keys.contains_key(&path) {
        if root_name(key).is_some() && sub.is_empty() {
            ensure(&mut r, &path); // the roots always exist
        } else if create {
            ensure(&mut r, &path);
        } else {
            return None;
        }
    }
    Some(new_handle(&mut r, path))
}

// ---- open / create / close ----------------------------------------------------------------

#[allow(non_snake_case)]
unsafe extern "stdcall" fn RegOpenKeyA(key: u32, sub: u32, out: *mut u32) -> u32 {
    match open(key, &s(sub), false) {
        Some(h) => { *out = h; ERROR_SUCCESS }
        None => ERROR_FILE_NOT_FOUND,
    }
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn RegOpenKeyExA(key: u32, sub: u32, _opt: u32, _sam: u32, out: *mut u32) -> u32 {
    RegOpenKeyA(key, sub, out)
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn RegCreateKeyA(key: u32, sub: u32, out: *mut u32) -> u32 {
    match open(key, &s(sub), true) {
        Some(h) => { *out = h; ERROR_SUCCESS }
        None => ERROR_FILE_NOT_FOUND,
    }
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn RegCreateKeyExA(key: u32, sub: u32, _r: u32, _class: u32, _opt: u32, _sam: u32, _sa: u32, out: *mut u32, disp: *mut u32) -> u32 {
    let existed = {
        let r = REG.lock().unwrap();
        full_path(&r, key, &s(sub)).is_some_and(|p| r.keys.contains_key(&p))
    };
    let code = RegCreateKeyA(key, sub, out);
    if !disp.is_null() {
        *disp = if existed { 2 } else { 1 }; // REG_OPENED_EXISTING_KEY / REG_CREATED_NEW_KEY
    }
    code
}

#[allow(non_snake_case)]
extern "stdcall" fn RegCloseKey(h: u32) -> u32 {
    REG.lock().unwrap().handles.remove(&h);
    ERROR_SUCCESS
}

// ---- values -------------------------------------------------------------------------------

fn set_value(key: u32, name: &str, ty: u32, data: Vec<u8>) -> u32 {
    let mut r = REG.lock().unwrap();
    let Some(path) = full_path(&r, key, "") else { return ERROR_FILE_NOT_FOUND };
    ensure(&mut r, &path);
    r.keys.get_mut(&path).unwrap().values.insert(name.to_ascii_lowercase(), Value { name: name.to_string(), ty, data });
    ERROR_SUCCESS
}

fn get_value(key: u32, sub: &str, name: &str) -> Option<(u32, Vec<u8>)> {
    let r = REG.lock().unwrap();
    let path = full_path(&r, key, sub)?;
    r.keys.get(&path)?.values.get(&name.to_ascii_lowercase()).map(|v| (v.ty, v.data.clone()))
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn RegSetValueA(key: u32, sub: u32, _ty: u32, data: u32, _cb: u32) -> u32 {
    let sub = s(sub);
    let mut v = s(data).into_bytes();
    v.push(0);
    match open(key, &sub, true) {
        Some(h) => {
            let code = set_value(h, "", REG_SZ, v);
            RegCloseKey(h);
            code
        }
        None => ERROR_FILE_NOT_FOUND,
    }
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn RegSetValueExA(key: u32, name: u32, _r: u32, ty: u32, data: *const u8, cb: u32) -> u32 {
    let bytes = if data.is_null() { Vec::new() } else { std::slice::from_raw_parts(data, cb as usize).to_vec() };
    set_value(key, &s(name), ty, bytes)
}

/// Copies `data` to `buf` honoring the size in `*cb` (input/output).
unsafe fn give(data: &[u8], buf: *mut u8, cb: *mut u32) -> u32 {
    if cb.is_null() {
        return ERROR_SUCCESS;
    }
    let cap = *cb as usize;
    *cb = data.len() as u32;
    if buf.is_null() {
        return ERROR_SUCCESS;
    }
    if cap < data.len() {
        return ERROR_MORE_DATA;
    }
    std::ptr::copy_nonoverlapping(data.as_ptr(), buf, data.len());
    ERROR_SUCCESS
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn RegQueryValueA(key: u32, sub: u32, buf: *mut u8, cb: *mut u32) -> u32 {
    match get_value(key, &s(sub), "") {
        Some((_, data)) => give(&data, buf, cb),
        None => ERROR_FILE_NOT_FOUND,
    }
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn RegQueryValueExA(key: u32, name: u32, _r: u32, ty: *mut u32, buf: *mut u8, cb: *mut u32) -> u32 {
    match get_value(key, "", &s(name)) {
        Some((t, data)) => {
            if !ty.is_null() {
                *ty = t;
            }
            give(&data, buf, cb)
        }
        None => ERROR_FILE_NOT_FOUND,
    }
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn RegDeleteValueA(key: u32, name: u32) -> u32 {
    let mut r = REG.lock().unwrap();
    let Some(path) = full_path(&r, key, "") else { return ERROR_FILE_NOT_FOUND };
    match r.keys.get_mut(&path).and_then(|k| k.values.remove(&s(name).to_ascii_lowercase())) {
        Some(_) => ERROR_SUCCESS,
        None => ERROR_FILE_NOT_FOUND,
    }
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn RegDeleteKeyA(key: u32, sub: u32) -> u32 {
    let mut r = REG.lock().unwrap();
    let Some(path) = full_path(&r, key, &s(sub)) else { return ERROR_FILE_NOT_FOUND };
    if r.keys.remove(&path).is_none() {
        return ERROR_FILE_NOT_FOUND;
    }
    let prefix = format!("{path}\\");
    r.keys.retain(|k, _| !k.starts_with(&prefix));
    ERROR_SUCCESS
}

// ---- enumeration --------------------------------------------------------------------------

#[allow(non_snake_case)]
unsafe extern "stdcall" fn RegEnumKeyA(key: u32, idx: u32, buf: *mut u8, cb: u32) -> u32 {
    let r = REG.lock().unwrap();
    let Some(path) = full_path(&r, key, "") else { return ERROR_FILE_NOT_FOUND };
    let prefix = format!("{path}\\");
    let name = r.keys.keys().filter_map(|k| k.strip_prefix(&prefix)).filter(|rest| !rest.contains('\\')).nth(idx as usize);
    match name {
        Some(n) if (n.len() as u32) < cb => {
            std::ptr::copy_nonoverlapping(n.as_ptr(), buf, n.len());
            *buf.add(n.len()) = 0;
            ERROR_SUCCESS
        }
        Some(_) => ERROR_MORE_DATA,
        None => ERROR_NO_MORE_ITEMS,
    }
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn RegEnumValueA(key: u32, idx: u32, name: *mut u8, name_len: *mut u32, _r: u32, ty: *mut u32, data: *mut u8, data_len: *mut u32) -> u32 {
    let r = REG.lock().unwrap();
    let Some(path) = full_path(&r, key, "") else { return ERROR_FILE_NOT_FOUND };
    let Some(v) = r.keys.get(&path).and_then(|k| k.values.values().nth(idx as usize)) else { return ERROR_NO_MORE_ITEMS };
    if (v.name.len() as u32) >= *name_len {
        return ERROR_MORE_DATA;
    }
    std::ptr::copy_nonoverlapping(v.name.as_ptr(), name, v.name.len());
    *name.add(v.name.len()) = 0;
    *name_len = v.name.len() as u32;
    if !ty.is_null() {
        *ty = v.ty;
    }
    if !data_len.is_null() {
        return give(&v.data, data, data_len);
    }
    ERROR_SUCCESS
}

// ---- .reg files ---------------------------------------------------------------------------

/// Parses a quoted .reg string, returning it and the rest of the line.
fn reg_string(s: &str) -> Option<(String, &str)> {
    let mut out = String::new();
    let mut chars = s.strip_prefix('"')?.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '\\' => out.push(chars.next()?.1),
            '"' => return Some((out, &s[i + 2..])),
            c => out.push(c),
        }
    }
    None
}

/// Imports a Regedit file (REGEDIT4 or version 5.00): keys, string (`"x"`), `dword:` and `hex:` values.
/// Games read from the registry what their installer wrote there (install folder, CD path).
pub fn import_reg(text: &str) -> Result<usize, String> {
    let mut r = REG.lock().unwrap();
    let mut key: Option<String> = None;
    let mut count = 0;
    // hex values may continue on the next line after a trailing backslash
    let joined = text.replace("\\\r\n", "").replace("\\\n", "");
    for (n, raw) in joined.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with("REGEDIT") || line.starts_with("Windows Registry") {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            let (root, rest) = name.split_once('\\').unwrap_or((name, ""));
            let root = match root.to_ascii_uppercase().as_str() {
                "HKEY_CLASSES_ROOT" | "HKCR" => "hkcr",
                "HKEY_CURRENT_USER" | "HKCU" => "hkcu",
                "HKEY_LOCAL_MACHINE" | "HKLM" => "hklm",
                "HKEY_USERS" | "HKU" => "hku",
                other => return Err(format!("line {}: unknown root key {other}", n + 1)),
            };
            let path = if rest.is_empty() { root.to_string() } else { format!("{root}\\{}", rest.to_ascii_lowercase()) };
            ensure(&mut r, &path);
            key = Some(path);
            continue;
        }
        let Some(k) = key.clone() else { return Err(format!("line {}: value outside a key", n + 1)) };
        let (name, rest) = if let Some(rest) = line.strip_prefix('@') {
            (String::new(), rest)
        } else {
            reg_string(line).ok_or(format!("line {}: bad value name", n + 1))?
        };
        let data = rest.trim_start().strip_prefix('=').ok_or(format!("line {}: missing '='", n + 1))?.trim();
        let (ty, bytes) = if data.starts_with('"') {
            let (v, _) = reg_string(data).ok_or(format!("line {}: bad string", n + 1))?;
            let mut b = v.into_bytes();
            b.push(0);
            (REG_SZ, b)
        } else if let Some(h) = data.strip_prefix("dword:") {
            let v = u32::from_str_radix(h.trim(), 16).map_err(|e| format!("line {}: {e}", n + 1))?;
            (4, v.to_le_bytes().to_vec())
        } else if let Some(h) = data.strip_prefix("hex") {
            // hex:aa,bb (binary) or hex(N):aa,bb (type N)
            let (ty, list) = match h.strip_prefix(':') {
                Some(list) => (3, list),
                None => {
                    let (t, list) = h.strip_prefix('(').and_then(|x| x.split_once("):")).ok_or(format!("line {}: bad hex value", n + 1))?;
                    (u32::from_str_radix(t, 16).map_err(|e| format!("line {}: {e}", n + 1))?, list)
                }
            };
            let bytes: Result<Vec<u8>, _> =
                list.split(',').map(str::trim).filter(|x| !x.is_empty()).map(|x| u8::from_str_radix(x, 16)).collect();
            (ty, bytes.map_err(|e| format!("line {}: {e}", n + 1))?)
        } else {
            return Err(format!("line {}: unsupported value {data}", n + 1));
        };
        r.keys.entry(k).or_default().values.insert(name.to_ascii_lowercase(), Value { name, ty, data: bytes });
        count += 1;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_reg_values() {
        let text = "REGEDIT4\n\n[HKEY_LOCAL_MACHINE\\Software\\Test Co\\Game]\n\"CDPath\"=\"D:\\\\\"\n@=\"default\"\n\"Count\"=dword:0000000a\n\"Bin\"=hex:01,02,\\\n  03\n";
        assert_eq!(import_reg(text), Ok(4));
        let r = REG.lock().unwrap();
        let k = &r.keys["hklm\\software\\test co\\game"];
        assert_eq!(k.values["cdpath"].data, b"D:\\\0");
        assert_eq!(k.values[""].data, b"default\0");
        assert_eq!(k.values["count"].data, 10u32.to_le_bytes());
        assert_eq!((k.values["bin"].ty, k.values["bin"].data.as_slice()), (3, &[1u8, 2, 3][..]));
    }
}
