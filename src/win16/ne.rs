//! Loader for NE executables (16-bit Windows 3.x).

use super::ldt::{self, Kind};
use super::trans::map_rwx;

pub struct Segment {
    pub sel: u16,
    pub base: u32,
    pub size: u32,
    pub is_data: bool,
}

#[derive(Clone, PartialEq, Debug)]
pub enum ResKey {
    Id(u16),
    Name(String),
}

pub struct ResEntry {
    pub key: ResKey,
    pub off: u32,
    pub len: u32,
}

pub struct ResType {
    pub key: ResKey,
    pub entries: Vec<ResEntry>,
}

pub struct Ne {
    pub resources: Vec<ResType>,
    pub segs: Vec<Segment>,
    /// Ordinal-1 -> (1-based segment number, offset).
    pub entries: Vec<Option<(u8, u16)>>,
    pub modules: Vec<String>,
    /// Index (0-based) of the automatic data segment (DGROUP).
    pub dgroup: Option<usize>,
    pub stack: u16,
    pub heap: u16,
    pub entry: (u8, u16),
    pub stack_start: (u8, u16),
}

fn u16_at(d: &[u8], o: usize) -> Result<u16, String> {
    d.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]])).ok_or_else(|| "truncated NE".to_string())
}

fn u32_at(d: &[u8], o: usize) -> Result<u32, String> {
    d.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).ok_or_else(|| "truncated NE".to_string())
}

/// Resolved address of an import: (selector, offset). Absolute constants (__AHSHIFT...) come
/// with selector 0 and the value in the offset.
pub type Resolver<'a> = dyn FnMut(&str, u16) -> (u16, u16) + 'a;

pub fn load(d: &[u8], resolve: &mut Resolver) -> Result<Ne, String> {
    let h = u32_at(d, 0x3C)? as usize;
    if d.get(h..h + 2) != Some(b"NE") {
        return Err("not an NE executable".into());
    }
    let off = |o: usize| u16_at(d, h + o);
    let (autodata, heap, stack) = (off(0x0E)? as usize, off(0x10)?, off(0x12)?);
    let cs_ip = u32_at(d, h + 0x14)?;
    let ss_sp = u32_at(d, h + 0x18)?;
    let (nseg, nmod) = (off(0x1C)? as usize, off(0x1E)? as usize);
    let (segtab, modtab, imptab) = (off(0x22)? as usize, off(0x28)? as usize, off(0x2A)? as usize);
    let (entry_tab, align) = (off(0x04)? as usize, off(0x32)? as u32);

    // ---- imported modules
    let mut modules = Vec::new();
    for i in 0..nmod {
        let o = h + imptab + u16_at(d, h + modtab + i * 2)? as usize;
        let n = *d.get(o).ok_or("truncated module table")? as usize;
        modules.push(String::from_utf8_lossy(&d[o + 1..o + 1 + n]).into_owned());
    }

    // ---- entry table (ordinal -> segment:offset)
    let mut entries: Vec<Option<(u8, u16)>> = Vec::new();
    let mut p = h + entry_tab;
    loop {
        let count = *d.get(p).ok_or("truncated entry table")? as usize;
        if count == 0 {
            break;
        }
        let ind = d[p + 1];
        p += 2;
        for _ in 0..count {
            match ind {
                0 => entries.push(None),
                0xFF => {
                    entries.push(Some((d[p + 3], u16_at(d, p + 4)?)));
                    p += 6;
                }
                seg => {
                    entries.push(Some((seg, u16_at(d, p + 1)?)));
                    p += 3;
                }
            }
        }
    }

    // ---- allocates and loads the segments
    struct Raw {
        file_off: usize,
        len: usize,
        flags: u16,
        alloc: usize,
    }
    let mut raws = Vec::new();
    for i in 0..nseg {
        let o = h + segtab + i * 8;
        let (so, sl, fl, ma) = (u16_at(d, o)?, u16_at(d, o + 2)?, u16_at(d, o + 4)?, u16_at(d, o + 6)?);
        let len = if so != 0 && sl == 0 { 0x1_0000 } else { sl as usize };
        let alloc = if ma == 0 { 0x1_0000 } else { ma as usize };
        raws.push(Raw { file_off: (so as usize) << align, len: if so == 0 { 0 } else { len }, flags: fl, alloc: alloc.max(len) });
    }
    let dg_index = if autodata != 0 { Some(autodata - 1) } else { None };
    let mut segs = Vec::new();
    for (i, r) in raws.iter().enumerate() {
        let is_data = r.flags & 1 != 0;
        let mut size = r.alloc;
        if Some(i) == dg_index {
            // DGROUP = data + stack + local heap, at most 64 KiB
            size = (r.alloc + stack as usize + heap as usize).min(0x1_0000);
        }
        let size = size.max(1);
        let mem = map_rwx(size.div_ceil(4096) * 4096);
        if mem as isize == -1 {
            return Err("segment mmap failed".into());
        }
        if r.len > 0 {
            let src = d.get(r.file_off..r.file_off + r.len).ok_or("segment past the end of the file")?;
            unsafe { std::ptr::copy_nonoverlapping(src.as_ptr(), mem, r.len.min(size)) };
        }
        let kind = if is_data { Kind::Data16 } else { Kind::Code16 };
        // Limit with slack: real programs go a few bytes past the end of the segment and work on Windows
        // (the memory is zeroed and there is a guard page after it, so the rounding is harmless).
        let limit = (size.div_ceil(4096) * 4096).min(0x1_0000) - 1;
        let sel = ldt::alloc(mem as usize as u32, limit as u32, kind)?;
        segs.push(Segment { sel, base: mem as usize as u32, size: size as u32, is_data });
    }

    // ---- relocations
    for (i, r) in raws.iter().enumerate() {
        if r.flags & 0x100 == 0 || r.file_off == 0 {
            continue;
        }
        let rp = r.file_off + r.len;
        let n = u16_at(d, rp)? as usize;
        let sbase = segs[i].base;
        for k in 0..n {
            let e = rp + 2 + k * 8;
            let (atype, rflags) = (d[e] & 0x0F, d[e + 1]);
            let loc = u16_at(d, e + 2)? as u32;
            let (a, b) = (u16_at(d, e + 4)?, u16_at(d, e + 6)?);
            let additive = rflags & 4 != 0;
            let target: (u16, u16) = match rflags & 3 {
                0 => {
                    // internal reference
                    let (seg, off) = if a & 0xFF == 0xFF {
                        entries.get(b as usize - 1).copied().flatten().ok_or("nonexistent entry in internal reference")?
                    } else {
                        ((a & 0xFF) as u8, b)
                    };
                    (segs.get(seg as usize - 1).ok_or("invalid segment in internal reference")?.sel, off)
                }
                1 => resolve(&modules[a as usize - 1], b), // import by ordinal
                2 => {
                    let o = h + imptab + b as usize;
                    let name = String::from_utf8_lossy(&d[o + 1..o + 1 + d[o] as usize]).into_owned();
                    let ord = super::ordinals::table(&modules[a as usize - 1]).iter().find(|(_, n)| n.eq_ignore_ascii_case(&name)).map_or(0, |(o, _)| *o);
                    resolve(&modules[a as usize - 1], ord)
                }
                _ => continue, // OSFIXUP: the image already uses the native 8087 format
            };
            apply(sbase, segs[i].size, atype, additive, loc, target);
        }
    }

    let resources = parse_resources(d, h, off(0x24)? as usize, off(0x26)? as usize)?;
    Ok(Ne {
        resources,
        segs,
        entries,
        modules,
        dgroup: dg_index,
        stack,
        heap,
        entry: ((cs_ip >> 16) as u8, cs_ip as u16),
        stack_start: ((ss_sp >> 16) as u8, ss_sp as u16),
    })
}

