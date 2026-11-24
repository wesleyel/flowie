//! Just enough ZIP for share packages.

use std::io::{Read, Write};

use super::{damaged, PackageError};

/// Most entries a package may have.
const MAX_ENTRIES: usize = 4096;

const LOCAL: u32 = 0x0403_4b50;
const CENTRAL: u32 = 0x0201_4b50;
const END: u32 = 0x0605_4b50;
/// Bit 11: names are UTF-8.
const UTF8: u16 = 1 << 11;
/// 1980-01-01, the earliest DOS date: entries carry no meaningful time.
const DOS_DATE: u16 = (1 << 5) | 1;

/// (name, data, deflate it).
pub fn write(entries: &[(&str, &[u8], bool)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for &(name, data, deflate) in entries {
        let crc = crc32fast::hash(data);
        let packed = if deflate {
            let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
            enc.write_all(data).expect("writing to memory");
            enc.finish().expect("writing to memory")
        } else {
            data.to_vec()
        };
        let method: u16 = if deflate { 8 } else { 0 };
        let offset = out.len() as u32;
        let fields = |v: &mut Vec<u8>| {
            v.extend(20u16.to_le_bytes()); // version needed: 2.0
            v.extend(UTF8.to_le_bytes());
            v.extend(method.to_le_bytes());
            v.extend(0u16.to_le_bytes()); // time
            v.extend(DOS_DATE.to_le_bytes());
            v.extend(crc.to_le_bytes());
            v.extend((packed.len() as u32).to_le_bytes());
            v.extend((data.len() as u32).to_le_bytes());
            v.extend((name.len() as u16).to_le_bytes());
            v.extend(0u16.to_le_bytes()); // extra field length
        };
        out.extend(LOCAL.to_le_bytes());
        fields(&mut out);
        out.extend(name.as_bytes());
        out.extend(&packed);

        central.extend(CENTRAL.to_le_bytes());
        central.extend((3u16 << 8 | 20).to_le_bytes()); // made by: Unix, 2.0
        fields(&mut central);
        central.extend(0u16.to_le_bytes()); // comment length
        central.extend(0u16.to_le_bytes()); // disk
        central.extend(0u16.to_le_bytes()); // internal attributes
        central.extend((0o100644u32 << 16).to_le_bytes()); // a regular file, rw-r--r--
        central.extend(offset.to_le_bytes());
        central.extend(name.as_bytes());
    }
    let central_offset = out.len() as u32;
    out.extend(&central);
    out.extend(END.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out.extend((entries.len() as u16).to_le_bytes());
    out.extend((entries.len() as u16).to_le_bytes());
    out.extend((central.len() as u32).to_le_bytes());
    out.extend(central_offset.to_le_bytes());
    out.extend(0u16.to_le_bytes()); // comment length
    out
}

pub struct Entry {
    pub name: String,
    method: u16,
    crc: u32,
    packed: u64,
    size: u64,
    offset: u64,
}

pub struct Archive<'a> {
    bytes: &'a [u8],
    /// In central directory order.
    pub entries: Vec<Entry>,
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

impl<'a> Archive<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Archive<'a>, PackageError> {
        let not = || PackageError::NotAPackage;
        if u32_at(bytes, 0) != Some(LOCAL) {
            return Err(not());
        }
        // The end record is the last 22 bytes plus a comment of up to 64 KiB.
        let last = bytes.len().checked_sub(22).ok_or_else(not)?;
        let end = (last.saturating_sub(0xFFFF)..=last).rev().find(|&i| u32_at(bytes, i) == Some(END)).ok_or_else(not)?;
        let count = u16_at(bytes, end + 10).ok_or_else(not)? as usize;
        let dir_offset = u32_at(bytes, end + 16).ok_or_else(not)? as usize;
        if count > MAX_ENTRIES {
            return Err(PackageError::TooLarge);
        }
        let mut entries = Vec::with_capacity(count);
        let mut at = dir_offset;
        for _ in 0..count {
            let field = |off: usize| u16_at(bytes, at + off).ok_or_else(|| damaged("central directory cut short"));
            let field32 = |off: usize| u32_at(bytes, at + off).ok_or_else(|| damaged("central directory cut short"));
            if field32(0)? != CENTRAL {
                return Err(damaged("bad central directory"));
            }
            let flags = field(8)?;
            let (name_len, extra_len, comment_len) = (field(28)? as usize, field(30)? as usize, field(32)? as usize);
            let name = bytes.get(at + 46..at + 46 + name_len).ok_or_else(|| damaged("central directory cut short"))?;
            if flags & 1 != 0 {
                return Err(damaged("encrypted entry"));
            }
            entries.push(Entry {
                name: String::from_utf8_lossy(name).into_owned(),
                method: field(10)?,
                crc: field32(16)?,
                packed: field32(20)? as u64,
                size: field32(24)? as u64,
                offset: field32(42)? as u64,
            });
            at += 46 + name_len + extra_len + comment_len;
        }
        Ok(Archive { bytes, entries })
    }

    /// An entry's contents, counted against `budget` (bytes left to unpack).
    pub fn read(&self, e: &Entry, budget: &mut u64) -> Result<Vec<u8>, PackageError> {
        if e.size > *budget {
            return Err(PackageError::TooLarge);
        }
        let short = || damaged(format!("{} cut short", e.name));
        let at = usize::try_from(e.offset).map_err(|_| short())?;
        if u32_at(self.bytes, at) != Some(LOCAL) {
            return Err(damaged(format!("{}: bad local header", e.name)));
        }
        let start =
            at + 30 + u16_at(self.bytes, at + 26).ok_or_else(short)? as usize + u16_at(self.bytes, at + 28).ok_or_else(short)? as usize;
        let data = self.bytes.get(start..start + e.packed as usize).ok_or_else(short)?;
        let out = match e.method {
            0 => data.to_vec(),
            8 => {
                let mut out = Vec::with_capacity(e.size.min(1 << 20) as usize);
                flate2::read::DeflateDecoder::new(data)
                    .take(e.size + 1)
                    .read_to_end(&mut out)
                    .map_err(|err| damaged(format!("{}: {err}", e.name)))?;
                out
            }
            m => return Err(damaged(format!("{}: compression method {m}", e.name))),
        };
        if out.len() as u64 != e.size || crc32fast::hash(&out) != e.crc {
            return Err(damaged(format!("{} does not match its checksum", e.name)));
        }
        *budget -= e.size;
        Ok(out)
    }
}
