//! avifil32.dll (reading AVI files) and the video codec part of msvfw32.dll (`IC*`), with a built-in
//! Cinepak decoder: the codec of most game videos of the time.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

pub fn resolve_avifil32(name: &str) -> Option<usize> {
    exports!(
        name, AVIFileInit, AVIFileExit, AVIStreamOpenFromFileA, AVIStreamRelease, AVIStreamInfoA, AVIStreamLength,
        AVIStreamReadFormat, AVIStreamRead, AVIStreamStart,
    )
}

pub fn resolve_msvfw32(name: &str) -> Option<usize> {
    exports!(name, ICLocate, ICOpen, ICClose, ICDecompress, ICSendMessage, ICInfo)
}

const AVIERR_FILEOPEN: u32 = 0x8004_406D;
const AVIERR_NODATA: u32 = 0x8004_4071;
const AVIERR_BUFFERTOOSMALL: u32 = 0x8004_4074;

fn fourcc(s: &[u8; 4]) -> u32 {
    u32::from_le_bytes(*s)
}

fn rd16(d: &[u8], o: usize) -> u32 {
    d.get(o..o + 2).map_or(0, |b| u16::from_le_bytes([b[0], b[1]]) as u32)
}

fn rd32(d: &[u8], o: usize) -> u32 {
    d.get(o..o + 4).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

// ---- AVI parsing ----------------------------------------------------------------------------

struct Stream {
    /// `strh` (56 bytes).
    header: Vec<u8>,
    /// `strf`: BITMAPINFOHEADER or WAVEFORMATEX.
    format: Vec<u8>,
    /// (offset, size) of each chunk in the file, in order.
    chunks: Vec<(usize, usize)>,
}

impl Stream {
    fn kind(&self) -> u32 {
        rd32(&self.header, 0)
    }
    fn sample_size(&self) -> u32 {
        rd32(&self.header, 44)
    }
    /// Audio is read by bytes (block align); video by chunk (one frame each).
    fn block_align(&self) -> usize {
        match self.sample_size() {
            0 => rd16(&self.format, 12).max(1) as usize,
            n => n as usize,
        }
    }
    fn byte_stream(&self) -> bool {
        self.kind() == fourcc(b"auds")
    }
    fn total_bytes(&self) -> usize {
        self.chunks.iter().map(|c| c.1).sum()
    }
    fn length(&self) -> usize {
        if self.byte_stream() { self.total_bytes() / self.block_align() } else { self.chunks.len() }
    }
}

struct AviFile {
    data: Vec<u8>,
    streams: Vec<Stream>,
}

/// Walks the RIFF tree: stream headers in `hdrl`, data chunks in `movi` (also inside `LIST rec`).
fn parse_avi(data: Vec<u8>) -> Option<AviFile> {
    if rd32(&data, 0) != fourcc(b"RIFF") || rd32(&data, 8) != fourcc(b"AVI ") {
        return None;
    }
    let mut streams: Vec<Stream> = Vec::new();
    fn walk(d: &[u8], mut pos: usize, end: usize, streams: &mut Vec<Stream>) {
        while pos + 8 <= end.min(d.len()) {
            let (id, size) = (rd32(d, pos), rd32(d, pos + 4) as usize);
            let body = pos + 8;
            if id == fourcc(b"LIST") {
                if rd32(d, body) == fourcc(b"strl") {
                    streams.push(Stream { header: Vec::new(), format: Vec::new(), chunks: Vec::new() });
                }
                walk(d, body + 4, body + size, streams);
            } else if id == fourcc(b"strh") {
                if let Some(s) = streams.last_mut() {
                    s.header = d[body..(body + size).min(d.len())].to_vec();
                }
            } else if id == fourcc(b"strf") {
                if let Some(s) = streams.last_mut() {
                    s.format = d[body..(body + size).min(d.len())].to_vec();
                }
            } else {
                // data chunk "NNxx": NN is the stream number in ASCII
                let tag = id.to_le_bytes();
                if tag[0].is_ascii_digit() && tag[1].is_ascii_digit() {
                    let n = ((tag[0] - b'0') * 10 + (tag[1] - b'0')) as usize;
                    if let Some(s) = streams.get_mut(n) {
                        s.chunks.push((body, size.min(d.len().saturating_sub(body))));
                    }
                }
            }
            pos = body + size + (size & 1);
        }
    }
    walk(&data, 12, data.len(), &mut streams);
    Some(AviFile { data, streams })
}

struct OpenStream {
    file: std::sync::Arc<AviFile>,
    index: usize,
}

static STREAMS: Mutex<BTreeMap<u32, OpenStream>> = Mutex::new(BTreeMap::new());
static NEXT: AtomicU32 = AtomicU32::new(0x7100);

#[allow(non_snake_case)]
extern "stdcall" fn AVIFileInit() {}

#[allow(non_snake_case)]
extern "stdcall" fn AVIFileExit() {}

/// Opens the `n`-th stream of type `kind` ('vids', 'auds'; 0 = any). The handle is opaque: programs
/// only pass it back to the AVIStream functions.
#[allow(non_snake_case)]
unsafe extern "stdcall" fn AVIStreamOpenFromFileA(out: *mut u32, file: *const i8, kind: u32, n: i32, _mode: u32, _clsid: u32) -> u32 {
    if out.is_null() || file.is_null() {
        return AVIERR_FILEOPEN;
    }
    *out = 0;
    let win = std::ffi::CStr::from_ptr(file).to_string_lossy().into_owned();
    let Some(avi) = std::fs::read(super::files::host_path(&win)).ok().and_then(parse_avi) else {
        crate::diag::warn(format!("AVIStreamOpenFromFileA: cannot open {win}"));
        return AVIERR_FILEOPEN;
    };
    let file = std::sync::Arc::new(avi);
    let Some(index) = file.streams.iter().enumerate().filter(|(_, s)| kind == 0 || s.kind() == kind).nth(n.max(0) as usize).map(|(i, _)| i)
    else {
        return AVIERR_NODATA;
    };
    let h = NEXT.fetch_add(1, Ordering::Relaxed);
    STREAMS.lock().unwrap().insert(h, OpenStream { file, index });
    *out = h;
    0
}

fn with_stream<R>(h: u32, f: impl FnOnce(&AviFile, &Stream) -> R) -> Option<R> {
    let s = STREAMS.lock().unwrap();
    let o = s.get(&h)?;
    Some(f(&o.file, &o.file.streams[o.index]))
}

#[allow(non_snake_case)]
extern "stdcall" fn AVIStreamRelease(h: u32) -> u32 {
    STREAMS.lock().unwrap().remove(&h);
    0
}

#[allow(non_snake_case)]
extern "stdcall" fn AVIStreamStart(_h: u32) -> i32 {
    0
}

#[allow(non_snake_case)]
extern "stdcall" fn AVIStreamLength(h: u32) -> i32 {
    with_stream(h, |_, s| s.length() as i32).unwrap_or(-1)
}

/// AVISTREAMINFOA (140 bytes), filled from the `strh` header.
#[allow(non_snake_case)]
unsafe extern "stdcall" fn AVIStreamInfoA(h: u32, out: *mut u8, size: i32) -> u32 {
    let Some(info) = with_stream(h, |_, s| {
        let mut i = [0u8; 140];
        let mut put = |o: usize, v: u32| i[o..o + 4].copy_from_slice(&v.to_le_bytes());
        let hd = &s.header;
        put(0, rd32(hd, 0)); // fccType
        put(4, rd32(hd, 4)); // fccHandler
        put(8, rd32(hd, 8)); // dwFlags
        put(16, rd32(hd, 12)); // wPriority, wLanguage
        put(20, rd32(hd, 20)); // dwScale
        put(24, rd32(hd, 24)); // dwRate
        put(28, rd32(hd, 28)); // dwStart
        put(32, s.length() as u32); // dwLength
        put(36, rd32(hd, 16)); // dwInitialFrames
        put(40, rd32(hd, 36)); // dwSuggestedBufferSize
        put(44, rd32(hd, 40)); // dwQuality
        put(48, if s.byte_stream() { s.block_align() as u32 } else { rd32(hd, 44) }); // dwSampleSize
        let (w, ht) = if s.kind() == fourcc(b"vids") { (rd32(&s.format, 4), rd32(&s.format, 8)) } else { (0, 0) };
        put(60, w); // rcFrame.right
        put(64, ht); // rcFrame.bottom
        i
    }) else {
        return AVIERR_NODATA;
    };
    let n = (size.max(0) as usize).min(info.len());
    std::ptr::copy_nonoverlapping(info.as_ptr(), out, n);
    0
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn AVIStreamReadFormat(h: u32, _pos: i32, out: *mut u8, size: *mut i32) -> u32 {
    let Some(fmt) = with_stream(h, |_, s| s.format.clone()) else { return AVIERR_NODATA };
    if size.is_null() {
        return AVIERR_NODATA;
    }
    if out.is_null() {
        *size = fmt.len() as i32;
        return 0;
    }
    let n = (*size).max(0) as usize;
    std::ptr::copy_nonoverlapping(fmt.as_ptr(), out, n.min(fmt.len()));
    *size = fmt.len() as i32;
    if n < fmt.len() { AVIERR_BUFFERTOOSMALL } else { 0 }
}

/// Video: one frame per sample. Audio: `samples` blocks from sample `start`, across chunks.
#[allow(non_snake_case)]
unsafe extern "stdcall" fn AVIStreamRead(h: u32, start: i32, samples: i32, buf: *mut u8, size: i32, bytes_out: *mut i32, samples_out: *mut i32) -> u32 {
    let r = with_stream(h, |f, s| {
        let start = start.max(0) as usize;
        let piece: Vec<u8> = if s.byte_stream() {
            let align = s.block_align();
            let total = s.total_bytes();
            let from = (start * align).min(total);
            let want = if samples < 0 { size.max(0) as usize / align * align } else { samples as usize * align };
            let want = if buf.is_null() { want } else { want.min(size.max(0) as usize / align * align) };
            let mut out = Vec::with_capacity(want);
            let mut skip = from;
            for &(off, len) in &s.chunks {
                if out.len() >= want {
                    break;
                }
                if skip >= len {
                    skip -= len;
                    continue;
                }
                let take = (len - skip).min(want - out.len());
                out.extend_from_slice(&f.data[off + skip..off + skip + take]);
                skip = 0;
            }
            out
        } else {
            match s.chunks.get(start) {
                Some(&(off, len)) => f.data[off..off + len].to_vec(),
                None => return Err(AVIERR_NODATA),
            }
        };
        let count = if s.byte_stream() { piece.len() / s.block_align() } else { 1 };
        Ok((piece, count))
    });
    let (piece, count) = match r {
        Some(Ok(v)) => v,
        Some(Err(e)) => return e,
        None => return AVIERR_NODATA,
    };
    if !buf.is_null() {
        if piece.len() > size.max(0) as usize {
            if !bytes_out.is_null() {
                *bytes_out = piece.len() as i32;
            }
            return AVIERR_BUFFERTOOSMALL;
        }
        std::ptr::copy_nonoverlapping(piece.as_ptr(), buf, piece.len());
    }
    if !bytes_out.is_null() {
        *bytes_out = piece.len() as i32;
    }
    if !samples_out.is_null() {
        *samples_out = count as i32;
    }
    0
}

// ---- codecs (msvfw32 IC*): only Cinepak decompression ------------------------------------------

static DECODERS: Mutex<BTreeMap<u32, Cinepak>> = Mutex::new(BTreeMap::new());

const ICERR_OK: i32 = 0;
const ICERR_BADFORMAT: i32 = -2;
const ICERR_ERROR: i32 = -100;

/// BITMAPINFOHEADER compression of the input: Cinepak ('cvid', any case).
unsafe fn is_cinepak(bih: *const u8) -> bool {
    !bih.is_null() && std::ptr::read_unaligned(bih.add(16) as *const u32).to_ascii_lowercase_fourcc() == fourcc(b"cvid")
}

trait LowerFourcc {
    fn to_ascii_lowercase_fourcc(self) -> u32;
}
impl LowerFourcc for u32 {
    fn to_ascii_lowercase_fourcc(self) -> u32 {
        u32::from_le_bytes(self.to_le_bytes().map(|b| b.to_ascii_lowercase()))
    }
}

fn new_decoder() -> u32 {
    let h = NEXT.fetch_add(1, Ordering::Relaxed);
    DECODERS.lock().unwrap().insert(h, Cinepak::default());
    h
}

#[allow(non_snake_case)]
unsafe extern "stdcall" fn ICLocate(_kind: u32, _handler: u32, input: *const u8, _out: *const u8, _flags: u16) -> u32 {
    if is_cinepak(input) { new_decoder() } else { 0 }
}

#[allow(non_snake_case)]
extern "stdcall" fn ICOpen(_kind: u32, handler: u32, _mode: u32) -> u32 {
    if handler.to_ascii_lowercase_fourcc() == fourcc(b"cvid") { new_decoder() } else { 0 }
}

/// Codec enumeration: only Cinepak exists (ICINFO: dwSize, fccType, fccHandler, dwFlags, ...).
#[allow(non_snake_case)]
unsafe extern "stdcall" fn ICInfo(kind: u32, handler: u32, info: *mut u32) -> i32 {
    let cvid = fourcc(b"cvid");
    let found = (handler == 0 || handler.to_ascii_lowercase_fourcc() == cvid) && (kind == 0 || kind == fourcc(b"vidc"));
    if !found || info.is_null() {
        return 0;
    }
    std::ptr::write_bytes(info as *mut u8, 0, 568); // sizeof(ICINFO)
    *info = 568;
    *info.add(1) = fourcc(b"vidc");
    *info.add(2) = cvid;
    1
}

#[allow(non_snake_case)]
extern "stdcall" fn ICClose(h: u32) -> u32 {
    DECODERS.lock().unwrap().remove(&h);
    0
}

/// DECOMPRESS_QUERY/BEGIN/END and friends: accepted; Cinepak needs no setup.
#[allow(non_snake_case)]
extern "stdcall" fn ICSendMessage(h: u32, _msg: u32, _a: u32, _b: u32) -> i32 {
    if DECODERS.lock().unwrap().contains_key(&h) { ICERR_OK } else { ICERR_ERROR }
}

/// cdecl (VFWAPIV), unlike the rest of the API. Writes the frame to `out` in the output format
/// (8-bit grey is not supported; 16, 24 and 32 bits, bottom-up unless the height is negative).
#[allow(non_snake_case)]
unsafe extern "C" fn ICDecompress(h: u32, _flags: u32, in_fmt: *const u8, input: *const u8, out_fmt: *const u8, out: *mut u8) -> i32 {
    if in_fmt.is_null() || input.is_null() || out_fmt.is_null() || out.is_null() {
        return ICERR_BADFORMAT;
    }
    let size = std::ptr::read_unaligned(in_fmt.add(20) as *const u32) as usize;
    let data = std::slice::from_raw_parts(input, size);
    let r32 = |p: *const u8, o: usize| std::ptr::read_unaligned(p.add(o) as *const i32);
    let (ow, oh_raw) = (r32(out_fmt, 4), r32(out_fmt, 8));
    let bpp = std::ptr::read_unaligned(out_fmt.add(14) as *const u16) as usize;
    let compression = r32(out_fmt, 16) as u32;
    let mut decs = DECODERS.lock().unwrap();
    let Some(dec) = decs.get_mut(&h) else { return ICERR_ERROR };
    let (w, ht) = (r32(in_fmt, 4).unsigned_abs() as usize, r32(in_fmt, 8).unsigned_abs() as usize);
    dec.decode(data, w, ht);
    let (ow, oh) = (ow.unsigned_abs() as usize, oh_raw.unsigned_abs() as usize);
    let stride = (ow * bpp).div_ceil(32) * 4;
    // 16 bits: 555 (BI_RGB) or the masks given after the header (BI_BITFIELDS, usually 565).
    let rgb565 = bpp == 16 && compression == 3 && r32(out_fmt, 40) as u32 == 0xF800;
    for y in 0..oh.min(dec.h) {
        let row = if oh_raw > 0 { oh - 1 - y } else { y };
        let dst = out.add(row * stride);
        for x in 0..ow.min(dec.w) {
            let [r, g, b] = dec.rgb[y * dec.w + x];
            match bpp {
                16 => {
                    let v = if rgb565 {
                        ((r as u16 >> 3) << 11) | ((g as u16 >> 2) << 5) | (b as u16 >> 3)
                    } else {
                        ((r as u16 >> 3) << 10) | ((g as u16 >> 3) << 5) | (b as u16 >> 3)
                    };
                    std::ptr::write_unaligned(dst.add(x * 2) as *mut u16, v);
                }
                24 => {
                    *dst.add(x * 3) = b;
                    *dst.add(x * 3 + 1) = g;
                    *dst.add(x * 3 + 2) = r;
                }
                32 => std::ptr::write_unaligned(dst.add(x * 4) as *mut u32, u32::from_le_bytes([b, g, r, 0])),
                _ => return ICERR_BADFORMAT,
            }
        }
    }
    ICERR_OK
}

// ---- Cinepak decoder -------------------------------------------------------------------------

#[derive(Clone, Copy, Default)]
struct Entry {
    y: [u8; 4],
    u: i8,
    v: i8,
}

#[derive(Clone)]
struct Strip {
    v1: [Entry; 256],
    v4: [Entry; 256],
}

impl Default for Strip {
    fn default() -> Strip {
        Strip { v1: [Entry::default(); 256], v4: [Entry::default(); 256] }
    }
}

/// Keeps the previous frame (inter frames update only some blocks) and the codebooks of each strip.
#[derive(Default)]
pub struct Cinepak {
    w: usize,
    h: usize,
    rgb: Vec<[u8; 3]>,
    strips: Vec<Strip>,
}

fn be16(d: &[u8], o: usize) -> usize {
    d.get(o..o + 2).map_or(0, |b| u16::from_be_bytes([b[0], b[1]]) as usize)
}

fn be24(d: &[u8], o: usize) -> usize {
    d.get(o..o + 3).map_or(0, |b| ((b[0] as usize) << 16) | ((b[1] as usize) << 8) | b[2] as usize)
}

fn be32(d: &[u8], o: usize) -> u32 {
    d.get(o..o + 4).map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

fn to_rgb(y: u8, u: i8, v: i8) -> [u8; 3] {
    let (y, u, v) = (y as i32, u as i32, v as i32);
    let c = |x: i32| x.clamp(0, 255) as u8;
    [c(y + v * 2), c(y - u / 2 - v), c(y + u * 2)]
}

/// Bit reader for the 32-bit flag words that precede codebook entries and blocks.
struct Flags {
    word: u32,
    mask: u32,
}

impl Flags {
    /// Next flag bit, loading a new word from `d[*p..]` when the current one is used up.
    fn next(&mut self, d: &[u8], p: &mut usize) -> Option<bool> {
        self.mask >>= 1;
        if self.mask == 0 {
            if *p + 4 > d.len() {
                return None;
            }
            self.word = be32(d, *p);
            *p += 4;
            self.mask = 0x8000_0000;
        }
        Some(self.word & self.mask != 0)
    }
}

impl Cinepak {
    pub fn decode(&mut self, d: &[u8], w: usize, h: usize) {
        if self.w != w || self.h != h {
            *self = Cinepak { w, h, rgb: vec![[0; 3]; w * h], strips: Vec::new() };
        }
        if d.len() < 10 {
            return;
        }
        let frame_flags = d[0];
        let nstrips = be16(d, 8);
        let mut p = 10;
        let mut y0 = 0;
        for i in 0..nstrips {
            if p + 12 > d.len() {
                break;
            }
            let strip_size = be24(d, p + 1);
            let height = be16(d, p + 8);
            let end = (p + strip_size).min(d.len());
            if self.strips.len() <= i {
                self.strips.push(Strip::default());
            }
            // Without flag bit 0, each strip starts from the previous strip's codebooks.
            if i > 0 && frame_flags & 1 == 0 {
                self.strips[i] = self.strips[i - 1].clone();
            }
            let y1 = (y0 + height).min(h);
            let mut c = p + 12;
            while c + 4 <= end {
                let id = d[c];
                let size = be24(d, c + 1);
                if size < 4 {
                    break;
                }
                let body = &d[c + 4..(c + size).min(end)];
                match id {
                    0x20 | 0x21 | 0x24 | 0x25 => decode_codebook(&mut self.strips[i].v4, id, body),
                    0x22 | 0x23 | 0x26 | 0x27 => decode_codebook(&mut self.strips[i].v1, id, body),
                    0x30..=0x32 => self.decode_vectors(i, id, body, y0, y1),
                    _ => {}
                }
                c += size;
            }
            y0 = y1;
            p += strip_size.max(12);
        }
    }

    fn put(&mut self, x: usize, y: usize, c: [u8; 3]) {
        if x < self.w && y < self.h {
            self.rgb[y * self.w + x] = c;
        }
    }

    fn decode_vectors(&mut self, strip: usize, id: u8, d: &[u8], y0: usize, y1: usize) {
        let mut p = 0;
        let mut f = Flags { word: 0, mask: 1 };
        let (v1, v4) = (self.strips[strip].v1, self.strips[strip].v4);
        let mut y = y0;
        while y < y1 {
            let mut x = 0;
            while x < self.w {
                // 0x31: first a "block changed" bit.
                let update = id & 1 == 0 || match f.next(d, &mut p) {
                    Some(b) => b,
                    None => return,
                };
                if update {
                    // 0x32: always v1; otherwise one bit chooses v4 (1) or v1 (0).
                    let use_v4 = id & 2 == 0 && match f.next(d, &mut p) {
                        Some(b) => b,
                        None => return,
                    };
                    if use_v4 {
                        if p + 4 > d.len() {
                            return;
                        }
                        for (q, &(qx, qy)) in [(0, 0), (2, 0), (0, 2), (2, 2)].iter().enumerate() {
                            let e = v4[d[p + q] as usize];
                            for (k, &(kx, ky)) in [(0, 0), (1, 0), (0, 1), (1, 1)].iter().enumerate() {
                                self.put(x + qx + kx, y + qy + ky, to_rgb(e.y[k], e.u, e.v));
                            }
                        }
                        p += 4;
                    } else {
                        let Some(&i) = d.get(p) else { return };
                        p += 1;
                        let e = v1[i as usize];
                        for by in 0..4 {
                            for bx in 0..4 {
                                let c = to_rgb(e.y[(by / 2) * 2 + bx / 2], e.u, e.v);
                                self.put(x + bx, y + by, c);
                            }
                        }
                    }
                }
                x += 4;
            }
            y += 4;
        }
    }
}

/// Bit 0x04: grey entries (4 bytes, no colour); bit 0x01: only entries flagged in 32-bit words change.
fn decode_codebook(book: &mut [Entry; 256], id: u8, d: &[u8]) {
    let n = if id & 4 != 0 { 4 } else { 6 };
    let mut p = 0;
    let mut f = Flags { word: 0, mask: 1 };
    for e in book.iter_mut() {
        let change = id & 1 == 0 || match f.next(d, &mut p) {
            Some(b) => b,
            None => return,
        };
        if change {
            if p + n > d.len() {
                return;
            }
            e.y.copy_from_slice(&d[p..p + 4]);
            (e.u, e.v) = if n == 6 { (d[p + 4] as i8, d[p + 5] as i8) } else { (0, 0) };
            p += n;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cinepak_v1_block() {
        // one 4x4 frame, one strip: a full colour v1 codebook with entry 0 = grey 100, then 0x32 (all v1) block 0
        let mut f = vec![0u8, 0, 0, 0, 0, 4, 0, 4, 0, 1];
        let mut strip = vec![0x10u8, 0, 0, 0, 0, 0, 0, 0, 0, 4, 0, 4];
        let mut book = vec![0x22u8, 0, 0, 0];
        book.extend(std::iter::repeat_n(0u8, 256 * 6));
        book[4..10].copy_from_slice(&[100, 100, 100, 100, 0, 0]);
        let n = book.len();
        book[1..4].copy_from_slice(&(n as u32).to_be_bytes()[1..]);
        let vectors = vec![0x32u8, 0, 0, 5, 0];
        strip.extend(&book);
        strip.extend(&vectors);
        let s = strip.len();
        strip[1..4].copy_from_slice(&(s as u32).to_be_bytes()[1..]);
        f.extend(&strip);
        let mut dec = Cinepak::default();
        dec.decode(&f, 4, 4);
        assert!(dec.rgb.iter().all(|&c| c == [100, 100, 100]));
    }
}