/// Applies a relocation (following the chain, if not additive).
fn apply(base: u32, size: u32, atype: u8, additive: bool, mut loc: u32, (sel, off): (u16, u16)) {
    let rd = |at: u32| unsafe { std::ptr::read_unaligned((base + at) as usize as *const u16) };
    let wr = |at: u32, v: u16| unsafe { std::ptr::write_unaligned((base + at) as usize as *mut u16, v) };
    loop {
        if loc + 2 > size {
            return;
        }
        let next = if additive { 0xFFFF } else { rd(loc) };
        match atype {
            0 => unsafe { *((base + loc) as usize as *mut u8) = off as u8 }, // LOBYTE
            2 => wr(loc, sel),                                                // selector
            3 | 11 => {
                // far pointer
                let old = if additive { rd(loc) } else { 0 };
                wr(loc, off.wrapping_add(old));
                wr(loc + 2, sel);
            }
            5 | 13 => {
                let old = if additive { rd(loc) } else { 0 };
                wr(loc, off.wrapping_add(old));
            }
            _ => {}
        }
        if next == 0xFFFF {
            return;
        }
        loc = next as u32;
    }
}

/// NE resource table: types -> resources (offset and size in bytes in the file).
fn parse_resources(d: &[u8], h: usize, rsrc: usize, restab: usize) -> Result<Vec<ResType>, String> {
    let mut out = Vec::new();
    if rsrc == restab || rsrc == 0 {
        return Ok(out); // no resources
    }
    let base = h + rsrc;
    let shift = u16_at(d, base)? as u32;
    let name_at = |o: usize| -> String {
        let p = base + o;
        let n = d.get(p).copied().unwrap_or(0) as usize;
        String::from_utf8_lossy(d.get(p + 1..p + 1 + n).unwrap_or(&[])).into_owned()
    };
    let mut p = base + 2;
    loop {
        let tid = u16_at(d, p)?;
        if tid == 0 {
            break;
        }
        let count = u16_at(d, p + 2)? as usize;
        let key = if tid & 0x8000 != 0 { ResKey::Id(tid & 0x7FFF) } else { ResKey::Name(name_at(tid as usize)) };
        p += 8;
        let mut entries = Vec::new();
        for _ in 0..count {
            let (o, l, _fl, id) = (u16_at(d, p)? as u32, u16_at(d, p + 2)? as u32, u16_at(d, p + 4)?, u16_at(d, p + 6)?);
            let k = if id & 0x8000 != 0 { ResKey::Id(id & 0x7FFF) } else { ResKey::Name(name_at(id as usize)) };
            entries.push(ResEntry { key: k, off: o << shift, len: l << shift });
            p += 12;
        }
        out.push(ResType { key, entries });
    }
    Ok(out)
}
