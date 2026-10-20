//! MDX/MDD 词典访问门面 (移植自 TS 版 src/mdict.ts 的高级逻辑)
//!
//! 包含: 词典文件发现/加载池、多分卷 MDD、查词、发音提取与播放、资源查找。

pub mod cache;
pub mod parse;

use parse::{
    decode_str, decompress_block, Ext, KeyWordItem, ParsedIndex, TextEncoding,
};
use regex::Regex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

// ---------------------------------------------------------------- 词典文件

pub struct MdictFile {
    pub path: String,
    pub data: Vec<u8>,
    pub index: ParsedIndex,
}

impl MdictFile {
    /// 打开 (优先读磁盘索引缓存)
    pub fn open(path: &str) -> Result<Self, String> {
        if let Some(index) = cache::load_index(path) {
            let data = std::fs::read(path).map_err(|e| format!("读取失败: {e}"))?;
            return Ok(Self {
                path: path.to_string(),
                data,
                index,
            });
        }
        Self::open_uncached(path)
    }

    /// 打开 (强制重新解析, 成功后写缓存)
    pub fn open_uncached(path: &str) -> Result<Self, String> {
        let data = std::fs::read(path).map_err(|e| format!("读取失败 {path}: {e}"))?;
        let ext = Ext::from_path(path);
        let index = parse::parse_index(&data, path, ext)?;
        cache::save_index(&index, path);
        Ok(Self {
            path: path.to_string(),
            data,
            index,
        })
    }

    pub fn ext(&self) -> Ext {
        self.index.meta.ext
    }

    pub fn encoding(&self) -> TextEncoding {
        self.index.meta.encoding
    }

    /// 二进制查找关键词: 精确匹配优先, 其次大小写不敏感
    pub fn lookup_key(&self, word: &str) -> Option<&KeyWordItem> {
        let list = &self.index.keyword_list;
        if list.is_empty() {
            return None;
        }
        let lw = word.to_lowercase();
        let start = list.partition_point(|k| k.key_text.to_lowercase() < lw);
        let end = list.partition_point(|k| k.key_text.to_lowercase() <= lw);
        if start >= end {
            return None;
        }
        // 先找精确匹配
        for item in &list[start..end] {
            if item.key_text == word {
                return Some(item);
            }
        }
        // 大小写不敏感兜底 (对应 TS 的 lookup(word.toLowerCase()))
        Some(&list[start])
    }

    /// MDX: 查词返回解码后的释义文本
    pub fn lookup_text(&self, word: &str) -> Option<String> {
        let item = self.lookup_key(word)?;
        let bytes = self.lookup_record(item).ok()?;
        Some(decode_str(&bytes, self.encoding()))
    }

    /// MDD: 按 key 找资源, 返回原始字节
    pub fn locate_raw(&self, key: &str) -> Option<Vec<u8>> {
        let normalized = normalize_mdd_key(key);
        let item = self
            .lookup_key(&normalized)
            .or_else(|| self.lookup_key(key))?;
        self.lookup_record(item).ok()
    }

    /// 读取关键词对应的记录块切片
    fn lookup_record(&self, item: &KeyWordItem) -> Result<Vec<u8>, String> {
        let idx = self.reduce_record_block_info(item.record_start)?;
        let ri = &self.index.record_info_list[idx];
        let start = self.index.record_block_start_offset + ri.pack_acc;
        let end = start + ri.pack_size;
        if end as usize > self.data.len() {
            return Err("record block 越界".into());
        }
        let block = &self.data[start as usize..end as usize];
        let unpacked = decompress_block(block, ri.unpack_size, self.index.meta.encrypt, true)?;
        let s = (item.record_start - ri.unpack_acc) as usize;
        let e = (item.record_end - ri.unpack_acc) as usize;
        if e > unpacked.len() || s > e {
            return Err("record 切片越界".into());
        }
        Ok(unpacked[s..e].to_vec())
    }

    fn reduce_record_block_info(&self, record_start: u64) -> Result<usize, String> {
        let list = &self.index.record_info_list;
        if list.is_empty() {
            return Err("record info 为空".into());
        }
        let mut left: i64 = 0;
        let mut right: i64 = list.len() as i64 - 1;
        while left <= right {
            let mid = left + (right - left) / 2;
            if record_start >= list[mid as usize].unpack_acc {
                left = mid + 1;
            } else {
                right = mid - 1;
            }
        }
        Ok((left - 1).max(0) as usize)
    }

