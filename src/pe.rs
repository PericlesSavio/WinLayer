//! Minimal parser for PE32 (32-bit Win32 executables).

use std::fmt;

pub const IMAGE_FILE_DLL: u16 = 0x2000;

#[derive(Debug)]
pub enum PeError {
    Truncated,
    /// Starts with "MZ" but has no PE header (plain DOS) or has NE/LE (Win16).
    NotPe(&'static str),
    Unsupported(&'static str),
}

impl fmt::Display for PeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PeError::Truncated => write!(f, "truncated file"),
            PeError::NotPe(s) => write!(f, "not a PE32: {s}"),
            PeError::Unsupported(s) => write!(f, "unsupported: {s}"),
        }
    }
}

pub struct Section {
    pub name: String,
    pub virtual_address: u32,
    pub virtual_size: u32,
    pub raw_offset: u32,
    pub raw_size: u32,
}

pub struct Import {
    pub dll: String,
    /// RVA of this DLL's IAT (each entry: 4 bytes).
    pub iat_rva: u32,
    pub names: Vec<ImportName>,
}

pub enum ImportName {
    Name(String),
    Ordinal(u16),
}

pub struct Pe32 {
    pub image_base: u32,
    pub size_of_image: u32,
    pub size_of_headers: u32,
    pub entry_rva: u32,
    pub characteristics: u16,
    pub sections: Vec<Section>,
    pub imports: Vec<Import>,
    pub reloc_dir: (u32, u32),
    pub resource_dir: (u32, u32),
    pub export_dir: (u32, u32),
}

fn u16_at(d: &[u8], o: usize) -> Result<u16, PeError> {
    d.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]])).ok_or(PeError::Truncated)
}
fn u32_at(d: &[u8], o: usize) -> Result<u32, PeError> {
    d.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).ok_or(PeError::Truncated)
}

/// Reads a NUL-terminated C string.
fn cstr_at(d: &[u8], o: usize) -> Result<String, PeError> {
    let s = d.get(o..).ok_or(PeError::Truncated)?;
    let n = s.iter().position(|&b| b == 0).ok_or(PeError::Truncated)?;
    Ok(String::from_utf8_lossy(&s[..n]).into_owned())
}

impl Pe32 {
    pub fn parse(d: &[u8]) -> Result<Pe32, PeError> {
        if d.get(0..2) != Some(b"MZ") {
            return Err(PeError::NotPe("no MZ signature"));
        }
        let lfanew = u32_at(d, 0x3c)? as usize;
        // Plain DOS (like Civilization 1): e_lfanew zero/outside the file.
        if lfanew == 0 || lfanew + 4 > d.len() {
            return Err(PeError::NotPe("plain DOS executable (needs DOSBox, not Win32)"));
        }
        match d.get(lfanew..lfanew + 2) {
            Some(b"PE") if d[lfanew + 2] == 0 && d[lfanew + 3] == 0 => {}
            Some(b"NE") => return Err(PeError::Unsupported("Win16 (NE), not implemented yet")),
            Some(b"LE") | Some(b"LX") => return Err(PeError::Unsupported("VxD/DOS extender (LE)")),
            _ => return Err(PeError::NotPe("missing PE signature")),
        }

        let coff = lfanew + 4;
        let machine = u16_at(d, coff)?;
        if machine != 0x014c {
            return Err(PeError::Unsupported("machine other than i386"));
        }
        let nsections = u16_at(d, coff + 2)? as usize;
        let opt_size = u16_at(d, coff + 16)? as usize;
        let characteristics = u16_at(d, coff + 18)?;

        let opt = coff + 20;
        if u16_at(d, opt)? != 0x010b {
            return Err(PeError::Unsupported("not PE32 (maybe PE32+)"));
        }
        let entry_rva = u32_at(d, opt + 16)?;
        let image_base = u32_at(d, opt + 28)?;
        let size_of_image = u32_at(d, opt + 56)?;
        let size_of_headers = u32_at(d, opt + 60)?;
        let ndirs = u32_at(d, opt + 92)? as usize;
        let dirs = opt + 96;
        let dir = |i: usize| -> Result<(u32, u32), PeError> {
            if i < ndirs { Ok((u32_at(d, dirs + i * 8)?, u32_at(d, dirs + i * 8 + 4)?)) } else { Ok((0, 0)) }
        };
        let import_dir = dir(1)?;
        let reloc_dir = dir(5)?;
        let resource_dir = dir(2)?;
        let export_dir = dir(0)?;

        let mut sections = Vec::with_capacity(nsections);
        let sh = opt + opt_size;
        for i in 0..nsections {
            let o = sh + i * 40;
            let raw = d.get(o..o + 8).ok_or(PeError::Truncated)?;
            let end = raw.iter().position(|&b| b == 0).unwrap_or(8);
            sections.push(Section {
                name: String::from_utf8_lossy(&raw[..end]).into_owned(),
                virtual_size: u32_at(d, o + 8)?,
                virtual_address: u32_at(d, o + 12)?,
                raw_size: u32_at(d, o + 16)?,
                raw_offset: u32_at(d, o + 20)?,
            });
        }

        let mut pe = Pe32 {
            image_base, size_of_image, size_of_headers, entry_rva,
            characteristics, sections, imports: Vec::new(), reloc_dir, resource_dir, export_dir,
        };
        pe.imports = pe.read_imports(d, import_dir.0)?;
        Ok(pe)
    }

    /// Converts an RVA to a file offset.
    pub fn rva_to_offset(&self, rva: u32) -> Option<usize> {
        if rva < self.size_of_headers {
            return Some(rva as usize);
        }
        self.sections.iter().find_map(|s| {
            let size = s.virtual_size.max(s.raw_size);
            (rva >= s.virtual_address && rva < s.virtual_address + size)
                .then(|| (rva - s.virtual_address + s.raw_offset) as usize)
        })
    }

    fn read_imports(&self, d: &[u8], rva: u32) -> Result<Vec<Import>, PeError> {
        let mut out = Vec::new();
        if rva == 0 {
            return Ok(out);
        }
        let mut o = self.rva_to_offset(rva).ok_or(PeError::Truncated)?;
        loop {
            let ilt = u32_at(d, o)?;
            let name_rva = u32_at(d, o + 12)?;
            let iat = u32_at(d, o + 16)?;
            if name_rva == 0 && iat == 0 {
                break;
            }
            let dll = cstr_at(d, self.rva_to_offset(name_rva).ok_or(PeError::Truncated)?)?;
            // Without OriginalFirstThunk (ILT = 0), the linker uses the IAT itself.
            let mut t = self.rva_to_offset(if ilt != 0 { ilt } else { iat }).ok_or(PeError::Truncated)?;
            let mut names = Vec::new();
            loop {
                let v = u32_at(d, t)?;
                if v == 0 {
                    break;
                }
                names.push(if v & 0x8000_0000 != 0 {
                    ImportName::Ordinal(v as u16)
                } else {
                    let off = self.rva_to_offset(v).ok_or(PeError::Truncated)? + 2; // skips the hint
                    ImportName::Name(cstr_at(d, off)?)
                });
                t += 4;
            }
            out.push(Import { dll, iat_rva: iat, names });
            o += 20;
        }
        Ok(out)
    }
}
