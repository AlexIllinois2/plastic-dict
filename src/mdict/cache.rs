//! 词典索引磁盘缓存 (替代 TS 版的 JSON 缓存, 格式更紧凑)
//!
//! 文件: <exe>/cache/index/<basename>.<fnv1a(path)>.pdc (便携包模式, 见 paths.rs)
//! 失效条件: 文件 size / mtime 变化, 或缓存格式版本不匹配

use crate::mdict::parse::{Ext, KeyHeader, KeyInfoItem, KeyWordItem, MdictMeta, ParsedIndex, RecordHeader, RecordInfo, TextEncoding};
use crate::util::fnv1a_hex;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

const MAGIC: &[u8] = b"PDIDX02";
const CACHE_FORMAT: u32 = 2;

fn cache_path_for(file: &str) -> PathBuf {
    let dir = crate::paths::index_cache_dir();
    let base = Path::new(file)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "dict".into());
    dir.join(format!("{base}.{}.pdc", fnv1a_hex(file)))
}

fn file_stamp(path: &str) -> Option<(u64, u64)> {
    let st = fs::metadata(path).ok()?;
    Some((st.len(), st.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as u64))
}

// ---------------------------------------------------------------- 写

fn w_u8(out: &mut Vec<u8>, v: u8) {
    out.push(v);
}
fn w_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn w_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn w_i32(out: &mut Vec<u8>, v: i32) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn w_f64(out: &mut Vec<u8>, v: f64) {
    out.extend_from_slice(&v.to_bits().to_le_bytes());
}
fn w_str(out: &mut Vec<u8>, s: &str) {
    w_u32(out, s.len() as u32);
    out.extend_from_slice(s.as_bytes());
}

pub fn save_index(index: &ParsedIndex, file: &str) {
    let Some((size, mtime)) = file_stamp(file) else { return };
    let path = cache_path_for(file);
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    w_u32(&mut out, CACHE_FORMAT);
    w_u64(&mut out, size);
    w_u64(&mut out, mtime);
    // meta
    w_u8(&mut out, match index.meta.ext {
        Ext::Mdx => 0,
        Ext::Mdd => 1,
    });
    w_f64(&mut out, index.meta.version);
    w_u32(&mut out, index.meta.num_width as u32);
    w_u8(&mut out, match index.meta.encoding {
        TextEncoding::Utf8 => 0,
        TextEncoding::Utf16Le => 1,
        TextEncoding::Gb18030 => 2,
        TextEncoding::Big5 => 3,
    });
    w_i32(&mut out, index.meta.encrypt);
    // header
    w_u32(&mut out, index.header.len() as u32);
    for (k, v) in &index.header {
        w_str(&mut out, k);
        w_str(&mut out, v);
    }
    // key header
    let kh = &index.key_header;
    w_u64(&mut out, kh.keyword_blocks_num);
    w_u64(&mut out, kh.keyword_num);
    w_u64(&mut out, kh.key_info_unpack_size);
    w_u64(&mut out, kh.key_info_packed_size);
    w_u64(&mut out, kh.keyword_block_packed_size);
    // key info list
    w_u32(&mut out, index.key_info_list.len() as u32);
    for ki in &index.key_info_list {
        w_str(&mut out, &ki.first_key);
        w_str(&mut out, &ki.last_key);
        w_u64(&mut out, ki.key_block_pack_size);
        w_u64(&mut out, ki.key_block_pack_acc);
        w_u64(&mut out, ki.key_block_unpack_size);
        w_u64(&mut out, ki.key_block_unpack_acc);
        w_u64(&mut out, ki.key_block_entries_num);
        w_u64(&mut out, ki.key_block_entries_num_acc);
    }
    // keyword list
    w_u32(&mut out, index.keyword_list.len() as u32);
    for kw in &index.keyword_list {
        w_str(&mut out, &kw.key_text);
        w_u64(&mut out, kw.record_start);
        w_u64(&mut out, kw.record_end);
        w_u32(&mut out, kw.key_block_idx as u32);
    }
    // record header
    let rh = &index.record_header;
    w_u64(&mut out, rh.record_blocks_num);
    w_u64(&mut out, rh.entries_num);
    w_u64(&mut out, rh.record_info_comp_size);
    w_u64(&mut out, rh.record_block_comp_size);
    // record info list
    w_u32(&mut out, index.record_info_list.len() as u32);
    for ri in &index.record_info_list {
        w_u64(&mut out, ri.pack_size);
        w_u64(&mut out, ri.pack_acc);
        w_u64(&mut out, ri.unpack_size);
        w_u64(&mut out, ri.unpack_acc);
    }
    w_u64(&mut out, index.record_block_start_offset);

    let tmp = path.with_extension("pdc.tmp");
    if fs::write(&tmp, &out).is_ok() {
        let _ = fs::rename(&tmp, &path);
    }
}