    pub fn all_keys(&self) -> Vec<String> {
        self.index.keyword_list.iter().map(|k| k.key_text.clone()).collect()
    }
}

/// MDD key 规范化: `/` → `\`, 补前导 `\` (对应 TS MDD.locate)
pub fn normalize_mdd_key(key: &str) -> String {
    let mut k = key.replace('/', "\\");
    if !k.starts_with('\\') {
        k = format!("\\{k}");
    }
    k
}

// ---------------------------------------------------------------- 词典池

static POOL: OnceLock<Mutex<HashMap<String, Arc<MdictFile>>>> = OnceLock::new();

fn pool() -> &'static Mutex<HashMap<String, Arc<MdictFile>>> {
    POOL.get_or_init(|| Mutex::new(HashMap::new()))
}

fn get_builder(path: &str) -> Option<Arc<MdictFile>> {
    {
        let guard = pool().lock().unwrap();
        if let Some(b) = guard.get(path) {
            return Some(Arc::clone(b));
        }
    }
    let t0 = Instant::now();
    let loaded = match MdictFile::open(path) {
        Ok(f) => f,
        Err(e) => {
            // 与 TS 一致: 失败时清掉可能损坏的缓存重试一次
            cache::purge_file_cache(path);
            match MdictFile::open_uncached(path) {
                Ok(f) => f,
                Err(e2) => {
                    crate::paths::log_err(format!(
                        "  [构建] 索引失败: {} ({e} / {e2})",
                        Path::new(path).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
                    ));
                    return None;
                }
            }
        }
    };
    let arc = Arc::new(loaded);
    pool().lock().unwrap().insert(path.to_string(), Arc::clone(&arc));
    if t0.elapsed().as_millis() > 300 {
        crate::paths::log_err(format!(
            "  [构建] 索引缓存(一次性): {} ({}ms)",
            Path::new(path).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
            t0.elapsed().as_millis()
        ));
    }
    Some(arc)
}

