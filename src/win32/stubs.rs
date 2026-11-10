//! Stubs generated in bulk (2026-10-16). They can be edited by hand and replaced by real implementations
//! (when implementing one, remove it from here and from the `resolve_*` list).
//! Functions without a real implementation yet: they return a constant value and clear the `n` arguments
//! (stdcall) so as not to unbalance the stack.

#[allow(non_snake_case)]
pub extern "stdcall" fn CreateFileMappingA(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32, _f: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SetHandleCount(_a: u32) -> u32 {
    32
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetFileTime(_a: u32, _b: u32, _c: u32, _d: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetUserDefaultLangID() -> u32 {
    0x409
}

#[allow(non_snake_case)]
pub extern "stdcall" fn FormatMessageW(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32, _f: u32, _g: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn FlushInstructionCache(_a: u32, _b: u32, _c: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn VirtualQuery(_a: u32, _b: u32, _c: u32) -> u32 {
    0 // real implementation in kernel32y.rs
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GlobalSize(_a: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetProfileStringA(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SearchPathA(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32, _f: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GlobalHandle(_a: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetTempFileNameA(_a: u32, _b: u32, _c: u32, _d: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetVolumeInformationA(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32, _f: u32, _g: u32, _h: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetShortPathNameA(_a: u32, _b: u32, _c: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn IsDBCSLeadByte(_a: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn IsValidLocale(_a: u32, _b: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn PeekNamedPipe(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32, _f: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn HeapCompact(_a: u32, _b: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn HeapLock(_a: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn HeapUnlock(_a: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn HeapWalk(_a: u32, _b: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetLocaleInfoA(_a: u32, _b: u32, _c: u32, _d: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetLocaleInfoW(_a: u32, _b: u32, _c: u32, _d: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn lstrcmpiW(_a: u32, _b: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn FreeEnvironmentStringsA(_a: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn FreeEnvironmentStringsW(_a: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn DisableThreadLibraryCalls(_module: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetProfileIntA(_app: u32, _key: u32, default: u32) -> u32 {
    default
}

#[allow(non_snake_case)]
pub extern "stdcall" fn LoadModule(_name: u32, _params: u32) -> u32 {
    2 // ERROR_FILE_NOT_FOUND (values < 32 are errors)
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn OutputDebugStringA(s: *const i8) {
    if !s.is_null() && std::env::var_os("WINLAYER_TRACE").is_some() {
        eprintln!("[debug] {}", std::ffi::CStr::from_ptr(s).to_string_lossy().trim_end());
    }
}

#[allow(non_snake_case)]
pub extern "stdcall" fn TerminateProcess(process: u32, code: u32) -> u32 {
    if process == 0xFFFF_FFFF {
        super::kernel32::exit_process(code);
    }
    0
}

unsafe fn cstr_at(p: *const i8) -> String {
    if p.is_null() { String::new() } else { std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned() }
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn CopyFileA(src: *const i8, dst: *const i8, fail_if_exists: i32) -> i32 {
    let (s, d) = (super::files::host_path(&cstr_at(src)), super::files::host_path(&cstr_at(dst)));
    if fail_if_exists != 0 && d.exists() {
        return 0;
    }
    std::fs::copy(s, d).is_ok() as i32
}

#[allow(non_snake_case)]
pub extern "stdcall" fn EnumSystemLocalesA(cb: u32, _flags: u32) -> i32 {
    if cb != 0 {
        unsafe { super::call1(cb, c"00000409".as_ptr() as u32) };
    }
    1
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn FatalAppExitA(_action: u32, msg: *const i8) {
    eprintln!("[winlayer] FatalAppExit: {}", cstr_at(msg));
    super::kernel32::exit_process(1)
}

/// FILETIME (100 ns since 1601) -> DOS date/time.
#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn FileTimeToDosDateTime(ft: *const u64, date: *mut u16, time: *mut u16) -> i32 {
    if ft.is_null() {
        return 0;
    }
    let secs = (std::ptr::read_unaligned(ft) / 10_000_000).saturating_sub(11_644_473_600) as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // days since 1970-01-01 -> year/month/day (H. Hinnant's civil algorithm)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + (m <= 2) as i64;
    if !date.is_null() {
        *date = ((((y - 1980).max(0) as u16) << 9) | ((m as u16) << 5) | d as u16) as u16;
    }
    if !time.is_null() {
        *time = (((rem / 3600) as u16) << 11) | ((((rem / 60) % 60) as u16) << 5) | ((rem % 60) / 2) as u16;
    }
    1
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn GetSystemTimeAsFileTime(ft: *mut u64) {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    std::ptr::write_unaligned(ft, (now.as_secs() + 11_644_473_600) * 10_000_000 + now.subsec_nanos() as u64 / 100);
}

/// MEMORYSTATUS: values of a 1999 machine with headroom (256 MB of RAM, 2 GB of virtual space).
#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn GlobalMemoryStatus(ms: *mut u32) {
    if ms.is_null() {
        return;
    }
    let v = [32u32, 10, 256 << 20, 200 << 20, 512 << 20, 400 << 20, 0x7FFE_0000, 0x7000_0000];
    for (i, x) in v.iter().enumerate() {
        *ms.add(i) = *x;
    }
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn InterlockedIncrement(p: *mut i32) -> i32 {
    std::sync::atomic::AtomicI32::from_ptr(p).fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn InterlockedDecrement(p: *mut i32) -> i32 {
    std::sync::atomic::AtomicI32::from_ptr(p).fetch_sub(1, std::sync::atomic::Ordering::SeqCst) - 1
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn lstrcpynA(dst: *mut u8, src: *const u8, n: i32) -> *mut u8 {
    if dst.is_null() || n <= 0 {
        return dst;
    }
    let mut i = 0;
    while i < n as usize - 1 && !src.is_null() && *src.add(i) != 0 {
        *dst.add(i) = *src.add(i);
        i += 1;
    }
    *dst.add(i) = 0;
    dst
}

/// OpenFile (16-bit API still present in Win32): OF_EXIST, OF_DELETE and opening via CreateFileA.
#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn OpenFile(name: *const i8, of: *mut u8, style: u32) -> i32 {
    const OF_EXIST: u32 = 0x4000;
    const OF_DELETE: u32 = 0x200;
    const OF_CREATE: u32 = 0x1000;
    let path = cstr_at(name);
    let host = super::files::host_path(&path);
    if !of.is_null() {
        std::ptr::write_bytes(of, 0, 136);
        *of = 136;
        let b = path.as_bytes();
        let n = b.len().min(127);
        std::ptr::copy_nonoverlapping(b.as_ptr(), of.add(8), n);
    }
    if style & OF_EXIST != 0 {
        return if host.is_file() { 1 } else { -1 };
    }
    if style & OF_DELETE != 0 {
        return if std::fs::remove_file(host).is_ok() { 1 } else { -1 };
    }
    // OF_READ=0, OF_WRITE=1, OF_READWRITE=2
    let access = match style & 3 { 0 => 0x8000_0000, 1 => 0x4000_0000, _ => 0xC000_0000 };
    let disp = if style & OF_CREATE != 0 { 2 } else { 3 }; // CREATE_ALWAYS / OPEN_EXISTING
    let h = super::files::CreateFileA(name, access, 3, 0, disp, 0, 0);
    if h == u32::MAX { -1 } else { h as i32 }
}

#[allow(non_snake_case)]
pub extern "stdcall" fn VirtualLock(_p: u32, _n: u32) -> i32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn VirtualUnlock(_p: u32, _n: u32) -> i32 {
    1
}

pub fn resolve_kernel32(name: &str) -> Option<usize> {
    exports!(name, _lopen, _hread, CompareFileTime, GetDateFormatA, GetTimeFormatA, GetLogicalDriveStringsA, InterlockedExchange, OpenMutexA, SetVolumeLabelA, CopyFileA, EnumSystemLocalesA, FatalAppExitA, FileTimeToDosDateTime, GetSystemTimeAsFileTime, GlobalMemoryStatus, InterlockedIncrement, InterlockedDecrement, lstrcpynA, OpenFile, VirtualLock, VirtualUnlock, DisableThreadLibraryCalls, GetProfileIntA, LoadModule, OutputDebugStringA, TerminateProcess, CreateFileMappingA, SetHandleCount, GetFileTime, GetUserDefaultLangID, FormatMessageW, FlushInstructionCache, VirtualQuery, GlobalSize, GetProfileStringA, SearchPathA, GlobalHandle, GetTempFileNameA, GetVolumeInformationA, GetShortPathNameA, IsDBCSLeadByte, IsValidLocale, PeekNamedPipe, HeapCompact, HeapLock, HeapUnlock, HeapWalk, GetLocaleInfoA, GetLocaleInfoW, lstrcmpiW, FreeEnvironmentStringsA, FreeEnvironmentStringsW)
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetMessagePos() -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetMessageTime() -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SetMenuDefaultItem(_a: u32, _b: u32, _c: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetDoubleClickTime() -> u32 {
    500
}

#[allow(non_snake_case)]
pub extern "stdcall" fn TrackPopupMenu(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32, _f: u32, _g: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn DrawFocusRect(_a: u32, _b: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetCaretPos(_a: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SetCaretPos(_a: u32, _b: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn DestroyIcon(_a: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn IsZoomed(_a: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn DrawMenuBar(_a: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn DefMDIChildProcA(_a: u32, _b: u32, _c: u32, _d: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn FindWindowW(_a: u32, _b: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn LoadStringW(_a: u32, _b: u32, _c: u32, _d: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SetWindowsHookExW(_a: u32, _b: u32, _c: u32, _d: u32) -> u32 {
    0x5001
}

#[allow(non_snake_case)]
pub extern "stdcall" fn VkKeyScanW(_a: u32) -> u32 {
    0xFFFF_FFFF
}

#[allow(non_snake_case)]
pub extern "stdcall" fn VkKeyScanA(_a: u32) -> u32 {
    0xFFFF_FFFF
}

#[allow(non_snake_case)]
pub extern "stdcall" fn keybd_event(_a: u32, _b: u32, _c: u32, _d: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SetKeyboardState(_a: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetForegroundWindow() -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn DeferWindowPos(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32, _f: u32, _g: u32, _h: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn EndDeferWindowPos(_a: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn BeginDeferWindowPos(_a: u32) -> u32 {
    0x5002
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetSystemMenu(_a: u32, _b: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn WaitForInputIdle(_a: u32, _b: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn CloseClipboard() -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn EmptyClipboard() -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn OpenClipboard(_a: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SetScrollRange(_a: u32, _b: u32, _c: u32, _d: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn IsClipboardFormatAvailable(_a: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn MapWindowPoints(_a: u32, _b: u32, _c: u32, _d: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn DdeConnect(_a: u32, _b: u32, _c: u32, _d: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn DdeFreeStringHandle(_a: u32, _b: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn DdeFreeDataHandle(_a: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn DdeCreateStringHandleA(_a: u32, _b: u32, _c: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn DdeNameService(_a: u32, _b: u32, _c: u32, _d: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn DdeQueryStringA(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn DdeQueryConvInfo(_a: u32, _b: u32, _c: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn DdeUninitialize(_a: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn DdeInitializeA(_a: u32, _b: u32, _c: u32, _d: u32) -> u32 {
    0x4003
}

#[allow(non_snake_case)]
pub extern "stdcall" fn DdeSetUserHandle(_a: u32, _b: u32, _c: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn DdeClientTransaction(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32, _f: u32, _g: u32, _h: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn DdeAbandonTransaction(_a: u32, _b: u32, _c: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn DdePostAdvise(_a: u32, _b: u32, _c: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn DdeCreateDataHandle(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32, _f: u32, _g: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn DdeGetData(_a: u32, _b: u32, _c: u32, _d: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn DdeGetLastError(_a: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn DdeDisconnect(_a: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn UnregisterClassA(_a: u32, _b: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SetWindowWord(_a: u32, _b: u32, _c: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetClassLongA(_a: u32, _b: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetAsyncKeyState(_a: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetTabbedTextExtentA(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn CreateIcon(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32, _f: u32, _g: u32) -> u32 {
    0x7002
}

#[allow(non_snake_case)]
pub extern "stdcall" fn CreateCursor(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32, _f: u32, _g: u32) -> u32 {
    0x7001
}

#[allow(non_snake_case)]
pub extern "stdcall" fn EnumClipboardFormats(_a: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn DestroyCursor(_a: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetCursor() -> u32 {
    0x7001
}

#[allow(non_snake_case)]
pub extern "stdcall" fn DrawIcon(_a: u32, _b: u32, _c: u32, _d: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn TranslateMDISysAccel(_a: u32, _b: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn RegisterClipboardFormatA(_a: u32) -> u32 {
    0xC100
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetClipboardFormatNameA(_a: u32, _b: u32, _c: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn PostThreadMessageA(_a: u32, _b: u32, _c: u32, _d: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn RegisterWindowMessageA(_a: u32) -> u32 {
    0xC200
}

#[allow(non_snake_case)]
pub extern "stdcall" fn AttachThreadInput(_a: u32, _b: u32, _c: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetUpdateRgn(_a: u32, _b: u32, _c: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn WindowFromPoint(_a: u32, _b: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetScrollPos(_a: u32, _b: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SetScrollPos(_a: u32, _b: u32, _c: u32, _d: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SetCursorPos(_a: u32, _b: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetClipboardData(_a: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SetClipboardData(_a: u32, _b: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SetForegroundWindow(_a: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetMenuState(_a: u32, _b: u32, _c: u32) -> u32 {
    0xFFFF_FFFF
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetMenuStringA(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn DeleteMenu(_a: u32, _b: u32, _c: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn DefFrameProcA(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn RemoveMenu(_a: u32, _b: u32, _c: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetActiveWindow() -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn InsertMenuA(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SetWindowContextHelpId(_a: u32, _b: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn CopyImage(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn CopyIcon(_a: u32) -> u32 {
    _a
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetIconInfo(_a: u32, _b: u32) -> u32 {
    0
}

/// SystemParametersInfoA: only the queries games make at startup.
#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn SystemParametersInfoA(action: u32, _ui: u32, pv: *mut u32, _ini: u32) -> u32 {
    if pv.is_null() {
        return 1;
    }
    match action {
        0x30 => {
            // SPI_GETWORKAREA: the whole screen
            *pv = 0;
            *pv.add(1) = 0;
            *pv.add(2) = super::user32::desktop_w() as u32;
            *pv.add(3) = super::user32::desktop_h() as u32;
        }
        0x10 | 0x0E | 0x72 => *pv = 0, // SCREENSAVEACTIVE, GETSCREENSAVETIMEOUT, SCREENSAVERRUNNING
        _ => {}
    }
    1
}

pub fn resolve_user32(name: &str) -> Option<usize> {
    exports!(name, GetKeyboardLayout, GetKeyboardLayoutList, SystemParametersInfoA, GetMessagePos, GetMessageTime, SetMenuDefaultItem, GetDoubleClickTime, TrackPopupMenu, DrawFocusRect, GetCaretPos, SetCaretPos, DestroyIcon, IsZoomed, DrawMenuBar, DefMDIChildProcA, FindWindowW, LoadStringW, SetWindowsHookExW, VkKeyScanW, VkKeyScanA, keybd_event, SetKeyboardState, GetForegroundWindow, DeferWindowPos, EndDeferWindowPos, BeginDeferWindowPos, GetSystemMenu, WaitForInputIdle, CloseClipboard, EmptyClipboard, OpenClipboard, SetScrollRange, IsClipboardFormatAvailable, MapWindowPoints, DdeConnect, DdeFreeStringHandle, DdeFreeDataHandle, DdeCreateStringHandleA, DdeNameService, DdeQueryStringA, DdeQueryConvInfo, DdeUninitialize, DdeInitializeA, DdeSetUserHandle, DdeClientTransaction, DdeAbandonTransaction, DdePostAdvise, DdeCreateDataHandle, DdeGetData, DdeGetLastError, DdeDisconnect, UnregisterClassA, SetWindowWord, GetClassLongA, GetAsyncKeyState, GetTabbedTextExtentA, CreateIcon, CreateCursor, EnumClipboardFormats, DestroyCursor, GetCursor, DrawIcon, TranslateMDISysAccel, RegisterClipboardFormatA, GetClipboardFormatNameA, PostThreadMessageA, RegisterWindowMessageA, AttachThreadInput, GetUpdateRgn, WindowFromPoint, GetScrollPos, SetScrollPos, SetCursorPos, GetClipboardData, SetClipboardData, SetForegroundWindow, GetMenuState, GetMenuStringA, DeleteMenu, DefFrameProcA, RemoveMenu, GetActiveWindow, InsertMenuA, SetWindowContextHelpId, CopyImage, CopyIcon, GetIconInfo)
}

#[allow(non_snake_case)]
pub extern "stdcall" fn OffsetWindowOrgEx(_a: u32, _b: u32, _c: u32, _d: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn UnrealizeObject(_a: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SetBrushOrgEx(_a: u32, _b: u32, _c: u32, _d: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetViewportExtEx(_a: u32, _b: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SetROP2(_a: u32, _b: u32) -> u32 {
    13
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetROP2(_a: u32) -> u32 {
    13
}

#[allow(non_snake_case)]
pub extern "stdcall" fn IntersectClipRect(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32) -> u32 {
    2
}

#[allow(non_snake_case)]
pub extern "stdcall" fn Escape(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn CombineRgn(_a: u32, _b: u32, _c: u32, _d: u32) -> u32 {
    2
}

#[allow(non_snake_case)]
pub extern "stdcall" fn ExcludeClipRect(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32) -> u32 {
    2
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SetRectRgn(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetWindowExtEx(_a: u32, _b: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SetWindowOrgEx(_a: u32, _b: u32, _c: u32, _d: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn CreateRectRgn(_a: u32, _b: u32, _c: u32, _d: u32) -> u32 {
    0x8001
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SetAbortProc(_a: u32, _b: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn EnumFontsA(_a: u32, _b: u32, _c: u32, _d: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn Arc(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32, _f: u32, _g: u32, _h: u32, _i: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SetStretchBltMode(_a: u32, _b: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn Pie(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32, _f: u32, _g: u32, _h: u32, _i: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetBitmapBits(_a: u32, _b: u32, _c: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn CopyEnhMetaFileA(_a: u32, _b: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn CopyMetaFileA(_a: u32, _b: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetDIBits(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32, _f: u32, _g: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn CloseMetaFile(_a: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn CreateMetaFileA(_a: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn DeleteMetaFile(_a: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SelectClipRgn(_a: u32, _b: u32) -> u32 {
    2
}

#[allow(non_snake_case)]
pub extern "stdcall" fn CreateRectRgnIndirect(_a: u32) -> u32 {
    0x8001
}

#[allow(non_snake_case)]
pub extern "stdcall" fn PlayMetaFile(_a: u32, _b: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn EndDoc(_a: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn StartPage(_a: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn EndPage(_a: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn AbortDoc(_a: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn ResetDCA(_a: u32, _b: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn StartDocA(_a: u32, _b: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetNearestColor(_a: u32, _b: u32) -> u32 {
    _b
}

#[allow(non_snake_case)]
pub extern "stdcall" fn DeleteEnhMetaFile(_a: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SetMetaFileBitsEx(_a: u32, _b: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetEnhMetaFileHeader(_a: u32, _b: u32, _c: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetBitmapDimensionEx(_a: u32, _b: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SetEnhMetaFileBits(_a: u32, _b: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn PlayMetaFileRecord(_a: u32, _b: u32, _c: u32, _d: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetWinMetaFileBits(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetWindowOrgEx(_a: u32, _b: u32) -> u32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn CreateHalftonePalette(_a: u32) -> u32 {
    0x8002
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetEnhMetaFileBits(_a: u32, _b: u32, _c: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetMetaFileBitsEx(_a: u32, _b: u32, _c: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SetDIBits(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32, _f: u32, _g: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SetBitmapBits(_a: u32, _b: u32, _c: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn PlayEnhMetaFile(_a: u32, _b: u32, _c: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn EnumMetaFile(_a: u32, _b: u32, _c: u32, _d: u32) -> u32 {
    0
}

/// System palette: the 20 fixed Windows colors at the ends and black in the middle.
#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn GetSystemPaletteEntries(_hdc: u32, start: u32, n: u32, out: *mut u32) -> u32 {
    const LOW: [u32; 10] = [0x000000, 0x000080, 0x008000, 0x008080, 0x800000, 0x800080, 0x808000, 0xC0C0C0, 0xC0DCC0, 0xF0CAA6];
    const HIGH: [u32; 10] = [0xF0FBFF, 0xA4A0A0, 0x808080, 0x0000FF, 0x00FF00, 0x00FFFF, 0xFF0000, 0xFF00FF, 0xFFFF00, 0xFFFFFF];
    if out.is_null() {
        return 256;
    }
    for i in 0..n.min(256u32.saturating_sub(start)) {
        let idx = (start + i) as usize;
        // PALETTEENTRY = R, G, B, flags; the constants above are COLORREF (0x00BBGGRR)
        *out.add(i as usize) = if idx < 10 { LOW[idx] } else if idx >= 246 { HIGH[idx - 246] } else { 0 };
    }
    n
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetSystemPaletteUse(_hdc: u32) -> u32 {
    1 // SYSPAL_STATIC
}

pub fn resolve_gdi32(name: &str) -> Option<usize> {
    exports!(name, GetLayout, SetLayout, SetSystemPaletteUse, GetSystemPaletteEntries, GetSystemPaletteUse, OffsetWindowOrgEx, UnrealizeObject, SetBrushOrgEx, GetViewportExtEx, SetROP2, GetROP2, IntersectClipRect, Escape, CombineRgn, ExcludeClipRect, SetRectRgn, GetWindowExtEx, SetWindowOrgEx, CreateRectRgn, SetAbortProc, EnumFontsA, Arc, SetStretchBltMode, Pie, GetBitmapBits, CopyEnhMetaFileA, CopyMetaFileA, GetDIBits, CloseMetaFile, CreateMetaFileA, DeleteMetaFile, SelectClipRgn, CreateRectRgnIndirect, PlayMetaFile, EndDoc, StartPage, EndPage, AbortDoc, ResetDCA, StartDocA, GetNearestColor, DeleteEnhMetaFile, SetMetaFileBitsEx, GetEnhMetaFileHeader, GetBitmapDimensionEx, SetEnhMetaFileBits, PlayMetaFileRecord, GetWinMetaFileBits, GetWindowOrgEx, CreateHalftonePalette, GetEnhMetaFileBits, GetMetaFileBitsEx, SetDIBits, SetBitmapBits, PlayEnhMetaFile, EnumMetaFile)
}

#[allow(non_snake_case)]
pub extern "stdcall" fn RegQueryValueExW(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32, _f: u32) -> u32 {
    2
}

#[allow(non_snake_case)]
pub extern "stdcall" fn RegDeleteValueW(_a: u32, _b: u32) -> u32 {
    2
}

#[allow(non_snake_case)]
pub extern "stdcall" fn RegQueryInfoKeyA(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32, _f: u32, _g: u32, _h: u32, _i: u32, _j: u32, _k: u32, _l: u32) -> u32 {
    2
}

#[allow(non_snake_case)]
pub extern "stdcall" fn OpenProcessToken(_a: u32, _b: u32, _c: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn LookupPrivilegeValueA(_a: u32, _b: u32, _c: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn AdjustTokenPrivileges(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32, _f: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn RegCreateKeyW(_a: u32, _b: u32, _c: u32) -> u32 {
    2
}

#[allow(non_snake_case)]
pub extern "stdcall" fn RegQueryValueW(_a: u32, _b: u32, _c: u32, _d: u32) -> u32 {
    2
}

#[allow(non_snake_case)]
pub extern "stdcall" fn RegEnumValueA(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32, _f: u32, _g: u32, _h: u32) -> u32 {
    259
}

#[allow(non_snake_case)]
pub extern "stdcall" fn RegEnumValueW(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32, _f: u32, _g: u32, _h: u32) -> u32 {
    259
}

#[allow(non_snake_case)]
pub extern "stdcall" fn RegEnumKeyW(_a: u32, _b: u32, _c: u32, _d: u32) -> u32 {
    259
}

#[allow(non_snake_case)]
pub extern "stdcall" fn RegDeleteValueA(_a: u32, _b: u32) -> u32 {
    2
}

#[allow(non_snake_case)]
pub extern "stdcall" fn RegOpenKeyA(_a: u32, _b: u32, _c: u32) -> u32 {
    2
}

#[allow(non_snake_case)]
pub extern "stdcall" fn RegDeleteKeyW(_a: u32, _b: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn RegOpenKeyW(_a: u32, _b: u32, _c: u32) -> u32 {
    2
}

#[allow(non_snake_case)]
pub extern "stdcall" fn RegSetValueExW(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32, _f: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn RegSetValueW(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn RegEnumKeyA(_a: u32, _b: u32, _c: u32, _d: u32) -> u32 {
    259
}

#[allow(non_snake_case)]
pub extern "stdcall" fn RegCreateKeyA(_a: u32, _b: u32, _c: u32) -> u32 {
    2
}

#[allow(non_snake_case)]
pub extern "stdcall" fn RegSetValueA(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn RegQueryValueA(_a: u32, _b: u32, _c: u32, _d: u32) -> u32 {
    2
}

pub fn resolve_advapi32(name: &str) -> Option<usize> {
    exports!(name, GetUserNameA, RegQueryValueExW, RegDeleteValueW, RegQueryInfoKeyA, OpenProcessToken, LookupPrivilegeValueA, AdjustTokenPrivileges, RegCreateKeyW, RegQueryValueW, RegEnumValueA, RegEnumValueW, RegEnumKeyW, RegDeleteValueA, RegOpenKeyA, RegDeleteKeyW, RegOpenKeyW, RegSetValueExW, RegSetValueW, RegEnumKeyA, RegCreateKeyA, RegSetValueA, RegQueryValueA)
}

#[allow(non_snake_case)]
pub extern "stdcall" fn OleCreateFromFile(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32, _f: u32, _g: u32, _h: u32, _i: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn OleQueryCreateFromData(_a: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn ReadClassStm(_a: u32, _b: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn OleSaveToStream(_a: u32, _b: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn CoIsOle1Class(_a: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn OleDoAutoConvert(_a: u32, _b: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn OleLoad(_a: u32, _b: u32, _c: u32, _d: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn OleSave(_a: u32, _b: u32, _c: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn OleQueryLinkFromData(_a: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn MkParseDisplayName(_a: u32, _b: u32, _c: u32, _d: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn CreateILockBytesOnHGlobal(_a: u32, _b: u32, _c: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn OleDuplicateData(_a: u32, _b: u32, _c: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn CoGetMalloc(_a: u32, _b: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn RevokeDragDrop(_a: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn RegisterDragDrop(_a: u32, _b: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn BindMoniker(_a: u32, _b: u32, _c: u32, _d: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn StgOpenStorageOnILockBytes(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32, _f: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn StgCreateDocfileOnILockBytes(_a: u32, _b: u32, _c: u32, _d: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn OleCreateFromData(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32, _f: u32, _g: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn OleCreateLinkFromData(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32, _f: u32, _g: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn OleGetIconOfClass(_a: u32, _b: u32, _c: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn OleGetClipboard(_a: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn OleSetClipboard(_a: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn OleCreateLink(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32, _f: u32, _g: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn OleCreateLinkToFile(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32, _f: u32, _g: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetClassFile(_a: u32, _b: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn CLSIDFromProgID(_a: u32, _b: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn OleGetAutoConvert(_a: u32, _b: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn IsAccelerator(_a: u32, _b: u32, _c: u32, _d: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
/// Only the classes winlayer implements (DirectPlay Lobby); the rest is "class not registered".
pub unsafe extern "stdcall" fn CoCreateInstance(clsid: u32, _outer: u32, _ctx: u32, _iid: u32, out: *mut u32) -> u32 {
    const REGDB_E_CLASSNOTREG: u32 = 0x8004_0154;
    let first = if clsid == 0 { 0 } else { std::ptr::read_unaligned(clsid as usize as *const u32) };
    if !out.is_null() {
        match first {
            super::dplay::CLSID_DIRECTPLAYLOBBY => {
                *out = super::dplay::create_lobby();
                return 0;
            }
            super::dplay::CLSID_DIRECTPLAY => {
                *out = super::dplay::create_dplay();
                return 0;
            }
            _ => {}
        }
    }
    if !out.is_null() {
        *out = 0;
    }
    if std::env::var_os("WINLAYER_TRACE").is_some() {
        eprintln!("[ole32] CoCreateInstance: classe {first:#010x} not registered");
    }
    REGDB_E_CLASSNOTREG
}

#[allow(non_snake_case)]
pub extern "stdcall" fn OleSetMenuDescriptor(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn OleConvertIStorageToOLESTREAM(_a: u32, _b: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn OleConvertOLESTREAMToIStorage(_a: u32, _b: u32, _c: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn ReadClassStg(_a: u32, _b: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn WriteClassStg(_a: u32, _b: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn OleRun(_a: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn CreateBindCtx(_a: u32, _b: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn OleLockRunning(_a: u32, _b: u32, _c: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn OleIsCurrentClipboard(_a: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn ReleaseStgMedium(_a: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn OleIsRunning(_a: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn StgCreateDocfile(_a: u32, _b: u32, _c: u32, _d: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn CoRegisterClassObject(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn CoRevokeClassObject(_a: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn CLSIDFromString(_a: u32, _b: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn ProgIDFromCLSID(_a: u32, _b: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn StringFromCLSID(_a: u32, _b: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn StringFromGUID2(_a: u32, _b: u32, _c: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn CoGetClassObject(_a: u32, _b: u32, _c: u32, _d: u32, _e: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn IsValidInterface(_a: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn CoDisconnectObject(_a: u32, _b: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn CreateStreamOnHGlobal(_a: u32, _b: u32, _c: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn StgIsStorageILockBytes(_a: u32) -> u32 {
    0x8000_4005
}

#[allow(non_snake_case)]
pub extern "stdcall" fn OleInitialize(_a: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn OleUninitialize() -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn OleBuildVersion() -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn CoRegisterMessageFilter(_a: u32, _b: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn CoFreeUnusedLibraries() -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn CoLockObjectExternal(_a: u32, _b: u32, _c: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn OleFlushClipboard() -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn CoInitialize(_reserved: u32) -> u32 {
    0 // S_OK
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn GetUserNameA(buf: *mut u8, size: *mut u32) -> i32 {
    let name = b"Player\0";
    if size.is_null() {
        return 0;
    }
    if buf.is_null() || (*size as usize) < name.len() {
        *size = name.len() as u32;
        return 0;
    }
    std::ptr::copy_nonoverlapping(name.as_ptr(), buf, name.len());
    *size = name.len() as u32;
    1
}

/// System tray icon: there is no tray, pretends it succeeded.
#[allow(non_snake_case)]
pub extern "stdcall" fn Shell_NotifyIconA(_msg: u32, _data: u32) -> i32 {
    1
}

pub fn resolve_shell32(name: &str) -> Option<usize> {
    exports!(name, Shell_NotifyIconA, ShellExecuteA, ShellAboutA)
}

pub fn resolve_ole32(name: &str) -> Option<usize> {
    exports!(name, CoUninitialize, CoInitialize, OleCreateFromFile, OleQueryCreateFromData, ReadClassStm, OleSaveToStream, CoIsOle1Class, OleDoAutoConvert, OleLoad, OleSave, OleQueryLinkFromData, MkParseDisplayName, CreateILockBytesOnHGlobal, OleDuplicateData, CoGetMalloc, RevokeDragDrop, RegisterDragDrop, BindMoniker, StgOpenStorageOnILockBytes, StgCreateDocfileOnILockBytes, OleCreateFromData, OleCreateLinkFromData, OleGetIconOfClass, OleGetClipboard, OleSetClipboard, OleCreateLink, OleCreateLinkToFile, GetClassFile, CLSIDFromProgID, OleGetAutoConvert, IsAccelerator, CoCreateInstance, OleSetMenuDescriptor, OleConvertIStorageToOLESTREAM, OleConvertOLESTREAMToIStorage, ReadClassStg, WriteClassStg, OleRun, CreateBindCtx, OleLockRunning, OleIsCurrentClipboard, ReleaseStgMedium, OleIsRunning, StgCreateDocfile, CoRegisterClassObject, CoRevokeClassObject, CLSIDFromString, ProgIDFromCLSID, StringFromCLSID, StringFromGUID2, CoGetClassObject, IsValidInterface, CoDisconnectObject, CreateStreamOnHGlobal, StgIsStorageILockBytes, OleInitialize, OleUninitialize, OleBuildVersion, CoRegisterMessageFilter, CoFreeUnusedLibraries, CoLockObjectExternal, OleFlushClipboard)
}

#[allow(non_snake_case)]
pub extern "stdcall" fn CreatePropertySheetPageA(_a: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn PropertySheetA(_a: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn InitCommonControls() -> u32 {
    0
}

pub fn resolve_comctl32(name: &str) -> Option<usize> {
    exports!(name, InitCommonControlsEx, CreatePropertySheetPageA, PropertySheetA, InitCommonControls)
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn CompareFileTime(a: *const u64, b: *const u64) -> i32 {
    match std::ptr::read_unaligned(a).cmp(&std::ptr::read_unaligned(b)) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

/// Copies `text` + NUL; with n == 0 only reports the required size. Returns the size including the NUL.
unsafe fn fill_text(buf: *mut u8, n: i32, text: &str) -> i32 {
    let need = text.len() as i32 + 1;
    if n == 0 {
        return need;
    }
    if buf.is_null() || n < need {
        return 0;
    }
    std::ptr::copy_nonoverlapping(text.as_ptr(), buf, text.len());
    *buf.add(text.len()) = 0;
    need
}

/// Given SYSTEMTIME, or the current time: year, month, day of week, day, hour, minute, second, ms.
unsafe fn system_time(st: *const u16) -> [u16; 8] {
    let mut now = [0u16; 8];
    if st.is_null() {
        super::kernel32::get_local_time(now.as_mut_ptr());
    } else {
        std::ptr::copy_nonoverlapping(st, now.as_mut_ptr(), 8);
    }
    now
}

/// The requested format is ignored: always MM/dd/yyyy.
#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn GetDateFormatA(_lcid: u32, _flags: u32, st: *const u16, _fmt: u32, buf: *mut u8, n: i32) -> i32 {
    let t = system_time(st);
    fill_text(buf, n, &format!("{:02}/{:02}/{:04}", t[1], t[3], t[0]))
}

/// The requested format is ignored: always HH:mm:ss.
#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn GetTimeFormatA(_lcid: u32, _flags: u32, st: *const u16, _fmt: u32, buf: *mut u8, n: i32) -> i32 {
    let t = system_time(st);
    fill_text(buf, n, &format!("{:02}:{:02}:{:02}", t[4], t[5], t[6]))
}

/// Only drive C: exists, which is the game folder.
#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn GetLogicalDriveStringsA(n: u32, buf: *mut u8) -> u32 {
    let drives = b"C:\\\0\0";
    if buf.is_null() || (n as usize) < drives.len() {
        return drives.len() as u32;
    }
    std::ptr::copy_nonoverlapping(drives.as_ptr(), buf, drives.len());
    drives.len() as u32 - 1
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn InterlockedExchange(p: *mut i32, v: i32) -> i32 {
    std::sync::atomic::AtomicI32::from_ptr(p).swap(v, std::sync::atomic::Ordering::SeqCst)
}

/// There are no named mutexes: another instance never exists.
#[allow(non_snake_case)]
pub extern "stdcall" fn OpenMutexA(_access: u32, _inherit: i32, _name: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SetVolumeLabelA(_root: u32, _label: u32) -> i32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn CoUninitialize() {}

#[allow(non_snake_case)]
pub extern "stdcall" fn ShellExecuteA(_hwnd: u32, _op: u32, _file: u32, _params: u32, _dir: u32, _show: u32) -> u32 {
    31 // SE_ERR_NOASSOC
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetKeyboardLayout(_thread: u32) -> u32 {
    0x0409_0409
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn GetKeyboardLayoutList(n: i32, list: *mut u32) -> i32 {
    if n > 0 && !list.is_null() {
        *list = 0x0409_0409;
    }
    1
}

// ---- imm32.dll: Asian text input; there is no IME, so no context exists ------------------------

#[allow(non_snake_case)]
pub extern "stdcall" fn ImmCreateContext() -> u32 { 0 }
#[allow(non_snake_case)]
pub extern "stdcall" fn ImmDestroyContext(_h: u32) -> i32 { 1 }
#[allow(non_snake_case)]
pub extern "stdcall" fn ImmGetCandidateListA(_h: u32, _i: u32, _l: u32, _n: u32) -> u32 { 0 }
#[allow(non_snake_case)]
pub extern "stdcall" fn ImmGetCompositionStringA(_h: u32, _i: u32, _b: u32, _n: u32) -> i32 { 0 }
#[allow(non_snake_case)]
pub extern "stdcall" fn ImmGetContext(_w: u32) -> u32 { 0 }
#[allow(non_snake_case)]
pub extern "stdcall" fn ImmGetDescriptionA(_k: u32, _b: u32, _n: u32) -> u32 { 0 }
#[allow(non_snake_case)]
pub extern "stdcall" fn ImmGetOpenStatus(_h: u32) -> i32 { 0 }
#[allow(non_snake_case)]
pub extern "stdcall" fn ImmGetProperty(_k: u32, _i: u32) -> u32 { 0 }
#[allow(non_snake_case)]
pub extern "stdcall" fn ImmNotifyIME(_h: u32, _a: u32, _i: u32, _v: u32) -> i32 { 0 }
#[allow(non_snake_case)]
pub extern "stdcall" fn ImmReleaseContext(_w: u32, _h: u32) -> i32 { 1 }
#[allow(non_snake_case)]
pub extern "stdcall" fn ImmSetCompositionStringA(_h: u32, _i: u32, _c: u32, _cn: u32, _r: u32, _rn: u32) -> i32 { 0 }
#[allow(non_snake_case)]
pub extern "stdcall" fn ImmSetCompositionWindow(_h: u32, _c: u32) -> i32 { 0 }
#[allow(non_snake_case)]
pub extern "stdcall" fn ImmSetOpenStatus(_h: u32, _o: i32) -> i32 { 0 }

pub fn resolve_comdlg32(name: &str) -> Option<usize> {
    exports!(name, GetOpenFileNameA)
}

pub fn resolve_msvfw32(name: &str) -> Option<usize> {
    exports!(name, MCIWndCreateA)
}

/// There is no file dialog: answers as if the user had cancelled it.
#[allow(non_snake_case)]
extern "stdcall" fn GetOpenFileNameA(_ofn: u32) -> i32 {
    eprintln!("[winlayer] GetOpenFileNameA: no file dialog, reporting cancel");
    0
}

/// There is no MCI video window: creation fails (a null HWND), so the video is not played.
#[allow(non_snake_case)]
unsafe extern "C" fn MCIWndCreateA(_parent: u32, _instance: u32, _style: u32, file: *const i8) -> u32 {
    let file = if file.is_null() { "".into() } else { std::ffi::CStr::from_ptr(file).to_string_lossy() };
    eprintln!("[winlayer] MCIWndCreateA: video not supported ({file})");
    0
}

pub fn resolve_imm32(name: &str) -> Option<usize> {
    exports!(name, ImmCreateContext, ImmDestroyContext, ImmGetCandidateListA, ImmGetCompositionStringA, ImmGetContext, ImmGetDescriptionA, ImmGetOpenStatus, ImmGetProperty, ImmNotifyIME, ImmReleaseContext, ImmSetCompositionStringA, ImmSetCompositionWindow, ImmSetOpenStatus)
}

#[allow(non_snake_case)]
pub extern "stdcall" fn InitCommonControlsEx(_init: u32) -> i32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn ShellAboutA(_hwnd: u32, _app: u32, _other: u32, _icon: u32) -> i32 {
    1
}

#[allow(non_snake_case)]
pub extern "stdcall" fn GetLayout(_hdc: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SetLayout(_hdc: u32, _layout: u32) -> u32 {
    0
}

#[allow(non_snake_case)]
pub extern "stdcall" fn SetSystemPaletteUse(_hdc: u32, _usage: u32) -> u32 {
    1 // SYSPAL_STATIC
}

/// 16-bit API: mode 0 = read, 1 = write, 2 = read and write.
#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn _lopen(name: *const i8, mode: u32) -> i32 {
    let access = match mode & 3 { 0 => 0x8000_0000, 1 => 0x4000_0000, _ => 0xC000_0000 };
    let h = super::files::CreateFileA(name, access, 3, 0, 3, 0, 0);
    if h == u32::MAX { -1 } else { h as i32 }
}

#[allow(non_snake_case)]
pub unsafe extern "stdcall" fn _hread(h: u32, buf: *mut u8, n: u32) -> i32 {
    let mut got = 0u32;
    if super::files::ReadFile(h, buf, n, &mut got, 0) != 0 { got as i32 } else { -1 }
}
