//! Interrupções de software (INT n) executadas pelo código de 16 bits. No Linux elas viram SIGSEGV;
//! o handler decodifica `CD nn`, chama `handle` e retoma a execução depois da instrução.

use crate::win32::user32;

/// Registradores salvos no contexto do sinal (ordem do `sigcontext` do i386):
/// GS FS ES DS EDI ESI EBP ESP EBX EDX ECX EAX TRAPNO ERR EIP CS EFL UESP SS.
pub struct Regs<'a>(pub &'a mut [u32]);

impl Regs<'_> {
    fn ax(&self) -> u16 {
        self.0[11] as u16
    }
    fn set_ax(&mut self, v: u16) {
        self.0[11] = (self.0[11] & 0xFFFF_0000) | v as u32;
    }
    fn ah(&self) -> u8 {
        (self.0[11] >> 8) as u8
    }
    fn al(&self) -> u8 {
        self.0[11] as u8
    }
    fn set_al(&mut self, v: u8) {
        self.0[11] = (self.0[11] & !0xFF) | v as u32;
    }
    fn bx(&self) -> u16 {
        self.0[8] as u16
    }
    fn cx(&self) -> u16 {
        self.0[10] as u16
    }
    fn dx(&self) -> u16 {
        self.0[9] as u16
    }
    /// Caminho ASCIZ em DS:DX.
    fn path_dsdx(&self) -> Option<String> {
        let p = crate::win16::ldt::linear(self.0[3] as u16, self.dx())?;
        Some(unsafe { std::ffi::CStr::from_ptr(p as usize as *const i8) }.to_string_lossy().into_owned())
    }
    fn fail(&mut self, code: u16) {
        self.set_ax(code);
        self.set_carry(true);
    }
    fn set_cx(&mut self, v: u16) {
        self.0[10] = (self.0[10] & 0xFFFF_0000) | v as u32;
    }
    fn set_dx(&mut self, v: u16) {
        self.0[9] = (self.0[9] & 0xFFFF_0000) | v as u32;
    }
    fn set_bx(&mut self, v: u16) {
        self.0[8] = (self.0[8] & 0xFFFF_0000) | v as u32;
    }
    fn set_dh_dl(&mut self, dh: u8, dl: u8) {
        self.set_dx(((dh as u16) << 8) | dl as u16);
    }
    fn set_carry(&mut self, on: bool) {
        self.0[16] = if on { self.0[16] | 1 } else { self.0[16] & !1 };
    }
}

fn unimplemented(int: u8, r: &Regs) -> ! {
    eprintln!("\n[win16] INT {int:02X}h não implementada (AX={:04x})", r.ax());
    crate::win32::kernel32::exit_process(3)
}

/// Data/hora local simples (UTC) a partir do relógio do sistema.
fn clock() -> (u16, u8, u8, u8, u8, u8, u8, u8) {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
    let (days, rem) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + (month <= 2) as i64;
    (year as u16, month as u8, day as u8, (days + 4).rem_euclid(7) as u8, (rem / 3600) as u8, (rem % 3600 / 60) as u8, (rem % 60) as u8, 0)
}