/// 递归列出目录下所有 .mdx (排序稳定)
pub fn list_mdx_files() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let mut items: Vec<_> = entries.flatten().collect();
        items.sort_by_key(|e| e.file_name());
        for e in items {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.is_file()
                && p.extension().map(|x| x.eq_ignore_ascii_case("mdx")).unwrap_or(false)
            {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    walk(&crate::paths::dict_dir(), &mut out);
    out.sort();
    out
}

/// 递归列出目录下所有 .css
pub fn list_css_files(dir: &Path) -> Vec<PathBuf> {
    fn walk(d: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(d) else { return };
        let mut items: Vec<_> = entries.flatten().collect();
        items.sort_by_key(|e| e.file_name());
        for e in items {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.is_file()
                && p.extension().map(|x| x.eq_ignore_ascii_case("css")).unwrap_or(false)
            {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, &mut out);
    out.sort();
    out
}

// ---------------------------------------------------------------- 多分卷 MDD

/// base.mdd + base.1.mdd + base.2.mdd ...
struct MultiMdd {
    vols: Vec<String>,
    mounted: Option<(String, Arc<MdictFile>)>,
}

impl MultiMdd {
    fn new(mdx_file: &str) -> Self {
        // stem = 去掉 .mdx/.MDX 后缀 (对应 TS: replace(/\.(mdx|MDX)$/, ""))
        let stem = mdx_file
            .strip_suffix(".mdx")
            .or_else(|| mdx_file.strip_suffix(".MDX"))
            .unwrap_or(mdx_file)
            .to_string();
        let dir = Path::new(&stem).parent().unwrap_or(Path::new("."));
        let base = Path::new(&stem)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let mut names = vec![format!("{base}.mdd")];
        let re = Regex::new(&format!(
            "^{}\\.\\d+\\.mdd$",
            regex::escape(&base)
        ))
        .unwrap();
        if let Ok(entries) = std::fs::read_dir(dir) {
            let mut found: Vec<String> = entries
                .flatten()
                .map(|e| e.file_name().to_string_lossy().to_string())
                .filter(|f| re.is_match(f))
                .collect();
            found.sort();
            names.extend(found);
        }
        let mut vols = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for n in names {
            let p = dir.join(&n);
            if let Ok(meta) = std::fs::metadata(&p) {
                if !meta.is_file() {
                    continue;
                }
                let rp = std::fs::canonicalize(&p).unwrap_or(p.clone());
                if seen.insert(rp) {
                    vols.push(p.to_string_lossy().to_string());
                }
            }
        }
        Self { vols, mounted: None }
    }

    fn mount(&mut self, vol: &str) -> Option<Arc<MdictFile>> {
        if self.mounted.as_ref().map(|(v, _)| v == vol).unwrap_or(false) {
            return self.mounted.as_ref().map(|(_, f)| Arc::clone(f));
        }
        let f = get_builder(vol)?;
        self.mounted = Some((vol.to_string(), Arc::clone(&f)));
        Some(f)
    }

    fn lookup(&mut self, ref_: &str) -> Option<Vec<u8>> {
        let candidates = [
            normalize_mdd_key(ref_),
            ref_.to_string(),
            format!("/{}", ref_.replace('\\', "/").trim_start_matches('/')),
            format!("\\{}", ref_.replace('/', "\\").trim_start_matches('\\')),
        ];
        for vol in self.vols.clone() {
            let Some(inst) = self.mount(&vol) else { continue };
            for cand in &candidates {
                if let Some(data) = inst.locate_raw(cand) {
                    if !data.is_empty() {
                        return Some(data);
                    }
                }
            }
        }
        // 后缀兜底: 所有 key 里 basename 匹配的
        let suffix = format!("/{}", ref_.replace('\\', "/").trim_start_matches('/'));
        for vol in self.vols.clone() {
            let Some(inst) = self.mount(&vol) else { continue };
            let keys = inst.all_keys();
            for k in keys {
                if k.replace('\\', "/").ends_with(&suffix) {
                    if let Some(data) = inst.locate_raw(&k) {
                        if !data.is_empty() {
                            return Some(data);
                        }
                    }
                }
            }
        }
        None
    }

    fn all_keys(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        for vol in self.vols.clone() {
            if let Some(inst) = self.mount(&vol) {
                out.extend(inst.all_keys());
            }
        }
        out
    }
}

static MDD_POOL: OnceLock<Mutex<HashMap<String, MultiMdd>>> = OnceLock::new();

fn mdd_pool() -> &'static Mutex<HashMap<String, MultiMdd>> {
    MDD_POOL.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) fn multi_mdd_lookup(mdx: &str, ref_: &str) -> Option<Vec<u8>> {
    let mut pool = mdd_pool().lock().unwrap();
    let m = pool.entry(mdx.to_string()).or_insert_with(|| MultiMdd::new(mdx));
    m.lookup(ref_)
}

fn multi_mdd_all_keys(mdx: &str) -> Vec<String> {
    let mut pool = mdd_pool().lock().unwrap();
    let m = pool.entry(mdx.to_string()).or_insert_with(|| MultiMdd::new(mdx));
    m.all_keys()
}

// ---------------------------------------------------------------- 查词结果

#[derive(Debug, Default, Clone)]
pub struct LookupResult {
    pub found: bool,
    pub word: String,
    /// CLI 纯文本
    pub text: Option<String>,
    /// 原始词条 HTML
    pub html: Option<String>,
    /// 词典自带 CSS (拼接)
    pub css: String,
    pub dict: Option<String>,
    pub mdx: Option<String>,
    pub audio_refs: Vec<String>,
    pub headword_chn: Option<String>,
}

/// 编译音频引用正则 (与 TS AUDIO_PATTERNS 一致)
fn audio_patterns() -> &'static [Regex; 4] {
    use std::sync::OnceLock;
    static PATS: OnceLock<[Regex; 4]> = OnceLock::new();
    PATS.get_or_init(|| {
        [
            Regex::new(r#"sound://([^"'\s>\\]+)"#).unwrap(),
            Regex::new(r#"data-src-mp3=["']([^"']+)["']"#).unwrap(),
            Regex::new(r#"href=["']([^"']*?\.mp3)"#).unwrap(),
            // 牛津 9 用的是 data-sound (GoldenDict 约定的内联发音属性)
            Regex::new(r#"data-sound=["']([^"']+)["']"#).unwrap(),
        ]
    })
}

/// 从词条 HTML 中提取音频引用 (对应 TS parseAudioRefs)
pub fn parse_audio_refs(html: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut refs = Vec::new();
    for pat in audio_patterns().iter() {
        for cap in pat.captures_iter(html) {
            let mut r = cap[1].trim().to_string();
            if let Some(rest) = r.strip_prefix("sound://") {
                r = rest.to_string();
            }
            if r.is_empty() || seen.contains(&r) {
                continue;
            }
            let base = Path::new(&r.replace('\\', "/"))
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            if base.starts_with('_') {
                continue;
            }
            if base.contains("__")
                && ["gbs", "uss", "brs", "ams"].iter().any(|t| base.contains(t))
            {
                continue;
            }
            seen.insert(r.clone());
            refs.push(r);
        }
    }
    refs
}

// 主查词入口 (对应 TS lookup)
pub fn lookup(word: &str, bold: bool) -> LookupResult {
    let mut r = LookupResult {
        word: word.to_string(),
        ..Default::default()
    };
    let mdx_files = list_mdx_files();
    if mdx_files.is_empty() {
        return r;
    }
    for mdx in mdx_files {
        let mdx_path = mdx.to_string_lossy().to_string();
        let Some(builder) = get_builder(&mdx_path) else { continue };
        let mut def: Option<String> = builder.lookup_text(word);
        if def.is_none() {
            def = builder.lookup_text(&word.to_lowercase());
        }
        let Some(def) = def else { continue };
        let html = def
            .trim_end_matches('\0')
            .trim_end()
            .to_string();
        if html.is_empty() {
            continue;
        }
        r.found = true;
        r.dict = Some(
            Path::new(&mdx_path)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default(),
        );
        r.mdx = Some(mdx_path.clone());
        let stripped = crate::textrender::strip_dup(&html);
        r.text = Some(crate::textrender::render(&stripped, 76, bold));
        let mut css_parts = Vec::new();
        for cp in list_css_files(mdx.parent().unwrap_or(Path::new("."))) {
            if let Ok(s) = std::fs::read_to_string(&cp) {
                css_parts.push(s);
            }
        }
        r.css = css_parts.join("\n\n");
        r.html = Some(stripped.clone());
        let mut hw = crate::textrender::headword_chn(&stripped);
        if !hw.is_empty() {
            hw = strip_parens(&hw);
            hw = hw
                .trim_matches(|c: char| c == ' ' || c == '；' || c == ';' || c == '，' || c == ',')
                .to_string();
            if hw.chars().count() > 30 {
                hw = hw.chars().take(30).collect::<String>() + "…";
            }
            if let Some(t) = r.text.as_mut() {
                let mut lines: Vec<String> = t.split('\n').map(|s| s.to_string()).collect();
                if let Some(first) = lines.first_mut() {
                    let trimmed = first.trim_end().to_string();
                    *first = format!("{trimmed} 【{hw}】");
                }
                *t = lines.join("\n");
            }
        }
        r.audio_refs = parse_audio_refs(&html);
        break;
    }
    r
}

fn strip_parens(s: &str) -> String {
    // 对应 TS: hw.replace(/（[^）]*）|\([^)]*\)/g, "")
    let mut out = String::new();
    let mut depth_cn = 0usize;
    let mut depth_en = 0usize;
    for c in s.chars() {
        match c {
            '（' => depth_cn += 1,
            '）' => {
                if depth_cn > 0 {
                    depth_cn -= 1;
                }
            }
            '(' => depth_en += 1,
            ')' => {
                if depth_en > 0 {
                    depth_en -= 1;
                }
            }
            _ => {
                if depth_cn == 0 && depth_en == 0 {
                    out.push(c);
                }
            }
        }
    }
    out
}

/// 提取音频到 /tmp 并返回路径 (对应 TS extractAudio)
pub fn extract_audio(mdx: &str, ref_: &str) -> Option<PathBuf> {
    let r = ref_.replace('\\', "/");
    let r = r.trim().trim_start_matches('/').to_string();
    let mut data = multi_mdd_lookup(mdx, &r);
    if data.is_none() {
        let base = Path::new(&r)
            .file_name()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        for k in multi_mdd_all_keys(mdx) {
            let kb = Path::new(&k.replace('\\', "/"))
                .file_name()
                .map(|s| s.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if kb == base {
                data = multi_mdd_lookup(mdx, &k);
                if data.is_some() {
                    break;
                }
            }
        }
    }
    let data = data?;
    let audio_dir = crate::paths::audio_dir();
    std::fs::create_dir_all(&audio_dir).ok()?;
    let name = Path::new(&r)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "audio.bin".into());
    let out = audio_dir.join(&name);
    if out.exists() && std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0) == data.len() as u64 {
        return Some(out);
    }
    // 只保留最新一个文件 (对应 TS 行为)
    if let Ok(entries) = std::fs::read_dir(&audio_dir) {
        for e in entries.flatten() {
            if e.file_name().to_string_lossy() != name {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    std::fs::write(&out, &data).ok()?;
    Some(out)
}

/// 按发音变体找可播放音频 (对应 TS getPlayable)
pub fn get_playable(word: &str, variant: &str) -> Option<PathBuf> {
    for mdx in list_mdx_files() {
        let mdx_path = mdx.to_string_lossy().to_string();
        let Some(b) = get_builder(&mdx_path) else { continue };
        let mut def = b.lookup_text(word);
        if def.is_none() {
            def = b.lookup_text(&word.to_lowercase());
        }
        let Some(def) = def else { continue };
        for r in parse_audio_refs(&def) {
            let base = Path::new(&r.replace('\\', "/"))
                .file_name()
                .map(|s| s.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if base.contains(&format!("_{variant}_")) {
                return extract_audio(&mdx_path, &r);
            }
        }
    }
    None
}

/// 跨所有词典的 MDD 卷找资源 (对应 TS dicresFind)
pub fn dicres_find(ref_: &str) -> Option<Vec<u8>> {
    for mdx in list_mdx_files() {
        let mdx_path = mdx.to_string_lossy().to_string();
        if let Some(data) = multi_mdd_lookup(&mdx_path, ref_) {
            if !data.is_empty() {
                return Some(data);
            }
        }
    }
    let base = Path::new(&ref_.replace('\\', "/"))
        .file_name()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    for mdx in list_mdx_files() {
        let mdx_path = mdx.to_string_lossy().to_string();
        for k in multi_mdd_all_keys(&mdx_path) {
            let kb = Path::new(&k.replace('\\', "/"))
                .file_name()
                .map(|s| s.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if kb == base {
                if let Some(data) = multi_mdd_lookup(&mdx_path, &k) {
                    if !data.is_empty() {
                        return Some(data);
                    }
                }
            }
        }
    }
    None
}

/// 供 GUI 准备发音按钮: 从词条 HTML 提取 (label, phon, ref) 三元组
pub fn extract_prons(html: &str) -> Vec<(String, String, Option<String>)> {
    let refs = parse_audio_refs(html);
    let gb = refs.iter().find(|r| basename_lc(r).contains("_gb_")).cloned();
    let us = refs.iter().find(|r| basename_lc(r).contains("_us_")).cloned();
    let mut out = Vec::new();
    if gb.is_some() || us.is_some() {
        if gb.is_some() {
            out.push(("BrE".to_string(), String::new(), gb));
        }
        if us.is_some() {
            out.push(("NAmE".to_string(), String::new(), us));
        }
    } else if let Some(first) = refs.first() {
        out.push(("🔊".to_string(), String::new(), Some(first.clone())));
    }
    out
}

fn basename_lc(p: &str) -> String {
    Path::new(&p.replace('\\', "/"))
        .file_name()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

/// 进程退出时释放 (Rust 版无需显式关闭文件句柄, 保留接口)
pub fn close_all_builders() {
    pool().lock().unwrap().clear();
    mdd_pool().lock().unwrap().clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_mdd_key_variants() {
        assert_eq!(normalize_mdd_key("Logo.jpg"), "\\Logo.jpg");
        assert_eq!(normalize_mdd_key("/Logo.jpg"), "\\Logo.jpg");
        assert_eq!(normalize_mdd_key("media/audio/x.spx"), "\\media\\audio\\x.spx");
        assert_eq!(normalize_mdd_key("\\x\\y.png"), "\\x\\y.png");
    }

    #[test]
    fn strip_parens_removes_groups() {
        assert_eq!(strip_parens("苹果（fruit）x"), "苹果x");
        assert_eq!(strip_parens("a (b) c"), "a  c");
        assert_eq!(strip_parens("plain"), "plain");
    }

    /// 测试夹具 (tools/make_fixture.py 生成) 的端到端回归:
    /// MDX v2.0 多块解析 → 查词 → MDD 资源定位。缺夹具时提示重新生成。
    #[test]
    fn fixture_mdx_mdd_roundtrip() {
        use crate::mdict::parse::{parse_index, Ext};
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata");
        let mdx_path = dir.join("Oxford9.mdx");
        let mdd_path = dir.join("Oxford9.mdd");
        assert!(
            mdx_path.exists() && mdd_path.exists(),
            "缺少测试夹具, 请先运行: python tools/make_fixture.py"
        );

        // ---- MDX 解析 (2 key blocks / 2 record blocks) ----
        let data = std::fs::read(&mdx_path).unwrap();
        let idx = parse_index(&data, "Oxford9.mdx", Ext::Mdx).expect("MDX 解析失败");
        let keys: Vec<&str> = idx.keyword_list.iter().map(|k| k.key_text.as_str()).collect();
        assert_eq!(keys, vec!["apple", "banana", "run"]);
        assert_eq!(idx.key_header.keyword_num, 3);
        assert_eq!(idx.key_header.keyword_blocks_num, 2);
        assert_eq!(idx.record_header.record_blocks_num, 2);
        assert_eq!(idx.record_info_list.len(), 2);
        let get = |w: &str| idx.keyword_list.iter().find(|k| k.key_text == w).unwrap();
        let (apple, banana, run) = (get("apple"), get("banana"), get("run"));
        assert_eq!(apple.record_start, 0);
        assert_eq!(apple.record_end, banana.record_start);
        assert_eq!(banana.record_end, run.record_start);
        let total = idx
            .record_info_list
            .last()
            .map(|ri| ri.unpack_acc + ri.unpack_size)
            .unwrap();
        assert_eq!(run.record_end, total, "最后一个词的 record_end 应为解压总量");

        // ---- 查词 (绕过磁盘缓存, 直接构造) ----
        let f = MdictFile {
            path: mdx_path.to_string_lossy().to_string(),
            data,
            index: idx,
        };
        let html = f.lookup_text("apple").expect("apple 查不到");
        assert!(html.contains("<h>apple</h>"), "html 缺 headword: {html}");
        assert!(html.contains("苹果"), "html 缺中文释义: {html}");
        assert!(html.contains("sound_us_apple.spx"), "html 缺音频引用: {html}");
        assert!(f.lookup_text("Apple").is_some(), "大小写不敏感查词失败");
        assert!(f.lookup_text("banana").is_some());
        assert!(f.lookup_text("run").is_some(), "跨 record block 查词失败");
        assert!(f.lookup_text("zzznotaword").is_none());

        // ---- MDD 资源定位 ----
        let mdata = std::fs::read(&mdd_path).unwrap();
        let midx = parse_index(&mdata, "Oxford9.mdd", Ext::Mdd).expect("MDD 解析失败");
        let mkeys: Vec<&str> = midx.keyword_list.iter().map(|k| k.key_text.as_str()).collect();
        assert_eq!(mkeys, vec!["\\apple.png", "\\sound_us_apple.spx"]);
        let mf = MdictFile {
            path: mdd_path.to_string_lossy().to_string(),
            data: mdata,
            index: midx,
        };
        let png = mf.locate_raw("\\apple.png").expect("apple.png 定位失败");
        assert_eq!(&png[..4], &[0x89, b'P', b'N', b'G']);
        assert_eq!(png.len(), 69);
        assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");
        let spx = mf.locate_raw("\\sound_us_apple.spx").expect("spx 定位失败");
        assert_eq!(spx, b"fake-spx-audio-bytes");
        // 归一化候选 (对应 TS 的 candidates())
        assert!(mf.locate_raw("/apple.png").is_some(), "斜杠归一化失败");
    }
}
