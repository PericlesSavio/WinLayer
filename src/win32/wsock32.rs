//! wsock32.dll — no network. Initialization works, but creating sockets fails with "network down":
//! the game carries on in local mode only. Address conversions are real because they do not depend on the network.

const INVALID_SOCKET: u32 = u32::MAX;
const SOCKET_ERROR: i32 = -1;
const WSAENETDOWN: u32 = 10050;

pub fn resolve(name: &str) -> Option<usize> {
    exports!(
        name, accept, bind, closesocket, connect, htonl, htons, inet_addr, inet_ntoa, ioctlsocket, listen, ntohl, ntohs,
        recv, recvfrom, send, sendto, setsockopt, shutdown, socket, gethostbyname, gethostname, WSAGetLastError,
        WSAStartup, WSACleanup,
    )
}

/// Programs usually import wsock32 by ordinal.
pub fn name_of(ordinal: u32) -> Option<&'static str> {
    Some(match ordinal {
        1 => "accept",
        2 => "bind",
        3 => "closesocket",
        4 => "connect",
        8 => "htonl",
        9 => "htons",
        10 => "inet_addr",
        11 => "inet_ntoa",
        12 => "ioctlsocket",
        13 => "listen",
        14 => "ntohl",
        15 => "ntohs",
        16 => "recv",
        17 => "recvfrom",
        19 => "send",
        20 => "sendto",
        21 => "setsockopt",
        22 => "shutdown",
        23 => "socket",
        52 => "gethostbyname",
        57 => "gethostname",
        111 => "WSAGetLastError",
        115 => "WSAStartup",
        116 => "WSACleanup",
        _ => return None,
    })
}

extern "stdcall" fn accept(_s: u32, _addr: u32, _len: u32) -> u32 {
    INVALID_SOCKET
}

extern "stdcall" fn bind(_s: u32, _addr: u32, _len: u32) -> i32 {
    SOCKET_ERROR
}

extern "stdcall" fn closesocket(_s: u32) -> i32 {
    0
}

extern "stdcall" fn connect(_s: u32, _addr: u32, _len: u32) -> i32 {
    SOCKET_ERROR
}

extern "stdcall" fn htonl(v: u32) -> u32 {
    v.to_be()
}

extern "stdcall" fn ntohl(v: u32) -> u32 {
    u32::from_be(v)
}

extern "stdcall" fn htons(v: u32) -> u32 {
    (v as u16).to_be() as u32
}

extern "stdcall" fn ntohs(v: u32) -> u32 {
    u16::from_be(v as u16) as u32
}

unsafe extern "stdcall" fn inet_addr(s: *const i8) -> u32 {
    if s.is_null() {
        return u32::MAX;
    }
    match std::ffi::CStr::from_ptr(s).to_string_lossy().parse::<std::net::Ipv4Addr>() {
        Ok(ip) => u32::from_ne_bytes(ip.octets()),
        Err(_) => u32::MAX, // INADDR_NONE
    }
}

/// As on Windows, returns a fixed buffer.
extern "stdcall" fn inet_ntoa(addr: u32) -> *const u8 {
    static BUF: std::sync::Mutex<[u8; 16]> = std::sync::Mutex::new([0; 16]);
    let text = std::net::Ipv4Addr::from(addr.to_ne_bytes()).to_string();
    let mut b = BUF.lock().unwrap();
    b.fill(0);
    b[..text.len()].copy_from_slice(text.as_bytes());
    b.as_ptr()
}

extern "stdcall" fn ioctlsocket(_s: u32, _cmd: u32, _arg: u32) -> i32 {
    SOCKET_ERROR
}

extern "stdcall" fn listen(_s: u32, _n: u32) -> i32 {
    SOCKET_ERROR
}

extern "stdcall" fn recv(_s: u32, _buf: u32, _n: u32, _flags: u32) -> i32 {
    SOCKET_ERROR
}

extern "stdcall" fn recvfrom(_s: u32, _buf: u32, _n: u32, _flags: u32, _from: u32, _len: u32) -> i32 {
    SOCKET_ERROR
}

extern "stdcall" fn send(_s: u32, _buf: u32, _n: u32, _flags: u32) -> i32 {
    SOCKET_ERROR
}

extern "stdcall" fn sendto(_s: u32, _buf: u32, _n: u32, _flags: u32, _to: u32, _len: u32) -> i32 {
    SOCKET_ERROR
}

extern "stdcall" fn setsockopt(_s: u32, _level: u32, _opt: u32, _val: u32, _len: u32) -> i32 {
    SOCKET_ERROR
}

extern "stdcall" fn shutdown(_s: u32, _how: u32) -> i32 {
    SOCKET_ERROR
}

extern "stdcall" fn socket(_af: u32, _ty: u32, _proto: u32) -> u32 {
    INVALID_SOCKET
}

extern "stdcall" fn gethostbyname(_name: u32) -> u32 {
    0
}

unsafe extern "stdcall" fn gethostname(buf: *mut u8, n: i32) -> i32 {
    let name = b"winlayer\0";
    if buf.is_null() || (n as usize) < name.len() {
        return SOCKET_ERROR;
    }
    std::ptr::copy_nonoverlapping(name.as_ptr(), buf, name.len());
    0
}

#[allow(non_snake_case)]
extern "stdcall" fn WSAGetLastError() -> u32 {
    WSAENETDOWN
}

/// WSADATA: wVersion, wHighVersion, description[257], status[129], iMaxSockets, iMaxUdpDg, lpVendorInfo (400 bytes).
#[allow(non_snake_case)]
unsafe extern "stdcall" fn WSAStartup(version: u32, data: *mut u8) -> i32 {
    if !data.is_null() {
        std::ptr::write_bytes(data, 0, 400);
        std::ptr::write_unaligned(data as *mut u16, version as u16);
        std::ptr::write_unaligned(data.add(2) as *mut u16, 0x0101);
    }
    0
}

#[allow(non_snake_case)]
extern "stdcall" fn WSACleanup() -> i32 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinals_match_exported_names() {
        for ordinal in 1..200 {
            if let Some(name) = name_of(ordinal) {
                assert!(resolve(name).is_some(), "{name}");
            }
        }
    }

    #[test]
    fn inet_addr_and_ntoa_round_trip() {
        unsafe {
            let a = inet_addr(c"10.1.2.3".as_ptr());
            assert_eq!(std::ffi::CStr::from_ptr(inet_ntoa(a) as *const i8).to_str().unwrap(), "10.1.2.3");
        }
    }
}
