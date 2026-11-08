//! PE resources (`.rsrc`) read directly from the image already mapped in memory.

pub const RT_MENU: u32 = 4;
pub const RT_DIALOG: u32 = 5;
pub const RT_STRING: u32 = 6;
pub const RT_ACCELERATOR: u32 = 9;

/// Resource identifier: number (MAKEINTRESOURCE) or name.
pub enum ResId {
    Id(u32),
    Name(String),
}

/// Interprets the `LPCSTR` argument of FindResource/LoadString etc.
pub unsafe fn res_id(p: u32) -> ResId {
    if p < 0x10000 {
        return ResId::Id(p);
    }
    let s = std::ffi::CStr::from_ptr(p as usize as *const i8).to_string_lossy().into_owned();
    match s.strip_prefix('#').and_then(|n| n.parse().ok()) {
        Some(n) => ResId::Id(n),
        None => ResId::Name(s),
    }
}

unsafe fn u16_at(a: u32) -> u32 {
    *(a as usize as *const u16) as u32
}
unsafe fn u32_at(a: u32) -> u32 {
    *(a as usize as *const u32)
}

/// Looks up `id` in a resource directory; returns the entry's `OffsetToData` field.
unsafe fn lookup(root: u32, dir_off: u32, id: Option<&ResId>) -> Option<u32> {
    let dir = root + dir_off;
    let (named, ids) = (u16_at(dir + 12), u16_at(dir + 14));
    let entries = dir + 16;
    for i in 0..named + ids {
        let e = entries + i * 8;
        let (name, off) = (u32_at(e), u32_at(e + 4));
        let hit = match id {
            None => true, // any entry (language)
            Some(ResId::Id(n)) => i >= named && name == *n,
            Some(ResId::Name(s)) => i < named && {
                let p = root + (name & 0x7FFF_FFFF);
                let len = u16_at(p) as usize;
                let chars: Vec<u16> = (0..len).map(|k| u16_at(p + 2 + 2 * k as u32) as u16).collect();
                String::from_utf16_lossy(&chars).eq_ignore_ascii_case(s)
            },
        };
        if hit {
            return Some(off);
        }
    }
    None
}

/// Locates a resource; returns (IMAGE_RESOURCE_DATA_ENTRY address, data address, size).
pub unsafe fn find(base: u32, ty: &ResId, name: &ResId) -> Option<(u32, u32, u32)> {
    if base == 0 {
        return None;
    }
    // Resource directory read from the module's own headers (exe or DLL).
    let lfanew = u32_at(base + 0x3C);
    let rva = u32_at(base + lfanew + 24 + 96 + 16);
    if rva == 0 {
        return None;
    }
    let root = base + rva;
    let d1 = lookup(root, 0, Some(ty))?;
    if d1 & 0x8000_0000 == 0 { return None }
    let d2 = lookup(root, d1 & 0x7FFF_FFFF, Some(name))?;
    if d2 & 0x8000_0000 == 0 { return None }
    let d3 = lookup(root, d2 & 0x7FFF_FFFF, None)?;
    let entry = root + (d3 & 0x7FFF_FFFF);
    Some((entry, base + u32_at(entry), u32_at(entry + 4)))
}

/// Reads a NUL-terminated UTF-16 string at `p`; returns (text, next address).
pub unsafe fn wide_z(p: u32) -> (String, u32) {
    let mut chars = Vec::new();
    let mut q = p;
    loop {
        let c = u16_at(q) as u16;
        q += 2;
        if c == 0 { break }
        chars.push(c);
    }
    (String::from_utf16_lossy(&chars), q)
}
