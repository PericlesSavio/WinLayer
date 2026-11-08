//! DirectPlay Lobby (dplayx.dll) mínimo: o objeto existe, mas não há lobby nem provedores de rede.
//!
//! Jogos como o Revenant criam um `IDirectPlayLobby` (via `CoCreateInstance`) na inicialização e
//! desistem se ele não existir. Aqui ele responde "o programa não foi iniciado por um lobby" e lista
//! zero provedores/aplicações, então o jogo segue só com o modo de um jogador. O `IDirectPlay4` também
//! existe e lista zero conexões e zero sessões; criar jogadores, abrir sessões e enviar dados falham.

use std::sync::OnceLock;

const DP_OK: u32 = 0;
const E_NOINTERFACE: u32 = 0x8000_4002;
const DPERR_UNSUPPORTED: u32 = 0x8000_4001;
/// MAKE_DPHRESULT(1070)
const DPERR_NOTLOBBIED: u32 = 0x8877_042E;
/// MAKE_DPHRESULT(160) — sem conexão (ConnectEx)
const DPERR_NOCONNECTION: u32 = 0x8877_00A0;

/// Primeiro dword de CLSID_DirectPlayLobby {2FE8F810-B2A5-11D0-A787-0000F803ABFC}.
pub const CLSID_DIRECTPLAYLOBBY: u32 = 0x2FE8_F810;
/// Primeiro dword de CLSID_DirectPlay {D1EB6D20-8923-11D0-9D97-00A0C90A43CB}.
pub const CLSID_DIRECTPLAY: u32 = 0xD1EB_6D20;

fn trace() -> bool {
    std::env::var_os("WINLAYER_DPLAY_TRACE").is_some()
}

macro_rules! vtable {
    ($($f:expr),* $(,)?) => {{
        static V: OnceLock<Vec<u32>> = OnceLock::new();
        V.get_or_init(|| vec![$($f as *const () as usize as u32),*]).as_slice()
    }};
}

/// Cria o objeto do lobby (IDirectPlayLobbyA/2A/3A: a vtable da versão 3 contém as anteriores).
pub fn create_lobby() -> u32 {
    if trace() {
        eprintln!("[dplay] objeto IDirectPlayLobby criado");
    }
    Box::leak(Box::new(lobby_vtbl().as_ptr() as u32)) as *mut u32 as u32
}

fn lobby_vtbl() -> &'static [u32] {
    vtable![
        qi, addref, release, connect, create_address, enum_address, enum_address_types,
        enum_local_apps, get_connection_settings, receive_lobby_message, run_application,
        send_lobby_message, set_connection_settings, set_lobby_message_event,
        // IDirectPlayLobby2
        create_compound_address,
        // IDirectPlayLobby3
        connect_ex, register_application, unregister_application, wait_for_connection_settings,
    ]
}

fn log(name: &str) {
    if trace() {
        eprintln!("[dplay] {name}");
    }
}

unsafe extern "stdcall" fn qi(this: u32, riid: u32, out: *mut u32) -> u32 {
    let first = if riid == 0 { 0 } else { std::ptr::read_unaligned(riid as usize as *const u32) };
    // IUnknown, IDirectPlayLobby {AF465C71-...}, A {26C66A70-...}, 2 {0194C220-...}, 2A {1BB4AF80-...},
    // 3 {2DB72490-...}, 3A {2DB72491-...}
    if matches!(first, 0 | 0xAF46_5C71 | 0x26C6_6A70 | 0x0194_C220 | 0x1BB4_AF80 | 0x2DB7_2490 | 0x2DB7_2491) {
        *out = this;
        DP_OK
    } else {
        log(&format!("QueryInterface {first:#010x}: não suportada"));
        *out = 0;
        E_NOINTERFACE
    }
}

extern "stdcall" fn addref(_this: u32) -> u32 {
    1
}

extern "stdcall" fn release(_this: u32) -> u32 {
    0
}

