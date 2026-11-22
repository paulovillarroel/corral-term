//! Unzip, for Ark's releases (published only as .zip): the central directory,
//! stored and deflated entries, CRC-32 checked. Inflate follows the classic
//! "puff" decoder (zlib's reference): fixed and dynamic Huffman blocks.

use crate::i18n::tr;
use std::path::{Component, Path};

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    buf: u32,
    cnt: u32,
}

struct Huffman {
    count: [u16; 16],
    symbol: Vec<u16>,
}

const LBASE: [u16; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LEXT: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DBASE: [u16; 30] = [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049,
                          3073, 4097, 6145, 8193, 12289, 16385, 24577];
const DEXT: [u8; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];
const ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

impl Bits<'_> {
    fn bits(&mut self, need: u32) -> Result<u32, String> {
        let mut val = self.buf;
        while self.cnt < need {
            let byte = *self.data.get(self.pos).ok_or_else(|| tr("zip: the compressed data is incomplete", "zip: datos comprimidos incompletos"))?;
            self.pos += 1;
            val |= u32::from(byte) << self.cnt;
            self.cnt += 8;
        }
        self.buf = if need >= 32 { 0 } else { val >> need };
        self.cnt -= need;
        Ok(if need == 0 { 0 } else { val & ((1u32 << need) - 1) })
    }

    fn decode(&mut self, h: &Huffman) -> Result<usize, String> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= self.bits(1)? as i32;
            let count = i32::from(h.count[len]);
            if code - count < first {
                return Ok(usize::from(h.symbol[(index + (code - first)) as usize]));
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err(tr("zip: invalid Huffman code", "zip: código de Huffman inválido").into())
    }
}

fn huffman(lengths: &[u8]) -> Huffman {
    let mut h = Huffman { count: [0; 16], symbol: vec![0; lengths.len()] };
    for &l in lengths {
        h.count[usize::from(l)] += 1;
    }
    let mut offs = [0u16; 16];
    for len in 1..15 {
        offs[len + 1] = offs[len] + h.count[len];
    }
    for (sym, &l) in lengths.iter().enumerate() {
        if l != 0 {
            h.symbol[usize::from(offs[usize::from(l)])] = sym as u16;
            offs[usize::from(l)] += 1;
        }
    }
    h
}

fn codes(s: &mut Bits, out: &mut Vec<u8>, lencode: &Huffman, distcode: &Huffman) -> Result<(), String> {
    loop {
        let sym = s.decode(lencode)?;
        if sym < 256 {
            out.push(sym as u8);
        } else if sym == 256 {
            return Ok(());
        } else {
            let i = sym - 257;
            if i >= 29 {
                return Err(tr("zip: invalid length", "zip: largo inválido").into());
            }
            let len = usize::from(LBASE[i]) + s.bits(u32::from(LEXT[i]))? as usize;
            let d = s.decode(distcode)?;
            if d >= 30 {
                return Err(tr("zip: invalid distance", "zip: distancia inválida").into());
            }
            let dist = usize::from(DBASE[d]) + s.bits(u32::from(DEXT[d]))? as usize;
            if dist > out.len() {
                return Err(tr("zip: distance out of range", "zip: distancia fuera de rango").into());
            }
            let start = out.len() - dist;
            for k in 0..len {
                let b = out[start + k];
                out.push(b);
            }
        }
    }
}

