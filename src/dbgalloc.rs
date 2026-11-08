//! Alocador de depuração (ativado por `WINLAYER_HEAPCHECK=1`): cerca cada alocação com canários e
//! permite conferir todas as alocações vivas (`check_all`), para achar escritas fora de buffers.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub struct Dbg;

const PRE: usize = 32;
const POST: usize = 32;
const MAGIC: u8 = 0xA5;
const SLOTS: usize = 1 << 16;

static ENABLED: AtomicBool = AtomicBool::new(false);
/// Modo "bump": nunca libera; evita o malloc do musl (para isolar corrupção de metadados dele).
static BUMP: AtomicBool = AtomicBool::new(false);
static BUMP_BASE: AtomicUsize = AtomicUsize::new(0);
static BUMP_POS: AtomicUsize = AtomicUsize::new(0);
const BUMP_SIZE: usize = 1 << 30;

extern "C" {
    fn mmap(addr: *mut u8, len: usize, prot: i32, flags: i32, fd: i32, off: i64) -> *mut u8;
}

extern "C" {
    fn mprotect(addr: *mut u8, len: usize, prot: i32) -> i32;
}

/// Com o modo bump, torna o heap do host somente-leitura enquanto o código de 16 bits executa: uma
/// escrita errada vira SIGSEGV com o CS:IP do culpado. `on=false` volta a liberar a escrita.
static GUARD: AtomicBool = AtomicBool::new(false);

pub fn enable_guard() {
    GUARD.store(true, Ordering::SeqCst);
}

pub fn protect(on: bool) {
    if !BUMP.load(Ordering::Relaxed) || !GUARD.load(Ordering::Relaxed) {
        return;
    }
    let used = (BUMP_POS.load(Ordering::SeqCst) + 4095) & !4095;
    if used > 0 {
        unsafe { mprotect(BUMP_BASE.load(Ordering::Relaxed) as *mut u8, used, if on { 1 } else { 3 }) };
    }
}

pub fn enable_bump() {
    // PROT_READ|WRITE, MAP_PRIVATE|MAP_ANONYMOUS|MAP_NORESERVE
    let p = unsafe { mmap(std::ptr::null_mut(), BUMP_SIZE, 3, 0x22 | 0x4000, -1, 0) };
    BUMP_BASE.store(p as usize, Ordering::SeqCst);
    BUMP.store(true, Ordering::SeqCst);
}
static LOCK: AtomicBool = AtomicBool::new(false);
// (ponteiro do usuário, tamanho) — tabela aberta com sondagem linear; ponteiro 0 = vazio
static mut TABLE: [(usize, usize); SLOTS] = [(0, 0); SLOTS];
static LIVE: AtomicUsize = AtomicUsize::new(0);

pub fn enable() {
    ENABLED.store(true, Ordering::SeqCst);
}

fn lock() {
    while LOCK.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        std::hint::spin_loop();
    }
}

fn unlock() {
    LOCK.store(false, Ordering::Release);
}

fn slot(p: usize) -> usize {
    (p >> 4).wrapping_mul(0x9E37_79B1) % SLOTS
}

