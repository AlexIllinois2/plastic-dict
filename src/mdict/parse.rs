//! MDX/MDD 二进制格式解析 (移植自 js-mdict 7.0.0 的解析逻辑, MIT 时期实现)
//!
//! 文件布局 (v1.2 / v2.0):
//!   [0:4]                   header 长度 (大端 u32)
//!   [4:4+len]               header 文本 (UTF-16LE, XML 属性对)
//!   [+4]                    header adler32 (跳过)
//!   key header              (v2: 5×u64, v1: 4×u32)
//!   key block info          (v2: 4字节压缩类型+4 adler+zlib; v1: 原始)
//!   key blocks              (每块: 4字节压缩类型+4 adler+数据)
//!   record header           (4×numWidth)
//!   record block info       (每块 pack/unpack size)
//!   record blocks           (每块: 4字节压缩类型+4 adler+数据)
//!
//! Encrypted 语义 (与 writemdict/mdict4j 规范一致):
//!   0x00 无加密; 0x01 关键字头 Salsa20 加密(需注册码, 不支持);
//!   0x02 关键字索引 fast-XOR 加密(ripemd128 密钥, 可解密)

use crate::util::unescape_entities;
use encoding_rs::{Encoding, BIG5, GB18030, UTF_16LE};
use flate2::read::ZlibDecoder;
use std::io::Read;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Ext {
    #[default]
    Mdx,
    Mdd,
}

