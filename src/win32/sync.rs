//! Threads, TLS and synchronization objects (event, mutex) of kernel32.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

extern "C" {
    fn syscall(num: i64, ...) -> i64;
    fn pthread_exit(ret: *mut c_void) -> !;
}

const INFINITE: u32 = u32::MAX;
const WAIT_OBJECT_0: u32 = 0;
const WAIT_TIMEOUT: u32 = 0x102;
const WAIT_FAILED: u32 = u32::MAX;
const CREATE_SUSPENDED: u32 = 4;
const PSEUDO_CURRENT_THREAD: u32 = 0xFFFF_FFFE;
const TLS_SLOTS: usize = 64;

enum Kind {
    Event { manual: bool },
    Mutex,
    Thread,
}

#[derive(Default)]
struct State {
    signaled: bool,
    owner: u32,
    count: u32,
    suspended: bool,
    /// Threads blocked on this event and how many PulseEvent released that have not woken up yet.
    waiters: u32,
    pulses: u32,
}

struct Obj {
    kind: Kind,
    st: Mutex<State>,
    cv: Condvar,
}

static TABLE: Mutex<BTreeMap<u32, Arc<Obj>>> = Mutex::new(BTreeMap::new());
static NEXT: AtomicU32 = AtomicU32::new(0x100);
static NEXT_TLS: AtomicU32 = AtomicU32::new(0);
static START: Mutex<Option<Instant>> = Mutex::new(None);

thread_local! {
    static TLS: RefCell<[u32; TLS_SLOTS]> = const { RefCell::new([0; TLS_SLOTS]) };
    static CURRENT: Cell<u32> = const { Cell::new(0) };
}

pub fn resolve(name: &str) -> Option<usize> {
    exports!(
        name,
        CreateEventA, CreateMutexA, SetEvent, ReleaseMutex, WaitForSingleObject,
        CreateThread, ExitThread, ResumeThread, GetThreadContext, GetCurrentThread,
        GetCurrentThreadId, GetCurrentProcessId, TlsAlloc, TlsFree, TlsGetValue, TlsSetValue,
        GetTickCount, ResetEvent, PulseEvent, WaitForMultipleObjects, SetThreadPriority, SuspendThread, SetPriorityClass,
        QueryPerformanceCounter, QueryPerformanceFrequency,
    )
}

pub fn current_tid() -> u32 {
    tid()
}

fn tid() -> u32 {
    unsafe { syscall(224) as u32 } // gettid (i386)
}

fn add(kind: Kind, st: State) -> u32 {
    let h = NEXT.fetch_add(4, Ordering::Relaxed);
    TABLE.lock().unwrap().insert(h, Arc::new(Obj { kind, st: Mutex::new(st), cv: Condvar::new() }));
    h
}

fn get(h: u32) -> Option<Arc<Obj>> {
    TABLE.lock().unwrap().get(&h).cloned()
}

pub fn close(h: u32) {
    TABLE.lock().unwrap().remove(&h);
}

// ---- events and mutexes -------------------------------------------------------------------

#[allow(non_snake_case)]
extern "stdcall" fn CreateEventA(_attr: u32, manual: i32, initial: i32, _name: u32) -> u32 {
    add(Kind::Event { manual: manual != 0 }, State { signaled: initial != 0, ..Default::default() })
}

#[allow(non_snake_case)]
extern "stdcall" fn CreateMutexA(_attr: u32, owned: i32, _name: u32) -> u32 {
    let st = if owned != 0 { State { owner: tid(), count: 1, ..Default::default() } } else { State { signaled: true, ..Default::default() } };
    add(Kind::Mutex, st)
}

#[allow(non_snake_case)]
extern "stdcall" fn SetEvent(h: u32) -> i32 {
    let Some(o) = get(h) else { return 0 };
    o.st.lock().unwrap().signaled = true;
    o.cv.notify_all();
    1
}

#[allow(non_snake_case)]
extern "stdcall" fn ReleaseMutex(h: u32) -> i32 {
    let Some(o) = get(h) else { return 0 };
    let mut st = o.st.lock().unwrap();
    if st.owner != tid() {
        return 0;
    }
    st.count -= 1;
    if st.count == 0 {
        st.owner = 0;
        st.signaled = true;
        o.cv.notify_one();
    }
    1
}