// ---------------------------------------------------------------- 读

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
    fn u8(&mut self) -> Option<u8> {
        let v = *self.data.get(self.pos)?;
        self.pos += 1;
        Some(v)
    }
    fn u32(&mut self) -> Option<u32> {
        let b = self.data.get(self.pos..self.pos + 4)?;
        self.pos += 4;
        Some(u32::from_le_bytes(b.try_into().ok()?))
    }
    fn u64(&mut self) -> Option<u64> {
        let b = self.data.get(self.pos..self.pos + 8)?;
        self.pos += 8;
        Some(u64::from_le_bytes(b.try_into().ok()?))
    }
    fn i32(&mut self) -> Option<i32> {
        let b = self.data.get(self.pos..self.pos + 4)?;
        self.pos += 4;
        Some(i32::from_le_bytes(b.try_into().ok()?))
    }
    fn f64(&mut self) -> Option<f64> {
        Some(f64::from_bits(self.u64()?))
    }
    fn string(&mut self) -> Option<String> {
        let len = self.u32()? as usize;
        let b = self.data.get(self.pos..self.pos + len)?;
        self.pos += len;
        Some(String::from_utf8_lossy(b).into_owned())
    }
    fn done(&self) -> bool {
        self.pos >= self.data.len()
    }
}

pub fn load_index(file: &str) -> Option<ParsedIndex> {
    let (size, mtime) = file_stamp(file)?;
    let path = cache_path_for(file);
    let mut buf = Vec::new();
    fs::File::open(&path).ok()?.read_to_end(&mut buf).ok()?;
    let mut r = Reader::new(&buf);
    if r.data.get(..MAGIC.len())? != MAGIC {
        return None;
    }
    r.pos = MAGIC.len();
    if r.u32()? != CACHE_FORMAT {
        return None;
    }
    if r.u64()? != size || r.u64()? != mtime {
        return None;
    }
    let ext = match r.u8()? {
        0 => Ext::Mdx,
        1 => Ext::Mdd,
        _ => return None,
    };
    let version = r.f64()?;
    let num_width = r.u32()? as usize;
    let encoding = match r.u8()? {
        0 => TextEncoding::Utf8,
        1 => TextEncoding::Utf16Le,
        2 => TextEncoding::Gb18030,
        3 => TextEncoding::Big5,
        _ => return None,
    };
    let encrypt = r.i32()?;
    let header_len = r.u32()? as usize;
    let mut header = Vec::with_capacity(header_len);
    for _ in 0..header_len {
        let k = r.string()?;
        let v = r.string()?;
        header.push((k, v));
    }
    let mut key_header = KeyHeader::default();
    key_header.keyword_blocks_num = r.u64()?;
    key_header.keyword_num = r.u64()?;
    key_header.key_info_unpack_size = r.u64()?;
    key_header.key_info_packed_size = r.u64()?;
    key_header.keyword_block_packed_size = r.u64()?;
    let ki_len = r.u32()? as usize;
    let mut key_info_list = Vec::with_capacity(ki_len);
    for _ in 0..ki_len {
        key_info_list.push(KeyInfoItem {
            first_key: r.string()?,
            last_key: r.string()?,
            key_block_pack_size: r.u64()?,
            key_block_pack_acc: r.u64()?,
            key_block_unpack_size: r.u64()?,
            key_block_unpack_acc: r.u64()?,
            key_block_entries_num: r.u64()?,
            key_block_entries_num_acc: r.u64()?,
        });
    }
    let kw_len = r.u32()? as usize;
    let mut keyword_list = Vec::with_capacity(kw_len);
    for _ in 0..kw_len {
        keyword_list.push(KeyWordItem {
            key_text: r.string()?,
            record_start: r.u64()?,
            record_end: r.u64()?,
            key_block_idx: r.u32()? as usize,
        });
    }
    let mut record_header = RecordHeader::default();
    record_header.record_blocks_num = r.u64()?;
    record_header.entries_num = r.u64()?;
    record_header.record_info_comp_size = r.u64()?;
    record_header.record_block_comp_size = r.u64()?;
    let ri_len = r.u32()? as usize;
    let mut record_info_list = Vec::with_capacity(ri_len);
    for _ in 0..ri_len {
        record_info_list.push(RecordInfo {
            pack_size: r.u64()?,
            pack_acc: r.u64()?,
            unpack_size: r.u64()?,
            unpack_acc: r.u64()?,
        });
    }
    let record_block_start_offset = r.u64()?;
    if !r.done() {
        return None;
    }
    Some(ParsedIndex {
        meta: MdictMeta {
            fname: file.to_string(),
            ext,
            version,
            num_width,
            encoding,
            encrypt,
        },
        header,
        key_header,
        key_info_list,
        keyword_list,
        record_header,
        record_info_list,
        record_block_start_offset,
    })
}

pub fn purge_file_cache(file: &str) {
    let path = cache_path_for(file);
    let _ = fs::remove_file(path);
}

/// 清空整个索引缓存目录, 返回删除的文件数
pub fn purge_all_cache() -> usize {
    let dir = crate::paths::index_cache_dir();
    let mut n = 0;
    if let Ok(entries) = fs::read_dir(&dir) {
        for e in entries.flatten() {
            if fs::remove_file(e.path()).is_ok() {
                n += 1;
            }
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mdict::parse::parse_index;

    fn fixture_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
    }

    #[test]
    fn cache_roundtrip() {
        let mdx = fixture_dir().join("mini.mdx");
        if !mdx.exists() {
            return; // fixtures 未生成时跳过
        }
        let file = mdx.to_string_lossy().to_string();
        let data = fs::read(&mdx).unwrap();
        let index = parse_index(&data, &file, Ext::Mdx).unwrap();
        save_index(&index, &file);
        let loaded = load_index(&file).expect("缓存应可读");
        assert_eq!(loaded.keyword_list.len(), index.keyword_list.len());
        assert_eq!(loaded.keyword_list[0].key_text, index.keyword_list[0].key_text);
        assert_eq!(loaded.meta.encoding, index.meta.encoding);
        purge_file_cache(&file);
    }
}