impl Ext {
    pub fn as_str(&self) -> &'static str {
        match self {
            Ext::Mdx => "mdx",
            Ext::Mdd => "mdd",
        }
    }
    pub fn from_path(path: &str) -> Ext {
        let lower = path.to_lowercase();
        if lower.ends_with(".mdd") {
            Ext::Mdd
        } else {
            Ext::Mdx
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextEncoding {
    #[default]
    Utf8,
    Utf16Le,
    Gb18030,
    Big5,
}

impl TextEncoding {
    fn enc(&self) -> &'static Encoding {
        match self {
            TextEncoding::Utf8 => encoding_rs::UTF_8,
            TextEncoding::Utf16Le => UTF_16LE,
            TextEncoding::Gb18030 => GB18030,
            TextEncoding::Big5 => BIG5,
        }
    }
    /// header 里的 Encoding 字符串 → 内部编码
    fn from_header(value: &str) -> TextEncoding {
        match value.to_lowercase().as_str() {
            "gbk" | "gb2312" => TextEncoding::Gb18030,
            "big5" => TextEncoding::Big5,
            "utf16" | "utf-16" => TextEncoding::Utf16Le,
            _ => TextEncoding::Utf8,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct MdictMeta {
    pub fname: String,
    pub ext: Ext,
    pub version: f64,
    pub num_width: usize,
    pub encoding: TextEncoding,
    /// 0 无加密 / 1 关键字头 Salsa20(不支持) / 2 关键字索引 fast-XOR
    pub encrypt: i32,
}

#[derive(Debug, Clone, Default)]
pub struct KeyHeader {
    pub keyword_blocks_num: u64,
    pub keyword_num: u64,
    pub key_info_unpack_size: u64,
    pub key_info_packed_size: u64,
    pub keyword_block_packed_size: u64,
}

#[derive(Debug, Clone, Default)]
pub struct KeyInfoItem {
    pub first_key: String,
    pub last_key: String,
    pub key_block_pack_size: u64,
    pub key_block_pack_acc: u64,
    pub key_block_unpack_size: u64,
    pub key_block_unpack_acc: u64,
    pub key_block_entries_num: u64,
    pub key_block_entries_num_acc: u64,
}

#[derive(Debug, Clone, Default)]
pub struct KeyWordItem {
    pub record_start: u64,
    pub record_end: u64,
    pub key_text: String,
    pub key_block_idx: usize,
}

#[derive(Debug, Clone, Default)]
pub struct RecordHeader {
    pub record_blocks_num: u64,
    pub entries_num: u64,
    pub record_info_comp_size: u64,
    pub record_block_comp_size: u64,
}

#[derive(Debug, Clone, Default)]
pub struct RecordInfo {
    pub pack_size: u64,
    pub pack_acc: u64,
    pub unpack_size: u64,
    pub unpack_acc: u64,
}

/// 解析完成的索引 (可缓存到磁盘)
#[derive(Debug, Clone, Default)]
pub struct ParsedIndex {
    pub meta: MdictMeta,
    pub header: Vec<(String, String)>,
    pub key_header: KeyHeader,
    pub key_info_list: Vec<KeyInfoItem>,
    pub keyword_list: Vec<KeyWordItem>,
    pub record_header: RecordHeader,
    pub record_info_list: Vec<RecordInfo>,
    pub record_block_start_offset: u64,
}

// ---------------------------------------------------------------- 基础读取

/// 大端无符号整数 (宽度 1/2/4/8)
pub fn be_uint(data: &[u8], off: usize, width: usize) -> u64 {
    let mut n: u64 = 0;
    for i in 0..width {
        let b = data.get(off + i).copied().unwrap_or(0);
        n = (n << 8) | b as u64;
    }
    n
}

/// 小端无符号整数 (压缩块 comp_type 等字段用小端, 与 js-mdict 的字节序一致)
pub fn le_uint(data: &[u8], off: usize, width: usize) -> u64 {
    let mut n: u64 = 0;
    for i in (0..width).rev() {
        let b = data.get(off + i).copied().unwrap_or(0);
        n = (n << 8) | b as u64;
    }
    n
}

/// 用字典编码解码字节串
pub fn decode_str(bytes: &[u8], enc: TextEncoding) -> String {
    let (cow, _, _) = enc.enc().decode(bytes);
    cow.into_owned()
}

/// header 文本 (恒为 UTF-16LE)
fn decode_header_text(bytes: &[u8]) -> String {
    let (cow, _, _) = UTF_16LE.decode(bytes);
    cow.into_owned()
}

/// 解析 `key="value"` 属性对 (对应 TS parseHeader 的正则)
fn parse_header_pairs(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // 找 key 起始 (word char)
        if !(bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
            i += 1;
            continue;
        }
        let key_start = i;
        while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
            i += 1;
        }
        let key = text[key_start..i].to_string();
        // 跳过空白, 期待 '='
        let mut j = i;
        while j < bytes.len() && (bytes[j] as char).is_whitespace() {
            j += 1;
        }
        if j >= bytes.len() || bytes[j] != b'=' {
            continue;
        }
        j += 1;
        while j < bytes.len() && (bytes[j] as char).is_whitespace() {
            j += 1;
        }
        if j >= bytes.len() || bytes[j] != b'"' {
            continue;
        }
        j += 1;
        let val_start = j;
        while j < bytes.len() && bytes[j] != b'"' {
            j += 1;
        }
        let val = text[val_start..j.min(text.len())].to_string();
        out.push((key, unescape_entities(&val)));
        i = j + 1;
    }
    out
}

// ---------------------------------------------------------------- 解密

/// fast XOR 解密 (对应 TS fast_decrypt)
/// t = swap_nibbles(b) ^ prev ^ (i & 0xff) ^ key[i % key.len]; prev = b (解密前)
pub fn fast_decrypt(data: &[u8], key: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut previous: u8 = 0x36;
    for (i, &b) in data.iter().enumerate() {
        // swap nibbles: (b >> 4) | (b << 4), rotate_left(4) 等价且不会溢出
        let t = b.rotate_left(4) ^ previous ^ (i as u8 & 0xff) ^ key[i % key.len()];
        previous = b;
        out.push(t);
    }
    out
}

/// 块解密 (对应 TS mdxDecrypt):
/// key = ripemd128(block[4..8] ++ [0x95,0x36,0x00,0x00]); 返回 block[0..8] ++ decrypt(block[8..])
pub fn mdx_decrypt(block: &[u8]) -> Vec<u8> {
    use ripemd::{Digest, Ripemd128};
    let mut keyin = [0u8; 8];
    if block.len() >= 8 {
        keyin[..4].copy_from_slice(&block[4..8]);
    }
    keyin[4] ^= 0x95;
    keyin[5] ^= 0x36;
    // keyin[6] ^= 0x00; keyin[7] ^= 0x00;
    let mut hasher = Ripemd128::new();
    hasher.update(keyin);
    let key = hasher.finalize();
    let mut out = Vec::with_capacity(block.len());
    out.extend_from_slice(&block[..8.min(block.len())]);
    if block.len() > 8 {
        out.extend_from_slice(&fast_decrypt(&block[8..], &key));
    }
    out
}

// ---------------------------------------------------------------- 解压

/// 解压 key/record 块: [0:4] 压缩类型 (0 无 / 1 LZO / 2 zlib) + [4:8] adler32 + 数据
pub fn decompress_block(
    block: &[u8],
    _unpack_size: u64,
    encrypt: i32,
    is_record: bool,
) -> Result<Vec<u8>, String> {
    if block.len() < 8 {
        return Err("块长度不足 8 字节".into());
    }
    let comp_type = le_uint(block, 0, 4);
    if comp_type == 0 {
        // 注意: 与 js-mdict 一致, 无压缩块不做解密 (即使 encrypt==1)
        return Ok(block[8..].to_vec());
    }
    // 解密 (encrypt==1: 记录块; 与 js-mdict 语义一致)
    let decrypted: Vec<u8> = if encrypt == 1 && is_record {
        mdx_decrypt(block)
    } else {
        block[8..].to_vec()
    };
    match comp_type {
        1 => Err("该词典使用 LZO 压缩, 暂不支持 (可用其它工具转换为 zlib)".into()),
        2 => {
            let mut dec = ZlibDecoder::new(&decrypted[..]);
            let mut out = Vec::new();
            dec.read_to_end(&mut out).map_err(|e| format!("zlib 解压失败: {e}"))?;
            Ok(out)
        }
        other => Err(format!("未知压缩类型: {other}")),
    }
}

// ---------------------------------------------------------------- 索引解析

/// 完整解析 (无缓存路径)
pub fn parse_index(data: &[u8], fname: &str, ext: Ext) -> Result<ParsedIndex, String> {
    let mut idx = ParsedIndex {
        meta: MdictMeta {
            fname: fname.to_string(),
            ext,
            version: 0.0,
            num_width: 4,
            encoding: TextEncoding::Utf8,
            encrypt: 0,
        },
        ..Default::default()
    };

    // ---- STEP 1: header ----
    if data.len() < 8 {
        return Err("文件过小, 不是有效的 MDX/MDD".into());
    }
    let header_byte_size = be_uint(data, 0, 4) as usize;
    let header_end = 4 + header_byte_size + 4;
    if header_end > data.len() {
        return Err("header 长度越界".into());
    }
    let header_text = decode_header_text(&data[4..4 + header_byte_size]);
    let mut header = parse_header_pairs(&header_text);
    header.retain(|(k, _)| !k.is_empty());

    let get = |key: &str| -> Option<String> {
        header.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
    };

    // Encrypted: "" / "No" → 0; "Yes" → 1; 数字 → 该值
    idx.meta.encrypt = match get("Encrypted").as_deref() {
        None | Some("") | Some("No") => 0,
        Some("Yes") => 1,
        Some(v) => v.parse::<i32>().unwrap_or(0),
    };

    idx.meta.version = get("GeneratedByEngineVersion")
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.0);
    if idx.meta.version >= 2.0 {
        idx.meta.num_width = 8;
    } else {
        idx.meta.num_width = 4;
    }
    idx.meta.encoding = match get("Encoding").as_deref() {
        None | Some("") => TextEncoding::Utf8,
        Some(v) => TextEncoding::from_header(v),
    };
    // MDD 的关键词恒为 UTF-16LE
    if ext == Ext::Mdd {
        idx.meta.encoding = TextEncoding::Utf16Le;
    }

    let num_width = idx.meta.num_width;

    // ---- STEP 2: key header ----
    let key_header_size = if idx.meta.version >= 2.0 { 8 * 5 } else { 4 * 4 };
    let key_header_off = header_end;
    if key_header_off + key_header_size > data.len() {
        return Err("key header 越界".into());
    }
    // encrypt & 1 → 关键字头被 Salsa20 加密, 需要注册码, 无法读取
    if idx.meta.encrypt & 1 != 0 {
        return Err("词典关键字区已加密 (Salsa20), 需要注册码, 暂不支持".into());
    }
    let mut off = key_header_off;
    idx.key_header.keyword_blocks_num = be_uint(data, off, num_width);
    off += num_width;
    idx.key_header.keyword_num = be_uint(data, off, num_width);
    off += num_width;
    if idx.meta.version >= 2.0 {
        idx.key_header.key_info_unpack_size = be_uint(data, off, num_width);
        off += num_width;
    }
    idx.key_header.key_info_packed_size = be_uint(data, off, num_width);
    off += num_width;
    idx.key_header.keyword_block_packed_size = be_uint(data, off, num_width);
    let key_header_end = key_header_off + key_header_size + if idx.meta.version >= 2.0 { 4 } else { 0 };

    // ---- STEP 3: key block info ----
    let key_info_off = key_header_end;
    let key_info_packed_size = idx.key_header.key_info_packed_size as usize;
    if key_info_off + key_info_packed_size > data.len() {
        return Err("key block info 越界".into());
    }
    let mut key_info_buff = data[key_info_off..key_info_off + key_info_packed_size].to_vec();
    if idx.meta.encrypt == 2 {
        key_info_buff = mdx_decrypt(&key_info_buff);
    }
    // v2.0: key_index 恒为 [0:4] 压缩类型 (小端 uint32) + [4:8] adler32 + 数据 的封装
    // (fileformat.md: key_index_comp_len "including the comp_type and checksum parts")
    if idx.meta.version >= 2.0 && key_info_buff.len() >= 8 {
        match le_uint(&key_info_buff, 0, 4) {
            0 => key_info_buff = key_info_buff[8..].to_vec(),
            1 => return Err("key block info 使用 LZO 压缩, 暂不支持 (可用其它工具转换为 zlib)".into()),
            2 => {
                let mut dec = ZlibDecoder::new(&key_info_buff[8..]);
                let mut unpacked = Vec::new();
                dec.read_to_end(&mut unpacked)
                    .map_err(|e| format!("key block info zlib 解压失败: {e}"))?;
                key_info_buff = unpacked;
            }
            other => return Err(format!("key block info 未知压缩类型: {other}")),
        }
    }

    let mut key_info_list: Vec<KeyInfoItem> = Vec::new();
    let mut entries_acc: u64 = 0;
    let mut kb_pack_acc: u64 = 0;
    let mut kb_unpack_acc: u64 = 0;
    let mut io: usize = 0;
    let is_utf16 = idx.meta.encoding == TextEncoding::Utf16Le;
    for _ in 0..idx.key_header.keyword_blocks_num {
        let block_word_count = be_uint(&key_info_buff, io, num_width);
        io += num_width;
        let mut first_word_size = be_uint(&key_info_buff, io, num_width / 4);
        io += num_width / 4;
        if idx.meta.version >= 2.0 {
            first_word_size = if is_utf16 {
                (first_word_size + 1) * 2
            } else {
                first_word_size + 1
            };
        } else if is_utf16 {
            first_word_size *= 2;
        }
        let first_word = decode_str(
            &key_info_buff[io..(io + first_word_size as usize).min(key_info_buff.len())],
            idx.meta.encoding,
        );
        io += first_word_size as usize;
        let mut last_word_size = be_uint(&key_info_buff, io, num_width / 4);
        io += num_width / 4;
        if idx.meta.version >= 2.0 {
            last_word_size = if is_utf16 {
                (last_word_size + 1) * 2
            } else {
                last_word_size + 1
            };
        } else if is_utf16 {
            last_word_size *= 2;
        }
        let last_word = decode_str(
            &key_info_buff[io..(io + last_word_size as usize).min(key_info_buff.len())],
            idx.meta.encoding,
        );
        io += last_word_size as usize;
        let pack_size = be_uint(&key_info_buff, io, num_width);
        io += num_width;
        let unpack_size = be_uint(&key_info_buff, io, num_width);
        io += num_width;
        key_info_list.push(KeyInfoItem {
            first_key: first_word,
            last_key: last_word,
            key_block_pack_size: pack_size,
            key_block_pack_acc: kb_pack_acc,
            key_block_unpack_size: unpack_size,
            key_block_unpack_acc: kb_unpack_acc,
            key_block_entries_num: block_word_count,
            key_block_entries_num_acc: entries_acc,
        });
        entries_acc += block_word_count;
        kb_pack_acc += pack_size;
        kb_unpack_acc += unpack_size;
    }
    idx.key_info_list = key_info_list;

    let key_block_start = key_info_off + key_info_packed_size;
    idx.record_block_start_offset =
        key_block_start as u64 + idx.key_header.keyword_block_packed_size;

    // ---- STEP 4: key blocks → 关键词列表 ----
    let mut keyword_list: Vec<KeyWordItem> = Vec::new();
    let mut kb_off = key_block_start;
    for (block_idx, ki) in idx.key_info_list.iter().enumerate() {
        let pack_size = ki.key_block_pack_size as usize;
        if kb_off + pack_size > data.len() {
            return Err("key block 越界".into());
        }
        let packed = &data[kb_off..kb_off + pack_size];
        let key_block = decompress_block(packed, ki.key_block_unpack_size, idx.meta.encrypt, false)?;
        let items = split_key_block(&key_block, num_width, idx.meta.encoding, ext, block_idx);
        if let Some(last) = keyword_list.last_mut() {
            if last.record_end == u64::MAX {
                last.record_end = items.first().map(|k| k.record_start).unwrap_or(u64::MAX);
            }
        }
        keyword_list.extend(items);
        kb_off += pack_size;
    }
    if let Some(last) = keyword_list.last_mut() {
        if last.record_end == u64::MAX {
            last.record_end = u64::MAX; // 稍后由 record info 覆盖
        }
    }
    idx.keyword_list = keyword_list;

    // ---- STEP 5: record header ----
    let rec_header_off = key_block_start + idx.key_header.keyword_block_packed_size as usize;
    let rec_header_len = if idx.meta.version >= 2.0 { 4 * 8 } else { 4 * 4 };
    if rec_header_off + rec_header_len > data.len() {
        return Err("record header 越界".into());
    }
    let mut off = rec_header_off as usize;
    idx.record_header.record_blocks_num = be_uint(data, off, num_width);
    off += num_width;
    idx.record_header.entries_num = be_uint(data, off, num_width);
    off += num_width;
    idx.record_header.record_info_comp_size = be_uint(data, off, num_width);
    off += num_width;
    idx.record_header.record_block_comp_size = be_uint(data, off, num_width);
    let rec_info_off = rec_header_off + rec_header_len;

    // ---- STEP 6: record block info ----
    let rec_info_size = idx.record_header.record_info_comp_size as usize;
    if rec_info_off as usize + rec_info_size > data.len() {
        return Err("record block info 越界".into());
    }
    let rec_info_buff = &data[rec_info_off as usize..rec_info_off as usize + rec_info_size];
    let mut record_info_list: Vec<RecordInfo> = Vec::new();
    let mut comp_acc: u64 = 0;
    let mut uncomp_acc: u64 = 0;
    let mut off = 0usize;
    for _ in 0..idx.record_header.record_blocks_num {
        let pack_size = be_uint(rec_info_buff, off, num_width);
        off += num_width;
        let unpack_size = be_uint(rec_info_buff, off, num_width);
        off += num_width;
        record_info_list.push(RecordInfo {
            pack_size,
            pack_acc: comp_acc,
            unpack_size,
            unpack_acc: uncomp_acc,
        });
        comp_acc += pack_size;
        uncomp_acc += unpack_size;
    }
    idx.record_info_list = record_info_list;

    // 最后一个关键词 (解析顺序) 的 end = 解压后总量
    if !idx.record_info_list.is_empty() {
        if let Some(last) = idx.keyword_list.last_mut() {
            let last_ri = idx.record_info_list.last().unwrap();
            last.record_end = last_ri.unpack_acc + last_ri.unpack_size;
        }
    }
    idx.record_block_start_offset = rec_info_off as u64 + rec_info_size as u64;

    // ---- 排序 (与 js-mdict 一致, 解析后重排; 比较器: 大小写不敏感优先, 原始串次级) ----
    idx.keyword_list.sort_by(|a, b| key_cmp(&a.key_text, &b.key_text));

    idx.header = header;
    Ok(idx)
}

/// 关键词比较器: 先按小写比较, 再按原始串 (近似 localeCompare, 排序与查找一致即可)
pub fn key_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let la = a.to_lowercase();
    let lb = b.to_lowercase();
    la.cmp(&lb).then_with(|| a.cmp(b))
}