pub fn inflate(data: &[u8], size_hint: usize) -> Result<Vec<u8>, String> {
    let mut s = Bits { data, pos: 0, buf: 0, cnt: 0 };
    let mut out = Vec::with_capacity(size_hint);
    loop {
        let last = s.bits(1)?;
        match s.bits(2)? {
            0 => {
                // Stored: realign to a byte, then LEN and its complement.
                s.buf = 0;
                s.cnt = 0;
                let hdr = data.get(s.pos..s.pos + 4).ok_or_else(|| tr("zip: block without a header", "zip: bloque sin cabecera"))?;
                let len = usize::from(u16::from_le_bytes([hdr[0], hdr[1]]));
                if u16::from_le_bytes([hdr[2], hdr[3]]) != !(len as u16) {
                    return Err(tr("zip: invalid stored block", "zip: bloque guardado inválido").into());
                }
                s.pos += 4;
                out.extend_from_slice(data.get(s.pos..s.pos + len).ok_or_else(|| tr("zip: incomplete block", "zip: bloque incompleto"))?);
                s.pos += len;
            }
            1 => {
                let mut lengths = [0u8; 288];
                lengths[..144].fill(8);
                lengths[144..256].fill(9);
                lengths[256..280].fill(7);
                lengths[280..].fill(8);
                codes(&mut s, &mut out, &huffman(&lengths), &huffman(&[5u8; 30]))?;
            }
            2 => {
                let nlen = s.bits(5)? as usize + 257;
                let ndist = s.bits(5)? as usize + 1;
                let ncode = s.bits(4)? as usize + 4;
                let mut lengths = [0u8; 320];
                for &o in ORDER.iter().take(ncode) {
                    lengths[o] = s.bits(3)? as u8;
                }
                let lencode = huffman(&lengths[..19]);
                let mut index = 0;
                let mut lengths = [0u8; 320];
                while index < nlen + ndist {
                    let sym = s.decode(&lencode)?;
                    if sym < 16 {
                        lengths[index] = sym as u8;
                        index += 1;
                    } else {
                        let (value, repeat) = match sym {
                            16 => {
                                if index == 0 {
                                    return Err(tr("zip: repeat with no previous length", "zip: repetición sin largo previo").into());
                                }
                                (lengths[index - 1], 3 + s.bits(2)? as usize)
                            }
                            17 => (0, 3 + s.bits(3)? as usize),
                            _ => (0, 11 + s.bits(7)? as usize),
                        };
                        if index + repeat > nlen + ndist {
                            return Err(tr("zip: too many lengths", "zip: demasiados largos").into());
                        }
                        lengths[index..index + repeat].fill(value);
                        index += repeat;
                    }
                }
                let lencode = huffman(&lengths[..nlen]);
                let distcode = huffman(&lengths[nlen..nlen + ndist]);
                codes(&mut s, &mut out, &lencode, &distcode)?;
            }
            _ => return Err(tr("zip: invalid block type", "zip: tipo de bloque inválido").into()),
        }
        if last == 1 {
            return Ok(out);
        }
    }
}

fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (i, slot) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        *slot = c;
    }
    !data.iter().fold(!0u32, |c, &b| table[((c ^ u32::from(b)) & 0xff) as usize] ^ (c >> 8))
}

fn u16_at(d: &[u8], i: usize) -> usize {
    d.get(i..i + 2).map(|b| usize::from(u16::from_le_bytes([b[0], b[1]]))).unwrap_or(0)
}

fn u32_at(d: &[u8], i: usize) -> usize {
    d.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize).unwrap_or(0)
}

