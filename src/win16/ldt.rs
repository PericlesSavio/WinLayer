//! Descritores LDT (modify_ldt): segmentos de 16 bits para código e dados Win16.

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

/// Layout de `struct user_desc` do Linux (16 bytes).
#[repr(C)]
struct UserDesc {
    entry_number: u32,
    base_addr: u32,
    limit: u32,
    flags: u32,
}

static TABLE: Mutex<BTreeMap<u16, Seg>> = Mutex::new(BTreeMap::new());
static NEXT: Mutex<u32> = Mutex::new(1);
/// Entradas liberadas, para reaproveitar (a LDT tem no máximo 8192).
static FREE: Mutex<Vec<u32>> = Mutex::new(Vec::new()); // entrada 0 fica reservada (seletor "nulo" do Win16)

/// Seletor LDT (RPL 3) de uma entrada.
fn selector(entry: u32) -> u16 {
    ((entry << 3) | 7) as u16
}

/// Cria um segmento de 16 bits em [base, base + limit]; devolve o seletor.
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
    // flags: seg_32bit(bit0)=0 | contents(bits1-2): 0 dados, 2 código | read_exec_only(bit3)=0 | useable(bit6)
    let contents = if kind == Kind::Code16 { 2 } else { 0 };
    let mut d = UserDesc { entry_number: entry, base_addr: base, limit, flags: (contents << 1) | (1 << 6) };
    let r = unsafe { syscall(SYS_MODIFY_LDT, 1 as c_long, &mut d as *mut UserDesc, std::mem::size_of::<UserDesc>() as c_long) };
    if r != 0 {
        return Err(format!("modify_ldt falhou ({r}) ao criar o seletor {entry}"));
    }
    let sel = selector(entry);
    TABLE.lock().unwrap().insert(sel, Seg { base, limit, kind });
    Ok(sel)
}

/// Reaponta um segmento existente (usado por GlobalReAlloc de blocos movidos).
pub fn lookup(sel: u16) -> Option<Seg> {
    TABLE.lock().unwrap().get(&sel).copied()
}

/// Endereço linear de um ponteiro far `sel:off`.
pub fn linear(sel: u16, off: u16) -> Option<u32> {
    lookup(sel).map(|s| s.base.wrapping_add(off as u32))
}

/// Aloca `size` bytes a partir de `base` como uma corrida de seletores consecutivos de 64 KiB
/// (ponteiros "huge": o seletor seguinte é o anterior + 8, o `__AHINCR`). Devolve o primeiro.
pub fn alloc_run(base: u32, size: u32, kind: Kind) -> Result<u16, String> {
    let chunks = size.max(1).div_ceil(0x1_0000);
    // Reserva as entradas de uma vez para garantir que sejam consecutivas.
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
        return Err(format!("modify_ldt falhou ({r}) na entrada {entry}"));
    }
    TABLE.lock().unwrap().insert(selector(entry), Seg { base, limit, kind });
    Ok(())
}

/// Reaponta um seletor existente (GlobalReAlloc de um bloco que mudou de lugar).
pub fn update(sel: u16, base: u32, limit: u32) -> Result<(), String> {
    let kind = lookup(sel).map_or(Kind::Data16, |s| s.kind);
    write_entry((sel as u32) >> 3, base, limit, kind)
}

/// Devolve um seletor à lista de livres (o descritor antigo é sobrescrito na próxima alocação).
pub fn release(sel: u16) {
    if std::env::var_os("WINLAYER_NO_LDT_REUSE").is_some() {
        return;
    }
    if TABLE.lock().unwrap().remove(&sel).is_some() {
        FREE.lock().unwrap().push((sel as u32) >> 3);
    }
}

/// Depuração: confere se todo segmento cabe inteiro numa região mapeada de leitura/escrita
/// (lendo /proc/self/maps) e imprime os que não cabem. Devolve quantos estão errados.
pub fn audit(tag: &str) -> usize {
    let Ok(maps) = std::fs::read_to_string("/proc/self/maps") else { return 0 };
    let regions: Vec<(u64, u64, bool)> = maps
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let (range, perms) = (it.next()?, it.next()?);
            let (a, b) = range.split_once('-')?;
            // regiões "nomeadas" (binário, [heap], [stack]...) pertencem ao host: segmentos não podem tocá-las
            let named = it.nth(3).is_some();
            Some((u64::from_str_radix(a, 16).ok()?, u64::from_str_radix(b, 16).ok()?, perms.starts_with("rw") && !named))
        })
        .collect();
    let mut bad = 0;
    for (sel, seg) in TABLE.lock().unwrap().iter() {
        let (lo, hi) = (seg.base as u64, seg.base as u64 + seg.limit as u64 + 1);
        // pode atravessar regiões adjacentes: confere cobertura contínua
        let mut cur = lo;
        while cur < hi {
            match regions.iter().find(|r| r.0 <= cur && cur < r.1 && r.2) {
                Some(r) => cur = r.1,
                None => break,
            }
        }
        if cur < hi {
            bad += 1;
            eprintln!("[audit {tag}] seletor {sel:#06x} base={:#x} limite={:#x}: fora de região RW (cobre até {cur:#x})", seg.base, seg.limit);
        }
    }
    bad
}

/// Libera uma corrida de `chunks` seletores consecutivos (bloco global liberado).
pub fn release_run(sel: u16, chunks: u32) {
    for i in 0..chunks {
        release(sel.wrapping_add((i * 8) as u16));
    }
}

/// Depuração: lista os segmentos vivos que se sobrepõem a outro (aliasing suspeito).
pub fn overlaps() {
    let t = TABLE.lock().unwrap();
    let v: Vec<(&u16, &Seg)> = t.iter().collect();
    for i in 0..v.len() {
        for j in i + 1..v.len() {
            let (a, b) = (v[i].1, v[j].1);
            let (a0, a1, b0, b1) = (a.base as u64, a.base as u64 + a.limit as u64, b.base as u64, b.base as u64 + b.limit as u64);
            if a0 <= b1 && b0 <= a1 {
                eprintln!("[ldt] sobreposição: {:#06x} [{:#x}+{:#x}] x {:#06x} [{:#x}+{:#x}]", v[i].0, a.base, a.limit, v[j].0, b.base, b.limit);
            }
        }
    }
}

/// Depuração: número de segmentos vivos (percorre a tabela).
pub fn count() -> usize {
    let t = TABLE.lock().unwrap();
    t.iter().map(|(_, s)| (s.limit as usize) & 0).sum::<usize>() + t.len()
}
