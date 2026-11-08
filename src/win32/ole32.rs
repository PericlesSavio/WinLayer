//! ole32.dll — só o que o runtime do VB4 usa na inicialização (IMalloc). O resto são stubs.

use std::ffi::c_void;

pub fn resolve(name: &str) -> Option<usize> {
    exports!(name, CoGetMalloc, OleBuildVersion, CoRegisterClassObject, CoRevokeClassObject).or_else(|| super::stubs::resolve_ole32(name))
}

extern "C" {
    fn malloc(n: usize) -> *mut c_void;
    fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
    fn free(p: *mut c_void);
    fn malloc_usable_size(p: *mut c_void) -> usize;
}

// IMalloc: QueryInterface, AddRef, Release, Alloc, Realloc, Free, GetSize, DidAlloc, HeapMinimize.
extern "stdcall" fn qi(this: u32, _iid: u32, out: *mut u32) -> u32 {
    unsafe { *out = this };
    0
}
extern "stdcall" fn add_ref(_this: u32) -> u32 {
    1
}
extern "stdcall" fn release(_this: u32) -> u32 {
    1
}
extern "stdcall" fn alloc(_this: u32, cb: usize) -> *mut c_void {
    unsafe { malloc(cb.max(1)) }
}
extern "stdcall" fn realloc_(_this: u32, p: *mut c_void, cb: usize) -> *mut c_void {
    unsafe { realloc(p, cb.max(1)) }
}
extern "stdcall" fn free_(_this: u32, p: *mut c_void) {
    unsafe { free(p) }
}
extern "stdcall" fn get_size(_this: u32, p: *mut c_void) -> usize {
    if p.is_null() { usize::MAX } else { unsafe { malloc_usable_size(p) } }
}
extern "stdcall" fn did_alloc(_this: u32, _p: u32) -> i32 {
    1
}
extern "stdcall" fn heap_minimize(_this: u32) {}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn CoGetMalloc(_ctx: u32, out: *mut u32) -> u32 {
    // Objeto COM = ponteiro para tabela de funções; ambos vazam de propósito (vivem até o fim).
    let vtbl: &'static mut [u32; 9] = Box::leak(Box::new([
        qi as usize as u32, add_ref as usize as u32, release as usize as u32, alloc as usize as u32,
        realloc_ as usize as u32, free_ as usize as u32, get_size as usize as u32, did_alloc as usize as u32,
        heap_minimize as usize as u32,
    ]));
    let obj: &'static mut u32 = Box::leak(Box::new(vtbl.as_ptr() as u32));
    *out = obj as *mut u32 as u32;
    0 // S_OK
}

#[allow(non_snake_case)]
extern "stdcall" fn OleBuildVersion() -> u32 {
    std::env::var("WINLAYER_OLEVER").ok().and_then(|v| u32::from_str_radix(&v, 16).ok()).unwrap_or(0x0017_1BD0)
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn CoRegisterClassObject(_clsid: u32, _unk: u32, _ctx: u32, _flags: u32, cookie: *mut u32) -> u32 {
    if !cookie.is_null() {
        *cookie = 1;
    }
    0 // S_OK (a fábrica de classes não é guardada: nada ativa o servidor por fora)
}

#[allow(non_snake_case)]
extern "stdcall" fn CoRevokeClassObject(_cookie: u32) -> u32 {
    0
}