/// Extract every file of the zip into `target` (never outside it). Returns the names.
pub fn extract(zip: &[u8], target: &Path) -> Result<Vec<String>, String> {
    let eocd = (0..zip.len().saturating_sub(21)).rev().take(70_000)
        .find(|&i| zip[i..i + 4] == [0x50, 0x4b, 0x05, 0x06]).ok_or_else(|| tr("zip: couldn't find the index", "zip: no encontré el índice"))?;
    let entries = u16_at(zip, eocd + 10);
    let mut p = u32_at(zip, eocd + 16);
    let mut names = Vec::new();
    for _ in 0..entries {
        if zip.get(p..p + 4) != Some(&[0x50, 0x4b, 0x01, 0x02]) {
            return Err(tr("zip: damaged index", "zip: índice dañado").into());
        }
        let method = u16_at(zip, p + 10);
        let crc = u32_at(zip, p + 16) as u32;
        let (csize, usize_) = (u32_at(zip, p + 20), u32_at(zip, p + 24));
        let (nlen, elen, clen) = (u16_at(zip, p + 28), u16_at(zip, p + 30), u16_at(zip, p + 32));
        let local = u32_at(zip, p + 42);
        let name = String::from_utf8_lossy(zip.get(p + 46..p + 46 + nlen).ok_or_else(|| tr("zip: damaged name", "zip: nombre dañado"))?).into_owned();
        p += 46 + nlen + elen + clen;
        let rel = Path::new(&name);
        if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
            continue; // absolute paths or "..": never outside the target
        }
        let out_path = target.join(rel);
        if name.ends_with('/') {
            std::fs::create_dir_all(&out_path).map_err(|e| e.to_string())?;
            continue;
        }
        let data_at = local + 30 + u16_at(zip, local + 26) + u16_at(zip, local + 28);
        let raw = zip.get(data_at..data_at + csize).ok_or_else(|| tr("zip: incomplete file", "zip: archivo incompleto"))?;
        let bytes = match method {
            0 => raw.to_vec(),
            8 => inflate(raw, usize_)?,
            m => return Err(trf!("zip: compression method {m} isn't supported", "zip: método de compresión {m} no soportado")),
        };
        if bytes.len() != usize_ || crc32(&bytes) != crc {
            return Err(trf!("zip: {name} arrived damaged (CRC)", "zip: {name} llegó dañado (CRC)"));
        }
        if let Some(d) = out_path.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        std::fs::write(&out_path, bytes).map_err(|e| format!("{}: {e}", out_path.display()))?;
        names.push(name);
    }
    Ok(names)
}

/// Decompress a .gz file (one member, as release downloads are).
pub fn gunzip(data: &[u8]) -> Result<Vec<u8>, String> {
    if data.len() < 18 || data[0] != 0x1f || data[1] != 0x8b || data[2] != 8 {
        return Err(tr("not a gzip file", "no es un archivo gzip").into());
    }
    let flags = data[3];
    let mut p = 10;
    if flags & 4 != 0 {
        p += 2 + u16_at(data, p); // FEXTRA
    }
    for bit in [8u8, 16] {
        if flags & bit != 0 {
            // FNAME, FCOMMENT: zero-terminated
            p += data.get(p..).and_then(|d| d.iter().position(|&b| b == 0)).ok_or_else(|| tr("damaged gzip", "gzip dañado"))? + 1;
        }
    }
    if flags & 2 != 0 {
        p += 2; // FHCRC
    }
    let size = u32_at(data, data.len() - 4);
    let out = inflate(data.get(p..data.len() - 8).ok_or_else(|| tr("damaged gzip", "gzip dañado"))?, size)?;
    if crc32(&out) != u32_at(data, data.len() - 8) as u32 {
        return Err(tr("gzip: arrived damaged (CRC)", "gzip: llegó dañado (CRC)").into());
    }
    Ok(out)
}

/// The contents of the first regular file named `name` (whatever its folder) in a tar archive.
pub fn tar_member(tar: &[u8], name: &str) -> Option<Vec<u8>> {
    let mut p = 0;
    while p + 512 <= tar.len() {
        let h = &tar[p..p + 512];
        if h.iter().all(|&b| b == 0) {
            return None;
        }
        let field = |a: usize, b: usize| String::from_utf8_lossy(&h[a..b]).trim_end_matches('\0').to_string();
        let mut path = field(0, 100);
        let prefix = field(345, 500);
        if !prefix.is_empty() && &h[257..262] == b"ustar" {
            path = format!("{prefix}/{path}");
        }
        let size = usize::from_str_radix(field(124, 136).trim(), 8).unwrap_or(0);
        let kind = h[156];
        let start = p + 512;
        if (kind == b'0' || kind == 0) && Path::new(&path).file_name().is_some_and(|n| n == name) {
            return tar.get(start..start + size).map(<[u8]>::to_vec);
        }
        p = start + size.div_ceil(512) * 512;
    }
    None
}
