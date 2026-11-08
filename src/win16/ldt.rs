//! LDT descriptors (modify_ldt): 16-bit segments for Win16 code and data.

use std::collections::BTreeMap;
use std::ffi::c_long;
use std::sync::Mutex;

extern "C" {
    fn syscall(num: c_long, ...) -> c_long;
}

const SYS_MODIFY_LDT: c_long = 123; // i386

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Code16,
    Data16,
}

#[derive(Clone, Copy, Debug)]
pub struct Seg {
    pub base: u32,
    pub limit: u32,
    pub kind: Kind,
}

/// Layout of Linux `struct user_desc` (16 bytes).
#[repr(C)]
struct UserDesc {
    entry_number: u32,
    base_addr: u32,
    limit: u32,
    flags: u32,
}

static TABLE: Mutex<BTreeMap<u16, Seg>> = Mutex::new(BTreeMap::new());
static NEXT: Mutex<u32> = Mutex::new(1);
/// Freed entries, for reuse (the LDT has at most 8192).
static FREE: Mutex<Vec<u32>> = Mutex::new(Vec::new()); // entry 0 stays reserved (Win16 "null" selector)

/// LDT selector (RPL 3) of an entry.
fn selector(entry: u32) -> u16 {
    ((entry << 3) | 7) as u16
}

/// Creates a 16-bit segment over [base, base + limit]; returns the selector.
pub fn alloc(base: u32, limit: u32, kind: Kind) -> Result<u16, String> {
    let entry = match FREE.lock().unwrap().pop() {
        Some(e) => e,
        None => {
            let mut n = NEXT.lock().unwrap();
            let e = *n;
            *n += 1;
            e
        }
    };
    // flags: seg_32bit(bit0)=0 | contents(bits1-2): 0 data, 2 code | read_exec_only(bit3)=0 | useable(bit6)
    let contents = if kind == Kind::Code16 { 2 } else { 0 };
    let mut d = UserDesc { entry_number: entry, base_addr: base, limit, flags: (contents << 1) | (1 << 6) };
    let r = unsafe { syscall(SYS_MODIFY_LDT, 1 as c_long, &mut d as *mut UserDesc, std::mem::size_of::<UserDesc>() as c_long) };
    if r != 0 {
        return Err(format!("modify_ldt failed ({r}) creating selector {entry}"));
    }
    let sel = selector(entry);
    TABLE.lock().unwrap().insert(sel, Seg { base, limit, kind });
    Ok(sel)
}

/// Repoints an existing segment (used by GlobalReAlloc of moved blocks).
pub fn lookup(sel: u16) -> Option<Seg> {
    TABLE.lock().unwrap().get(&sel).copied()
}

/// Linear address of a far pointer `sel:off`.
pub fn linear(sel: u16, off: u16) -> Option<u32> {
    lookup(sel).map(|s| s.base.wrapping_add(off as u32))
}

/// Allocates `size` bytes starting at `base` as a run of consecutive 64 KiB selectors
/// ("huge" pointers: the next selector is the previous + 8, the `__AHINCR`). Returns the first.
pub fn alloc_run(base: u32, size: u32, kind: Kind) -> Result<u16, String> {
    let chunks = size.max(1).div_ceil(0x1_0000);
    // Reserves the entries at once to guarantee they are consecutive.
    let first_entry = {
        let mut n = NEXT.lock().unwrap();
        let e = *n;
        *n += chunks;
        e
    };
    for i in 0..chunks {
        let chunk_base = base.wrapping_add(i * 0x1_0000);
        let chunk_len = if i + 1 == chunks { size - i * 0x1_0000 } else { 0x1_0000 };
        write_entry(first_entry + i, chunk_base, chunk_len.max(1) - 1, kind)?;
    }
    Ok(selector(first_entry))
}