#[allow(non_snake_case)]
extern "stdcall" fn WaitForSingleObject(h: u32, ms: u32) -> u32 {
    let Some(o) = get(h) else { return WAIT_FAILED };
    let me = tid();
    let deadline = (ms != INFINITE).then(|| Instant::now() + Duration::from_millis(ms as u64));
    let mut st = o.st.lock().unwrap();
    let mut waiting = false;
    loop {
        match o.kind {
            Kind::Event { manual } if st.signaled => {
                if !manual { st.signaled = false }
                if waiting { st.waiters -= 1 }
                return WAIT_OBJECT_0;
            }
            Kind::Event { .. } if waiting && st.pulses > 0 => {
                st.pulses -= 1;
                st.waiters -= 1;
                return WAIT_OBJECT_0;
            }
            Kind::Thread if st.signaled => return WAIT_OBJECT_0,
            Kind::Mutex if st.signaled || st.owner == me => {
                st.signaled = false;
                st.owner = me;
                st.count += 1;
                return WAIT_OBJECT_0;
            }
            _ => {}
        }
        if !waiting && matches!(o.kind, Kind::Event { .. }) {
            waiting = true;
            st.waiters += 1;
        }
        st = match deadline {
            None => o.cv.wait(st).unwrap(),
            Some(d) => {
                let now = Instant::now();
                if now >= d {
                    if waiting { st.waiters -= 1 }
                    return WAIT_TIMEOUT;
                }
                o.cv.wait_timeout(st, d - now).unwrap().0
            }
        };
    }
}

// ---- threads ------------------------------------------------------------------------------

fn finish_current(code: u32) {
    let h = CURRENT.with(|c| c.get());
    if let Some(o) = get(h) {
        let mut st = o.st.lock().unwrap();
        st.signaled = true;
        st.count = code; // stores the exit code
        o.cv.notify_all();
    }
}

#[allow(non_snake_case)]
extern "stdcall" fn CreateThread(_attr: u32, _stack: u32, start: u32, param: u32, flags: u32, id_out: *mut u32) -> u32 {
    let suspended = flags & CREATE_SUSPENDED != 0;
    let h = add(Kind::Thread, State { suspended, ..Default::default() });
    let base = super::kernel32::image_base();
    let spawned = std::thread::Builder::new().spawn(move || {
        if super::teb::install(base).is_err() { return }
        CURRENT.with(|c| c.set(h));
        if let Some(o) = get(h) {
            let mut st = o.st.lock().unwrap();
            while st.suspended { st = o.cv.wait(st).unwrap() }
        }
        let code = unsafe { super::call1(start, param) };
        finish_current(code);
    });
    match spawned {
        Ok(_) => {
            if !id_out.is_null() { unsafe { *id_out = h } }
            h
        }
        Err(_) => 0,
    }
}

#[allow(non_snake_case)]
extern "stdcall" fn ExitThread(code: u32) -> ! {
    if CURRENT.with(|c| c.get()) == 0 {
        super::kernel32::exit_process(code) // main thread
    }
    finish_current(code);
    unsafe { pthread_exit(std::ptr::null_mut()) }
}

#[allow(non_snake_case)]
extern "stdcall" fn ResumeThread(h: u32) -> u32 {
    let Some(o) = get(h) else { return u32::MAX };
    let mut st = o.st.lock().unwrap();
    let was = st.suspended as u32;
    st.suspended = false;
    o.cv.notify_all();
    was
}

#[allow(non_snake_case)]
extern "stdcall" fn GetThreadContext(_h: u32, _ctx: u32) -> i32 {
    0 // not supported
}

#[allow(non_snake_case)]
extern "stdcall" fn GetCurrentThread() -> u32 {
    PSEUDO_CURRENT_THREAD
}

#[allow(non_snake_case)]
extern "stdcall" fn GetCurrentThreadId() -> u32 {
    tid()
}

#[allow(non_snake_case)]
extern "stdcall" fn GetCurrentProcessId() -> u32 {
    std::process::id()
}

#[allow(non_snake_case)]
extern "stdcall" fn GetTickCount() -> u32 {
    // Single clock (virtual in capture mode). Each query advances 1 virtual ms: without this, busy-wait
    // loops ("wait until GetTickCount passes X") never end in capture mode.
    super::user32::tick_virtual(1);
    super::user32::now_ms()
}

