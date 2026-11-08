//! oleaut32.dll — basic BSTR and VARIANT only (the VB4 runtime imports by ordinal).

pub fn resolve(name: &str) -> Option<usize> {
    exports!(name, SysAllocString, SysReAllocString, SysAllocStringLen, SysReAllocStringLen, SysFreeString, SysStringLen, VariantInit, VariantClear, SysStringByteLen, SysAllocStringByteLen, OaBuildVersion, VariantCopy, VariantCopyInd,
        VariantChangeType, SafeArrayCreate, SafeArrayDestroy, SafeArrayGetDim, SafeArrayGetElemsize,
        SafeArrayGetUBound, SafeArrayGetLBound, SafeArrayLock, SafeArrayUnlock, SafeArrayAccessData,
        SafeArrayUnaccessData, SafeArrayGetElement, SafeArrayPutElement, SafeArrayCopy,
        SafeArrayAllocDescriptor, SafeArrayAllocData, SafeArrayDestroyDescriptor, SafeArrayDestroyData)
}

/// Public oleaut32 ordinals (the ones we implement).
pub fn name_of(ordinal: u32) -> Option<&'static str> {
    Some(match ordinal {
        2 => "SysAllocString",
        3 => "SysReAllocString",
        4 => "SysAllocStringLen",
        5 => "SysReAllocStringLen",
        6 => "SysFreeString",
        7 => "SysStringLen",
        8 => "VariantInit",
        9 => "VariantClear",
        10 => "VariantCopy",
        11 => "VariantCopyInd",
        12 => "VariantChangeType",
        15 => "SafeArrayCreate",
        16 => "SafeArrayDestroy",
        17 => "SafeArrayGetDim",
        18 => "SafeArrayGetElemsize",
        19 => "SafeArrayGetUBound",
        20 => "SafeArrayGetLBound",
        21 => "SafeArrayLock",
        22 => "SafeArrayUnlock",
        23 => "SafeArrayAccessData",
        24 => "SafeArrayUnaccessData",
        25 => "SafeArrayGetElement",
        26 => "SafeArrayPutElement",
        27 => "SafeArrayCopy",
        36 => "SafeArrayAllocDescriptor",
        37 => "SafeArrayAllocData",
        38 => "SafeArrayDestroyDescriptor",
        39 => "SafeArrayDestroyData",
        149 => "SysStringByteLen",
        150 => "SysAllocStringByteLen",
        170 => "OaBuildVersion",
        _ => return None,
    })
}