extern "stdcall" fn connect(_this: u32, _flags: u32, _dp: u32, _outer: u32) -> u32 {
    log("Connect");
    DPERR_NOTLOBBIED
}

extern "stdcall" fn create_address(_this: u32, _sp: u32, _ty: u32, _data: u32, _size: u32, _addr: u32, _addr_size: u32) -> u32 {
    log("CreateAddress");
    DPERR_UNSUPPORTED
}

extern "stdcall" fn enum_address(_this: u32, _cb: u32, _addr: u32, _size: u32, _ctx: u32) -> u32 {
    log("EnumAddress");
    DP_OK
}

extern "stdcall" fn enum_address_types(_this: u32, _cb: u32, _sp: u32, _ctx: u32, _flags: u32) -> u32 {
    log("EnumAddressTypes");
    DP_OK
}

extern "stdcall" fn enum_local_apps(_this: u32, _cb: u32, _ctx: u32, _flags: u32) -> u32 {
    log("EnumLocalApplications");
    DP_OK
}

extern "stdcall" fn get_connection_settings(_this: u32, _app: u32, _data: u32, _size: u32) -> u32 {
    log("GetConnectionSettings");
    DPERR_NOTLOBBIED
}

extern "stdcall" fn receive_lobby_message(_this: u32, _flags: u32, _app: u32, _msg: u32, _data: u32, _size: u32) -> u32 {
    DPERR_NOTLOBBIED
}

extern "stdcall" fn run_application(_this: u32, _flags: u32, _app: u32, _conn: u32, _event: u32) -> u32 {
    log("RunApplication");
    DPERR_UNSUPPORTED
}

extern "stdcall" fn send_lobby_message(_this: u32, _flags: u32, _app: u32, _data: u32, _size: u32) -> u32 {
    DPERR_NOTLOBBIED
}

extern "stdcall" fn set_connection_settings(_this: u32, _flags: u32, _app: u32, _conn: u32) -> u32 {
    DPERR_NOTLOBBIED
}

extern "stdcall" fn set_lobby_message_event(_this: u32, _flags: u32, _app: u32, _event: u32) -> u32 {
    DPERR_NOTLOBBIED
}

extern "stdcall" fn create_compound_address(_this: u32, _elems: u32, _count: u32, _addr: u32, _size: u32) -> u32 {
    log("CreateCompoundAddress");
    DPERR_UNSUPPORTED
}

extern "stdcall" fn connect_ex(_this: u32, _flags: u32, _riid: u32, _dp: u32, _outer: u32) -> u32 {
    log("ConnectEx");
    DPERR_NOCONNECTION
}

extern "stdcall" fn register_application(_this: u32, _flags: u32, _desc: u32) -> u32 {
    DP_OK
}

extern "stdcall" fn unregister_application(_this: u32, _flags: u32, _guid: u32) -> u32 {
    DP_OK
}

extern "stdcall" fn wait_for_connection_settings(_this: u32, _flags: u32) -> u32 {
    DPERR_NOTLOBBIED
}

// ---- IDirectPlay2/3/4 (A e W têm a mesma vtable) --------------------------------------------

/// Cria o objeto IDirectPlay4A.
pub fn create_dplay() -> u32 {
    if trace() {
        eprintln!("[dplay] objeto IDirectPlay4 criado");
    }
    Box::leak(Box::new(dplay_vtbl().as_ptr() as u32)) as *mut u32 as u32
}