// ---- TLS ----------------------------------------------------------------------------------

#[allow(non_snake_case)]
extern "stdcall" fn TlsAlloc() -> u32 {
    let i = NEXT_TLS.fetch_add(1, Ordering::Relaxed);
    if (i as usize) < TLS_SLOTS { i } else { u32::MAX }
}

#[allow(non_snake_case)]
extern "stdcall" fn TlsFree(_i: u32) -> i32 {
    1
}

#[allow(non_snake_case)]
extern "stdcall" fn TlsGetValue(i: u32) -> u32 {
    TLS.with(|t| t.borrow().get(i as usize).copied().unwrap_or(0))
}

#[allow(non_snake_case)]
extern "stdcall" fn TlsSetValue(i: u32, v: u32) -> i32 {
    TLS.with(|t| match t.borrow_mut().get_mut(i as usize) {
        Some(s) => { *s = v; 1 }
        None => 0,
    })
}

#[allow(non_snake_case)]
extern "stdcall" fn ResetEvent(h: u32) -> i32 {
    let Some(o) = get(h) else { return 0 };
    o.st.lock().unwrap().signaled = false;
    1
}

/// Releases whoever is waiting now (all, if the event is manual; one, if automatic) and leaves the event
/// off. Anyone arriving later keeps waiting.
#[allow(non_snake_case)]
extern "stdcall" fn PulseEvent(h: u32) -> i32 {
    let Some(o) = get(h) else { return 0 };
    let mut st = o.st.lock().unwrap();
    st.signaled = false;
    st.pulses = match o.kind {
        Kind::Event { manual: true } => st.waiters,
        _ => st.waiters.min(1),
    };
    o.cv.notify_all();
    1
}

/// Waits on several objects by polling each one (without blocking) until the deadline.
#[allow(non_snake_case)]
unsafe extern "stdcall" fn WaitForMultipleObjects(n: u32, handles: *const u32, all: i32, ms: u32) -> u32 {
    if n == 0 || handles.is_null() {
        return WAIT_FAILED;
    }
    let hs: Vec<u32> = (0..n as usize).map(|i| *handles.add(i)).collect();
    let deadline = (ms != INFINITE).then(|| Instant::now() + Duration::from_millis(ms as u64));
    loop {
        if all != 0 {
            let ready = hs.iter().all(|&h| get(h).is_some_and(|o| { let st = o.st.lock().unwrap(); st.signaled || st.owner == tid() }));
            if ready {
                for &h in &hs {
                    WaitForSingleObject(h, 0);
                }
                return WAIT_OBJECT_0;
            }
        } else {
            for (i, &h) in hs.iter().enumerate() {
                if WaitForSingleObject(h, 0) == WAIT_OBJECT_0 {
                    return WAIT_OBJECT_0 + i as u32;
                }
            }
        }
        if deadline.is_some_and(|d| Instant::now() >= d) {
            return WAIT_TIMEOUT;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[allow(non_snake_case)]
extern "stdcall" fn SetThreadPriority(_h: u32, _prio: i32) -> i32 {
    1
}

#[allow(non_snake_case)]
extern "stdcall" fn SetPriorityClass(_h: u32, _class: u32) -> i32 {
    1
}

/// Suspending a running thread is not supported: returns the previous count (0) without stopping anything.
#[allow(non_snake_case)]
extern "stdcall" fn SuspendThread(_h: u32) -> u32 {
    0
}

/// High-resolution counter in microseconds. In capture mode it follows the virtual clock (and advances
/// a little on each query, like GetTickCount, so busy-wait loops end).
#[allow(non_snake_case)]
unsafe extern "stdcall" fn QueryPerformanceCounter(out: *mut u64) -> i32 {
    if out.is_null() {
        return 0;
    }
    *out = if super::user32::shot_mode() {
        super::user32::tick_virtual(1);
        super::user32::now_ms() as u64 * 1000
    } else {
        START.lock().unwrap().get_or_insert_with(Instant::now).elapsed().as_micros() as u64
    };
    1
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn QueryPerformanceFrequency(out: *mut u64) -> i32 {
    if out.is_null() {
        return 0;
    }
    *out = 1_000_000;
    1
}