/// 拆分 key block 为关键词列表 (对应 TS splitKeyBlock)
fn split_key_block(
    key_block: &[u8],
    num_width: usize,
    enc: TextEncoding,
    ext: Ext,
    block_idx: usize,
) -> Vec<KeyWordItem> {
    let width: usize = if enc == TextEncoding::Utf16Le || ext == Ext::Mdd { 2 } else { 1 };
    let mut list: Vec<KeyWordItem> = Vec::new();
    let mut key_start: usize = 0;
    while key_start < key_block.len() {
        let meaning_offset = be_uint(key_block, key_start, num_width);
        let mut i = key_start + num_width;
        let mut key_end: Option<usize> = None;
        while i < key_block.len() {
            let is_term = if width == 1 {
                key_block[i] == 0
            } else {
                key_block[i] == 0 && key_block.get(i + 1).copied() == Some(0)
            };
            if is_term {
                key_end = Some(i);
                break;
            }
            i += width;
        }
        let Some(key_end) = key_end else { break };
        let key_text = decode_str(&key_block[key_start + num_width..key_end], enc);
        if let Some(last) = list.last_mut() {
            last.record_end = meaning_offset;
        }
        list.push(KeyWordItem {
            record_start: meaning_offset,
            record_end: u64::MAX,
            key_text,
            key_block_idx: block_idx,
        });
        key_start = key_end + width;
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn be_uint_big_endian() {
        assert_eq!(be_uint(&[0, 0, 0, 2], 0, 4), 2);
        assert_eq!(be_uint(&[0x01, 0x02], 0, 2), 0x0102);
        assert_eq!(be_uint(&[0, 0, 0, 0, 0, 0, 0, 42], 0, 8), 42);
    }

    #[test]
    fn header_pairs() {
        let pairs = parse_header_pairs("<Dictionary Name=\"x\" Encoding=\"UTF-8\" Title=\"a &quot;b&quot;\"/>");
        assert_eq!(pairs.len(), 3);
        assert_eq!(pairs[1], ("Encoding".to_string(), "UTF-8".to_string()));
        assert_eq!(pairs[2].1, "a \"b\"");
    }

    #[test]
    fn fast_decrypt_roundtrip() {
        // 加密是解密的逆运算: c[i] = swap(p[i] ^ c[i-1] ^ i ^ key)
        let key = [1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
        let plain: Vec<u8> = (0..64u8).collect();
        let mut cipher = Vec::with_capacity(plain.len());
        let mut prev = 0x36u8;
        for (i, &p) in plain.iter().enumerate() {
            let t = p ^ prev ^ (i as u8 & 0xff) ^ key[i % key.len()];
            let c = t.rotate_left(4);
            cipher.push(c);
            prev = c;
        }
        let dec = fast_decrypt(&cipher, &key);
        assert_eq!(dec, plain);
    }

    #[test]
    fn key_cmp_case_insensitive_then_raw() {
        use std::cmp::Ordering;
        assert_eq!(key_cmp("apple", "Apple"), Ordering::Greater);
        assert_eq!(key_cmp("apple", "apple"), Ordering::Equal);
        assert_eq!(key_cmp("app", "apple"), Ordering::Less);
        assert_eq!(key_cmp("Banana", "apple"), Ordering::Greater);
    }
}