// Métodos sem efeito, pelo número total de argumentos (contando `this`): no stdcall quem é chamado limpa
// a pilha, então cada entrada precisa ter exatamente a aridade do método que ocupa.
extern "stdcall" fn ok1(_t: u32) -> u32 { DP_OK }
extern "stdcall" fn ok2(_t: u32, _a: u32) -> u32 { DP_OK }
extern "stdcall" fn ok5(_t: u32, _a: u32, _b: u32, _c: u32, _d: u32) -> u32 { DP_OK }
extern "stdcall" fn ok6(_t: u32, _a: u32, _b: u32, _c: u32, _d: u32, _e: u32) -> u32 { DP_OK }
extern "stdcall" fn no2(_t: u32, _a: u32) -> u32 { DPERR_UNSUPPORTED }
extern "stdcall" fn no3(_t: u32, _a: u32, _b: u32) -> u32 { DPERR_UNSUPPORTED }
extern "stdcall" fn no4(_t: u32, _a: u32, _b: u32, _c: u32) -> u32 { DPERR_UNSUPPORTED }
extern "stdcall" fn no5(_t: u32, _a: u32, _b: u32, _c: u32, _d: u32) -> u32 { DPERR_UNSUPPORTED }
extern "stdcall" fn no6(_t: u32, _a: u32, _b: u32, _c: u32, _d: u32, _e: u32) -> u32 { DPERR_UNSUPPORTED }
extern "stdcall" fn no7(_t: u32, _a: u32, _b: u32, _c: u32, _d: u32, _e: u32, _f: u32) -> u32 { DPERR_UNSUPPORTED }
#[allow(clippy::too_many_arguments)]
extern "stdcall" fn no10(_t: u32, _a: u32, _b: u32, _c: u32, _d: u32, _e: u32, _f: u32, _g: u32, _h: u32, _i: u32) -> u32 {
    DPERR_UNSUPPORTED
}

unsafe extern "stdcall" fn dp_qi(this: u32, riid: u32, out: *mut u32) -> u32 {
    let first = if riid == 0 { 0 } else { std::ptr::read_unaligned(riid as usize as *const u32) };
    // IUnknown, IDirectPlay2/2A, 3/3A, 4/4A
    if matches!(first, 0 | 0x2B74_F7C0 | 0x9D46_0580 | 0x133E_FE40 | 0x133E_FE41 | 0x0AB1_C530 | 0x0AB1_C531) {
        *out = this;
        DP_OK
    } else {
        log(&format!("IDirectPlay::QueryInterface {first:#010x}: não suportada"));
        *out = 0;
        E_NOINTERFACE
    }
}

fn dplay_vtbl() -> &'static [u32] {
    vtable![
        dp_qi, addref, release,
        no3,  // AddPlayerToGroup
        ok1,  // Close
        no6,  // CreateGroup
        no7,  // CreatePlayer
        no3,  // DeletePlayerFromGroup
        no2,  // DestroyGroup
        no2,  // DestroyPlayer
        ok6,  // EnumGroupPlayers
        ok5,  // EnumGroups
        ok5,  // EnumPlayers
        ok6,  // EnumSessions
        no3,  // GetCaps
        no5,  // GetGroupData
        no4,  // GetGroupName
        no3,  // GetMessageCount
        no4,  // GetPlayerAddress
        no4,  // GetPlayerCaps
        no5,  // GetPlayerData
        no4,  // GetPlayerName
        no3,  // GetSessionDesc
        ok2,  // Initialize
        no3,  // Open
        no6,  // Receive
        no6,  // Send
        no5,  // SetGroupData
        no4,  // SetGroupName
        no5,  // SetPlayerData
        no4,  // SetPlayerName
        no3,  // SetSessionDesc
        // IDirectPlay3
        no3,  // AddGroupToGroup
        no7,  // CreateGroupInGroup
        no3,  // DeleteGroupFromGroup
        ok5,  // EnumConnections
        ok6,  // EnumGroupsInGroup
        no5,  // GetGroupConnectionSettings
        no3,  // InitializeConnection
        no5,  // SecureOpen
        no5,  // SendChatMessage
        no4,  // SetGroupConnectionSettings
        no3,  // StartSession
        no3,  // GetGroupFlags
        no3,  // GetGroupParent
        no5,  // GetPlayerAccount
        no3,  // GetPlayerFlags
        // IDirectPlay4
        no3,  // GetGroupOwner
        no3,  // SetGroupOwner
        no10, // SendEx
        no6,  // GetMessageQueue
        no3,  // CancelMessage
        no4,  // CancelPriority
    ]
}