// BSTR: [u32 size in bytes][UTF-16][NUL]; the pointer points to the first character.
unsafe fn alloc_bstr(src: *const u16, len: u32) -> u32 {
    let bytes = len as usize * 2;
    let block = Box::leak(vec![0u8; 4 + bytes + 2].into_boxed_slice()).as_mut_ptr();
    *(block as *mut u32) = bytes as u32;
    if !src.is_null() {
        std::ptr::copy_nonoverlapping(src as *const u8, block.add(4), bytes);
    }
    block.add(4) as u32
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn SysAllocString(s: *const u16) -> u32 {
    if s.is_null() {
        return 0;
    }
    let mut n = 0;
    while *s.add(n) != 0 { n += 1 }
    alloc_bstr(s, n as u32)
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn SysAllocStringLen(s: *const u16, len: u32) -> u32 {
    alloc_bstr(s, len)
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn SysReAllocString(slot: *mut u32, s: *const u16) -> i32 {
    *slot = SysAllocString(s); // the old block leaks (no free): acceptable for now
    1
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn SysReAllocStringLen(slot: *mut u32, s: *const u16, len: u32) -> i32 {
    *slot = alloc_bstr(s, len);
    1
}

#[allow(non_snake_case)]
extern "stdcall" fn SysFreeString(_b: u32) {}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn SysStringLen(b: u32) -> u32 {
    if b == 0 { 0 } else { *((b - 4) as usize as *const u32) / 2 }
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn VariantInit(v: *mut u8) {
    std::ptr::write_bytes(v, 0, 16); // VT_EMPTY
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn VariantClear(v: *mut u8) -> u32 {
    std::ptr::write_bytes(v, 0, 16);
    0
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn SysStringByteLen(b: u32) -> u32 {
    if b == 0 { 0 } else { *((b - 4) as usize as *const u32) }
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn SysAllocStringByteLen(s: *const u8, bytes: u32) -> u32 {
    let block = Box::leak(vec![0u8; 4 + bytes as usize + 2].into_boxed_slice()).as_mut_ptr();
    *(block as *mut u32) = bytes;
    if !s.is_null() {
        std::ptr::copy_nonoverlapping(s, block.add(4), bytes as usize);
    }
    block.add(4) as u32
}

#[allow(non_snake_case)]
extern "stdcall" fn OaBuildVersion() -> u32 {
    std::env::var("WINLAYER_OAVER").ok().and_then(|v| u32::from_str_radix(&v, 16).ok()).unwrap_or(0x000A_0000 | 0x1BD0)
}

// ---- VARIANT ------------------------------------------------------------------------------
// VARIANT (16 bytes): vt u16 | reserved 6 | data 8 (starting at +8).

const VT_I2: u16 = 2;
const VT_I4: u16 = 3;
const VT_R4: u16 = 4;
const VT_R8: u16 = 5;
const VT_BSTR: u16 = 8;
const VT_BOOL: u16 = 11;
const VT_BYREF: u16 = 0x4000;
const E_TYPEMISMATCH: u32 = 0x8002_0005;

unsafe fn bstr_to_string(b: u32) -> String {
    if b == 0 {
        return String::new();
    }
    let n = SysStringLen(b) as usize;
    let chars: Vec<u16> = (0..n).map(|i| *((b as usize + i * 2) as *const u16)).collect();
    String::from_utf16_lossy(&chars)
}

unsafe fn string_to_bstr(t: &str) -> u32 {
    let w: Vec<u16> = t.encode_utf16().collect();
    alloc_bstr(w.as_ptr(), w.len() as u32)
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn VariantCopy(dst: *mut u8, src: *const u8) -> u32 {
    let vt = *(src as *const u16);
    let data = std::slice::from_raw_parts(src, 16).to_vec();
    std::ptr::copy_nonoverlapping(data.as_ptr(), dst, 16);
    if vt == VT_BSTR {
        *(dst.add(8) as *mut u32) = SysAllocStringByteLen(*(src.add(8) as *const u32) as *const u8, SysStringByteLen(*(src.add(8) as *const u32)));
    }
    0
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn VariantCopyInd(dst: *mut u8, src: *const u8) -> u32 {
    let vt = *(src as *const u16);
    if vt & VT_BYREF == 0 {
        return VariantCopy(dst, src);
    }
    let base = vt & !VT_BYREF;
    let p = *(src.add(8) as *const *const u8);
    *(dst as *mut u16) = base;
    std::ptr::write_bytes(dst.add(2), 0, 6);
    let size = match base { VT_I2 | VT_BOOL => 2, VT_R8 | 6 | 7 => 8, 12 => return E_TYPEMISMATCH, _ => 4 };
    std::ptr::copy_nonoverlapping(p, dst.add(8), size);
    if base == VT_BSTR {
        *(dst.add(8) as *mut u32) = SysAllocStringByteLen(*(p as *const u32) as *const u8, SysStringByteLen(*(p as *const u32)));
    }
    0
}

/// Reads the variant as f64 (numerics and numeric BSTR).
unsafe fn as_f64(v: *const u8) -> Option<f64> {
    let d = v.add(8);
    Some(match *(v as *const u16) {
        VT_I2 | VT_BOOL => *(d as *const i16) as f64,
        VT_I4 => *(d as *const i32) as f64,
        VT_R4 => *(d as *const f32) as f64,
        VT_R8 => *(d as *const f64),
        VT_BSTR => bstr_to_string(*(d as *const u32)).trim().parse().ok()?,
        _ => return None,
    })
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn VariantChangeType(dst: *mut u8, src: *const u8, _flags: u32, vt: u32) -> u32 {
    let to = vt as u16;
    if *(src as *const u16) == to {
        return VariantCopy(dst, src);
    }
    let Some(x) = as_f64(src) else { return E_TYPEMISMATCH };
    let mut out = [0u8; 16];
    out[..2].copy_from_slice(&to.to_le_bytes());
    match to {
        VT_I2 => out[8..10].copy_from_slice(&(x.round() as i16).to_le_bytes()),
        VT_BOOL => out[8..10].copy_from_slice(&(if x != 0.0 { -1i16 } else { 0 }).to_le_bytes()),
        VT_I4 => out[8..12].copy_from_slice(&(x.round() as i32).to_le_bytes()),
        VT_R4 => out[8..12].copy_from_slice(&(x as f32).to_le_bytes()),
        VT_R8 => out[8..16].copy_from_slice(&x.to_le_bytes()),
        VT_BSTR => {
            let text = if x.fract() == 0.0 { format!("{}", x as i64) } else { format!("{x}") };
            out[8..12].copy_from_slice(&string_to_bstr(&text).to_le_bytes());
        }
        _ => return E_TYPEMISMATCH,
    }
    std::ptr::copy_nonoverlapping(out.as_ptr(), dst, 16);
    0
}

// ---- SAFEARRAY ----------------------------------------------------------------------------
// cDims u16 | fFeatures u16 | cbElements u32 | cLocks u32 | pvData u32 | rgsabound[cDims] {cElements u32, lLbound i32}

extern "C" {
    fn calloc(n: usize, size: usize) -> *mut std::ffi::c_void;
}

fn elem_size(vt: u32) -> u32 {
    match vt {
        2 | 11 | 18 => 2,
        5 | 6 | 7 => 8,
        12 => 16,
        16 | 17 => 1,
        _ => 4,
    }
}

unsafe fn dims(psa: *const u8) -> usize {
    *(psa as *const u16) as usize
}

unsafe fn bound(psa: *const u8, i: usize) -> (u32, i32) {
    (*(psa.add(16 + i * 8) as *const u32), *(psa.add(20 + i * 8) as *const i32))
}

unsafe fn total_elements(psa: *const u8) -> usize {
    (0..dims(psa)).map(|i| bound(psa, i).0 as usize).product()
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn SafeArrayAllocDescriptor(cdims: u32, out: *mut u32) -> u32 {
    let psa = calloc(1, 16 + 8 * cdims as usize) as *mut u8;
    *(psa as *mut u16) = cdims as u16;
    *out = psa as u32;
    0
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn SafeArrayAllocData(psa: *mut u8) -> u32 {
    let bytes = total_elements(psa) * *(psa.add(4) as *const u32) as usize;
    *(psa.add(12) as *mut u32) = calloc(1, bytes.max(1)) as u32;
    0
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn SafeArrayCreate(vt: u32, cdims: u32, bounds: *const u32) -> u32 {
    let mut psa = 0u32;
    SafeArrayAllocDescriptor(cdims, &mut psa);
    let p = psa as usize as *mut u8;
    *(p.add(4) as *mut u32) = elem_size(vt);
    std::ptr::copy_nonoverlapping(bounds, p.add(16) as *mut u32, 2 * cdims as usize);
    SafeArrayAllocData(p);
    psa
}

#[allow(non_snake_case)]
extern "stdcall" fn SafeArrayDestroy(_psa: u32) -> u32 { 0 }
#[allow(non_snake_case)]
extern "stdcall" fn SafeArrayDestroyDescriptor(_psa: u32) -> u32 { 0 }
#[allow(non_snake_case)]
extern "stdcall" fn SafeArrayDestroyData(_psa: u32) -> u32 { 0 }

#[allow(non_snake_case)]
unsafe extern "stdcall" fn SafeArrayGetDim(psa: *const u8) -> u32 {
    dims(psa) as u32
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn SafeArrayGetElemsize(psa: *const u8) -> u32 {
    *(psa.add(4) as *const u32)
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn SafeArrayGetUBound(psa: *const u8, dim: u32, out: *mut i32) -> u32 {
    if dim == 0 || dim as usize > dims(psa) {
        return 0x8002_000B; // DISP_E_BADINDEX
    }
    let (n, lb) = bound(psa, dim as usize - 1);
    *out = lb + n as i32 - 1;
    0
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn SafeArrayGetLBound(psa: *const u8, dim: u32, out: *mut i32) -> u32 {
    if dim == 0 || dim as usize > dims(psa) {
        return 0x8002_000B;
    }
    *out = bound(psa, dim as usize - 1).1;
    0
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn SafeArrayLock(psa: *mut u8) -> u32 {
    *(psa.add(8) as *mut u32) += 1;
    0
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn SafeArrayUnlock(psa: *mut u8) -> u32 {
    let l = psa.add(8) as *mut u32;
    *l = (*l).saturating_sub(1);
    0
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn SafeArrayAccessData(psa: *mut u8, out: *mut u32) -> u32 {
    SafeArrayLock(psa);
    *out = *(psa.add(12) as *const u32);
    0
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn SafeArrayUnaccessData(psa: *mut u8) -> u32 {
    SafeArrayUnlock(psa)
}

/// Element address for the given indices (index 0 = leftmost dimension).
unsafe fn element_ptr(psa: *const u8, idx: *const i32) -> Option<*mut u8> {
    let nd = dims(psa);
    let mut off = 0usize;
    for i in 0..nd {
        let (n, lb) = bound(psa, nd - 1 - i);
        let k = *idx.add(i) - lb;
        if k < 0 || k as u32 >= n {
            return None;
        }
        off = off * n as usize + k as usize;
    }
    let size = *(psa.add(4) as *const u32) as usize;
    Some((*(psa.add(12) as *const u32) as usize + off * size) as *mut u8)
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn SafeArrayGetElement(psa: *const u8, idx: *const i32, out: *mut u8) -> u32 {
    match element_ptr(psa, idx) {
        Some(p) => {
            std::ptr::copy_nonoverlapping(p, out, *(psa.add(4) as *const u32) as usize);
            0
        }
        None => 0x8002_000B,
    }
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn SafeArrayPutElement(psa: *const u8, idx: *const i32, src: *const u8) -> u32 {
    match element_ptr(psa, idx) {
        Some(p) => {
            std::ptr::copy_nonoverlapping(src, p, *(psa.add(4) as *const u32) as usize);
            0
        }
        None => 0x8002_000B,
    }
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn SafeArrayCopy(psa: *const u8, out: *mut u32) -> u32 {
    let nd = dims(psa);
    let mut new = 0u32;
    SafeArrayAllocDescriptor(nd as u32, &mut new);
    let q = new as usize as *mut u8;
    std::ptr::copy_nonoverlapping(psa, q, 16 + 8 * nd);
    *(q.add(8) as *mut u32) = 0;
    SafeArrayAllocData(q);
    let bytes = total_elements(psa) * *(psa.add(4) as *const u32) as usize;
    std::ptr::copy_nonoverlapping(*(psa.add(12) as *const u32) as *const u8, *(q.add(12) as *const u32) as *mut u8, bytes);
    *out = new;
    0
}