unsafe impl GlobalAlloc for Dbg {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if BUMP.load(Ordering::Relaxed) {
            let align = l.align().max(16);
            let size = (l.size() + align - 1) & !(align - 1);
            let off = BUMP_POS.fetch_add(size + 16, Ordering::SeqCst);
            let p = (BUMP_BASE.load(Ordering::Relaxed) + off + align - 1) & !(align - 1);
            return if off + size + 16 + align < BUMP_SIZE { p as *mut u8 } else { std::ptr::null_mut() };
        }
        if !ENABLED.load(Ordering::Relaxed) {
            return System.alloc(l);
        }
        let align = l.align().max(32);
        let total = PRE + l.size() + POST;
        let raw = System.alloc(Layout::from_size_align_unchecked(total, align));
        if raw.is_null() {
            return raw;
        }
        std::ptr::write_bytes(raw, MAGIC, PRE);
        std::ptr::write_bytes(raw.add(PRE + l.size()), MAGIC, POST);
        let user = raw.add(PRE);
        lock();
        let mut i = slot(user as usize);
        #[allow(static_mut_refs)]
        while TABLE[i].0 != 0 && TABLE[i].0 != usize::MAX {
            i = (i + 1) % SLOTS; // reaproveita lápides
        }
        TABLE[i] = (user as usize, l.size());
        LIVE.fetch_add(1, Ordering::Relaxed);
        unlock();
        user
    }

    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        if BUMP.load(Ordering::Relaxed) {
            return;
        }
        if !ENABLED.load(Ordering::Relaxed) {
            return System.dealloc(p, l);
        }
        lock();
        let mut i = slot(p as usize);
        let mut found = false;
        for _ in 0..SLOTS {
            if TABLE[i].0 == p as usize {
                TABLE[i] = (usize::MAX, 0); // lápide
                found = true;
                break;
            }
            if TABLE[i].0 == 0 {
                break;
            }
            i = (i + 1) % SLOTS;
        }
        unlock();
        if !found {
            // alocada antes de ativar o alocador de depuração
            return System.dealloc(p, l);
        }
        verify(p as usize, l.size(), "free");
        let align = l.align().max(32);
        System.dealloc(p.sub(PRE), Layout::from_size_align_unchecked(PRE + l.size() + POST, align));
    }
}

/// Escreve texto no stderr sem alocar (o alocador de depuração não pode se chamar).
fn raw_print(bytes: &[u8]) {
    extern "C" {
        fn write(fd: i32, buf: *const u8, n: usize) -> isize;
    }
    unsafe { write(2, bytes.as_ptr(), bytes.len()) };
}

fn hex_into(out: &mut [u8; 512], n: &mut usize, mut v: usize) {
    let digits = b"0123456789abcdef";
    let mut buf = [0u8; 8];
    for i in 0..8 {
        buf[7 - i] = digits[v & 0xF];
        v >>= 4;
    }
    for c in buf {
        if *n < out.len() {
            out[*n] = c;
            *n += 1;
        }
    }
}

unsafe fn verify(user: usize, size: usize, when: &str) -> bool {
    let (pre, post) = (user - PRE, user + size);
    let bad_pre = (0..PRE).find(|&k| *((pre + k) as *const u8) != MAGIC);
    let bad_post = (0..POST).find(|&k| *((post + k) as *const u8) != MAGIC);
    if bad_pre.is_none() && bad_post.is_none() {
        return true;
    }
    let mut out = [0u8; 512];
    let mut n = 0;
    let mut put = |s: &[u8], out: &mut [u8; 512], n: &mut usize| {
        for &c in s {
            if *n < out.len() {
                out[*n] = c;
                *n += 1;
            }
        }
    };
    put(b"\n[heapcheck] CORRUPCAO ", &mut out, &mut n);
    put(when.as_bytes(), &mut out, &mut n);
    put(b": bloco ", &mut out, &mut n);
    hex_into(&mut out, &mut n, user);
    put(b" tamanho ", &mut out, &mut n);
    hex_into(&mut out, &mut n, size);
    put(if bad_pre.is_some() { b" ANTES:" } else { b" DEPOIS:" }, &mut out, &mut n);
    let at = if bad_pre.is_some() { pre } else { post };
    for k in 0..24 {
        let b = *((at + k) as *const u8);
        put(&[if (32..127).contains(&b) { b } else { b'.' }], &mut out, &mut n);
    }
    put(b"\n", &mut out, &mut n);
    raw_print(&out[..n]);
    false
}


/// Confere os canários de todas as alocações vivas; devolve quantas estão corrompidas.
pub fn check_all(tag: &str) -> usize {
    if !ENABLED.load(Ordering::Relaxed) {
        return 0;
    }
    let mut bad = 0;
    unsafe {
        lock();
        #[allow(static_mut_refs)]
        for &(p, n) in TABLE.iter() {
            if p != 0 && p != usize::MAX && !verify(p, n, tag) {
                bad += 1;
            }
        }
        unlock();
    }
    bad
}