pub fn handle(int: u8, r: &mut Regs) {
    match int {
        0x1A => match r.ah() {
            // BIOS: ler contador de ticks (18,2 Hz) desde a meia-noite
            0 => {
                // Modo captura: cada leitura avança o relógio virtual, então esperas ocupadas (que
                // chamam `clock()` em laço) terminam logo em vez de levarem segundos reais.
                user32::tick_virtual(2);
                let ms = if user32::shot_mode() { user32::now_ms() } else { user32::real_ms() };
                let ticks = (ms as u64 * 182_065 / 10_000_000) as u32;
                r.set_cx((ticks >> 16) as u16);
                r.set_dx(ticks as u16);
                r.set_al(0);
            }
            2 => {
                // RTC: hora em BCD
                let (_, _, _, _, h, m, s, _) = clock();
                let bcd = |v: u8| ((v / 10) << 4) | (v % 10);
                r.0[10] = ((bcd(h) as u32) << 8) | bcd(m) as u32; // CH:CL
                r.set_dh_dl(bcd(s), 0);
                r.set_carry(false);
            }
            _ => unimplemented(int, r),
        },
        0x21 => match r.ah() {
            0x30 => r.set_ax(0x1606), // DOS 6.22 (AL = maior, AH = menor)
            0x4C => crate::win32::kernel32::exit_process(r.al() as u32),
            0x2C => {
                let (_, _, _, _, h, m, s, cs) = clock();
                r.0[10] = ((h as u32) << 8) | m as u32;
                r.set_dh_dl(s, cs);
            }
            0x2A => {
                let (y, mo, d, dow, ..) = clock();
                r.set_cx(y);
                r.set_dh_dl(mo, d);
                r.set_al(dow);
            }
            0x19 => r.set_al(2), // unidade atual: C:
            0x47 => {
                // diretório atual (DS:SI recebe o caminho sem a letra da unidade e sem `\\` inicial): raiz
                if let Some(p) = crate::win16::ldt::linear(r.0[3] as u16, r.0[5] as u16) {
                    unsafe { *(p as usize as *mut u8) = 0 };
                }
                r.set_carry(false);
            }
            0x3B => r.set_carry(false), // chdir: sempre aceito
            0x3C | 0x3D => {
                // criar / abrir (DS:DX = caminho; 3D: AL = modo de acesso)
                let (create, mode) = (r.ah() == 0x3C, (r.al() & 3) as u16);
                let Some(path) = r.path_dsdx() else { return r.fail(3) };
                let h = super::kernel_ext::open_file(&path, create, mode);
                if h == 0xFFFF {
                    let exists = crate::win32::files::host_path(&path).exists();
                    r.fail(if exists { 5 } else { 2 });
                } else {
                    r.set_ax(h as u16);
                    r.set_carry(false);
                }
            }
            0x3E => {
                if super::kernel_ext::dos_close(r.bx()) { r.set_carry(false) } else { r.fail(6) }
            }
            0x3F | 0x40 => {
                let (n, h) = (r.cx() as u32, r.bx());
                let Some(buf) = crate::win16::ldt::linear(r.0[3] as u16, r.dx()) else { return r.fail(5) };
                let done = if r.ah() == 0x3F {
                    super::kernel_ext::read_into(h, buf, n)
                } else if n == 0 {
                    // gravar 0 bytes trunca na posição atual
                    if super::kernel_ext::dos_truncate_here(h) { 0 } else { 0xFFFF }
                } else {
                    super::kernel_ext::write_from(h, buf, n)
                };
                if done == 0xFFFF {
                    r.fail(6);
                } else {
                    r.set_ax(done as u16);
                    r.set_carry(false);
                }
            }
            0x41 => {
                let Some(path) = r.path_dsdx() else { return r.fail(3) };
                if std::fs::remove_file(crate::win32::files::host_path(&path)).is_ok() { r.set_carry(false) } else { r.fail(2) }
            }
            0x42 => {
                let off = ((r.cx() as u32) << 16 | r.dx() as u32) as i32;
                match super::kernel_ext::dos_seek(r.bx(), off, r.al()) {
                    Some(p) => {
                        r.set_ax(p as u16);
                        r.set_dx((p >> 16) as u16);
                        r.set_carry(false);
                    }
                    None => r.fail(6),
                }
            }
            0x43 => {
                // atributos de arquivo: 0 = ler (CX), 1 = gravar (aceito)
                let Some(path) = r.path_dsdx() else { return r.fail(3) };
                let p = crate::win32::files::host_path(&path);
                match std::fs::metadata(&p) {
                    Ok(m) => {
                        if r.al() == 0 {
                            r.set_cx(if m.is_dir() { 0x10 } else { 0x20 });
                        }
                        r.set_carry(false);
                    }
                    Err(_) => r.fail(2),
                }
            }
            0x36 => {
                // espaço livre: 8 setores/cluster, 512 bytes/setor, ~128 MB livres
                r.set_ax(8);
                r.set_bx(0x7000);
                r.set_cx(512);
                r.set_dx(0x7000);
            }
            0x44 => {
                // IOCTL: informação do dispositivo -> arquivo comum
                r.set_dx(0);
                r.set_ax(0);
                r.set_carry(false);
            }
            0x57 => {
                // data/hora do arquivo: devolve a data/hora atual
                let (y, mo, d, _, h, mi, s, _) = clock();
                if r.al() == 0 {
                    r.set_cx(((h as u16) << 11) | ((mi as u16) << 5) | (s as u16 / 2));
                    r.set_dx(((y.saturating_sub(1980)) << 9) | ((mo as u16) << 5) | d as u16);
                }
                r.set_carry(false);
            }
            0x0E | 0x25 | 0x0D | 0x2E => {}
            0x35 => {
                r.set_bx(0);
                r.0[2] = 0; // ES = 0
            }
            0x62 | 0x51 => r.set_bx(crate::win16::kernel::with_process(|p| p.psp_sel)), // PSP atual
            _ => unimplemented(int, r),
        },
        0x2F | 0x31 => {
            r.set_carry(true); // sem multiplex/DPMI: "não suportado"
        }
        _ => unimplemented(int, r),
    }
}