fn write_entry(entry: u32, base: u32, limit: u32, kind: Kind) -> Result<(), String> {
    let contents = if kind == Kind::Code16 { 2 } else { 0 };
    let mut d = UserDesc { entry_number: entry, base_addr: base, limit, flags: (contents << 1) | (1 << 6) };
    let r = unsafe { syscall(SYS_MODIFY_LDT, 1 as c_long, &mut d as *mut UserDesc, std::mem::size_of::<UserDesc>() as c_long) };
    if r != 0 {
        return Err(format!("modify_ldt failed ({r}) at entry {entry}"));
    }
    TABLE.lock().unwrap().insert(selector(entry), Seg { base, limit, kind });
    Ok(())
}

/// Repoints an existing selector (GlobalReAlloc of a block that moved).
pub fn update(sel: u16, base: u32, limit: u32) -> Result<(), String> {
    let kind = lookup(sel).map_or(Kind::Data16, |s| s.kind);
    write_entry((sel as u32) >> 3, base, limit, kind)
}

/// Returns a selector to the free list (the old descriptor is overwritten on the next allocation).
pub fn release(sel: u16) {
    if std::env::var_os("WINLAYER_NO_LDT_REUSE").is_some() {
        return;
    }
    if TABLE.lock().unwrap().remove(&sel).is_some() {
        FREE.lock().unwrap().push((sel as u32) >> 3);
    }
}

/// Debugging: checks that every segment fits entirely in a mapped read/write region
/// (reading /proc/self/maps) and prints those that do not. Returns how many are wrong.
pub fn audit(tag: &str) -> usize {
    let Ok(maps) = std::fs::read_to_string("/proc/self/maps") else { return 0 };
    let regions: Vec<(u64, u64, bool)> = maps
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let (range, perms) = (it.next()?, it.next()?);
            let (a, b) = range.split_once('-')?;
            // "named" regions (binary, [heap], [stack]...) belong to the host: segments must not touch them
            let named = it.nth(3).is_some();
            Some((u64::from_str_radix(a, 16).ok()?, u64::from_str_radix(b, 16).ok()?, perms.starts_with("rw") && !named))
        })
        .collect();
    let mut bad = 0;
    for (sel, seg) in TABLE.lock().unwrap().iter() {
        let (lo, hi) = (seg.base as u64, seg.base as u64 + seg.limit as u64 + 1);
        // may span adjacent regions: checks for continuous coverage
        let mut cur = lo;
        while cur < hi {
            match regions.iter().find(|r| r.0 <= cur && cur < r.1 && r.2) {
                Some(r) => cur = r.1,
                None => break,
            }
        }
        if cur < hi {
            bad += 1;
            eprintln!("[audit {tag}] selector {sel:#06x} base={:#x} limit={:#x}: outside RW region (covers up to {cur:#x})", seg.base, seg.limit);
        }
    }
    bad
}

/// Frees a run of `chunks` consecutive selectors (freed global block).
pub fn release_run(sel: u16, chunks: u32) {
    for i in 0..chunks {
        release(sel.wrapping_add((i * 8) as u16));
    }
}

/// Debugging: lists live segments that overlap another one (suspicious aliasing).
pub fn overlaps() {
    let t = TABLE.lock().unwrap();
    let v: Vec<(&u16, &Seg)> = t.iter().collect();
    for i in 0..v.len() {
        for j in i + 1..v.len() {
            let (a, b) = (v[i].1, v[j].1);
            let (a0, a1, b0, b1) = (a.base as u64, a.base as u64 + a.limit as u64, b.base as u64, b.base as u64 + b.limit as u64);
            if a0 <= b1 && b0 <= a1 {
                eprintln!("[ldt] overlap: {:#06x} [{:#x}+{:#x}] x {:#06x} [{:#x}+{:#x}]", v[i].0, a.base, a.limit, v[j].0, b.base, b.limit);
            }
        }
    }
}

/// Debugging: number of live segments (walks the table).
pub fn count() -> usize {
    let t = TABLE.lock().unwrap();
    t.iter().map(|(_, s)| (s.limit as usize) & 0).sum::<usize>() + t.len()
}
