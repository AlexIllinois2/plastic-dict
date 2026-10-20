//! GUI 词条 HTML 预处理 (移植自 TS 版 src/gui.ts 的 prepare/cssLocalize)
//!
//! 与原版的差异: 原版把资源改写成本地 HTTP 服务 URL (/res/...),
//! Rust 版直接内联为 data: URI (Blitz 的 data-uri net provider 可直接加载)。

use crate::dom::*;
use crate::mdict;
use regex::Regex;
use std::path::Path;

const RES_SKIP: &str = "^(https?:|data:|file:|about:|//)";
// 注意: 牛津词条修正 CSS 在 gui/style.rs (OXFORD9_CSS), 由调用方与词典自带
// CSS 拼接后传入 prepare_entry; 此处不再重复定义。

const MIME_BY_EXT: &[(&str, &str)] = &[
    (".png", "image/png"), (".jpg", "image/jpeg"), (".jpeg", "image/jpeg"),
    (".gif", "image/gif"), (".svg", "image/svg+xml"), (".webp", "image/webp"),
    (".ico", "image/x-icon"), (".bmp", "image/bmp"),
    (".css", "text/css"), (".js", "application/javascript"),
    (".mp3", "audio/mpeg"), (".wav", "audio/wav"), (".ogg", "audio/ogg"),
    (".oga", "audio/ogg"), (".aac", "audio/aac"), (".m4a", "audio/mp4"),
    (".spx", "audio/speex"), (".opus", "audio/opus"), (".wma", "audio/x-ms-wma"),
    (".ttf", "font/ttf"), (".otf", "font/otf"), (".woff", "font/woff"),
    (".woff2", "font/woff2"), (".html", "text/html"), (".htm", "text/html"),
];

fn mime_for(ref_: &str) -> &'static str {
    let ext = Path::new(&ref_.replace('\\', "/"))
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))
        .unwrap_or_default();
    MIME_BY_EXT
        .iter()
        .find(|(k, _)| *k == ext)
        .map(|(_, v)| *v)
        .unwrap_or("application/octet-stream")
}

/// 资源引用 → data: URI (找不到原引用则返回 None)
fn data_uri_for(ref_: &str) -> Option<String> {
    let data = mdict::dicres_find(ref_)?;
    if data.is_empty() {
        return None;
    }
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &data);
    Some(format!("data:{};base64,{}", mime_for(ref_), b64))
}

pub struct PreparedEntry {
    pub html: String,
    pub css: String,
    /// (label, phon, audio_ref)
    pub prons: Vec<(String, String, Option<String>)>,
}

/// 净化 + 资源化词条 HTML, 并本地化 CSS
pub fn prepare_entry(html: &str, css: &str, zoom: f64) -> PreparedEntry {
    let html = sanitize_html(html);
    let prons = collect_prons(&html);
    let css = localize_css(css, zoom);
    // DIC_DUMP=1: 把净化后的渲染输入落盘 (排查排版问题用, 见 README)
    if crate::paths::dump_enabled() {
        crate::paths::dump_html(&html, &css);
    }
    PreparedEntry { html, css, prons }
}

/// 收集工具栏发音按钮: DOM 音标 + MDD 音频引用按变体配对
fn collect_prons(html: &str) -> Vec<(String, String, Option<String>)> {
    let dom_prons = crate::textrender::extract_prons_from_html(html);
    let refs = mdict::parse_audio_refs(html);
    let gb = refs
        .iter()
        .find(|r| basename_lc(r).contains("_gb_"))
        .cloned();
    let us = refs
        .iter()
        .find(|r| basename_lc(r).contains("_us_"))
        .cloned();
    let mut out: Vec<(String, String, Option<String>)> = Vec::new();
    let find_phon = |label: &str| -> String {
        dom_prons
            .iter()
            .find(|(l, _)| l == label)
            .map(|(_, p)| p.clone())
            .unwrap_or_default()
    };
    if gb.is_some() || us.is_some() {
        if gb.is_some() {
            out.push(("BrE".to_string(), find_phon("BrE"), gb));
        }
        if us.is_some() {
            out.push(("NAmE".to_string(), find_phon("NAmE"), us));
        }
    } else if let Some(first) = refs.first() {
        let phon = dom_prons.first().map(|(_, p)| p.clone()).unwrap_or_default();
        out.push(("🔊".to_string(), phon, Some(first.clone())));
    }
    out
}

fn basename_lc(p: &str) -> String {
    Path::new(&p.replace('\\', "/"))
        .file_name()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

/// HTML 净化 (对应 TS prepare)
pub fn sanitize_html(html_text: &str) -> String {
    let mut s = html_text.replace("<xhtml:", "<").replace("</xhtml:", "</");
    // 去脚本
    s = strip_between(&s, "<script", "</script");
    s = strip_tag_re(&s, r"(?is)<script\b[^>]*/?>");
    // 去事件属性 / unselectable / user-select:none
    s = strip_re(&s, r#"(?is)\s+on[a-z]+\s*=\s*("[^"]*"|'[^']*'|[^\s>]+)"#);
    s = strip_re(&s, r#"(?is)\s+unselectable\s*=\s*['"]?(on|true)['"]?"#);
    s = strip_re(&s, r#"(?is)(style\s*=\s*['"][^'"]*?)user-select\s*:\s*none\s*;?"#);
    // 去文档级标签 (inner_html 注入不需要, 也避免 HTML5 树构建歧义)
    s = strip_re(&s, r"(?is)<!DOCTYPE[^>]*>");
    s = strip_re(&s, r"(?is)</?html\b[^>]*>");
    s = strip_re(&s, r"(?is)</?head\b[^>]*>");
    s = strip_re(&s, r"(?is)</?body\b[^>]*>");
    s = strip_re(&s, r"(?is)<link\b[^>]*>");
    s = strip_re(&s, r"(?is)<meta\b[^>]*>");

    let Some(root) = parse_html(&s) else {
        return s;
    };

    let res_skip = Regex::new(RES_SKIP).unwrap();
    let snd = Regex::new(r"(?i)^sound://|\.mp3(\?|#|$)").unwrap();
    let media_attr = ["href", "src", "data-src-mp3", "src-mp3", "data-sound"];

    // ---- Pass A: img / 音频引用 / a 标签 ----
    for el in iter_all(&root) {
        let tag = tag_of(&el);
        if tag == "img" {
            let v = attr_of(&el, "src").unwrap_or_default();
            let v = v.trim().to_string();
            if !v.is_empty() && !res_skip.is_match(&v) {
                let normalized = v.replace('\\', "/").trim_start_matches('/').to_string();
                if let Some(uri) = data_uri_for(&normalized) {
                    set_attr(&el, "src", &uri);
                }
            }
            del_attr(&el, "srcset");
        }
        let mut ref_: Option<String> = None;
        // 收集要删除的属性名快照
        let attr_names: Vec<String> = match &el.data {
            markup5ever_rcdom::NodeData::Element { attrs, .. } => attrs
                .borrow()
                .iter()
                .map(|a| a.name.local.as_ref().to_string())
                .collect(),
            _ => Vec::new(),
        };
        for attr in &attr_names {
            let a = attr.to_lowercase();
            if !media_attr.contains(&a.as_str()) {
                continue;
            }
            let v = attr_of(&el, attr).unwrap_or_default().trim().to_string();
            if a == "data-sound" {
                if ref_.is_none() && !v.is_empty() {
                    ref_ = Some(v);
                }
            } else if snd.is_match(&v) || a == "data-src-mp3" || a == "src-mp3" {
                if ref_.is_none() {
                    ref_ = Some(v.trim_start_matches("sound://").to_string());
                }
                del_attr(&el, attr);
            }
        }
        if let Some(r) = ref_ {
            if !r.is_empty() {
                set_attr(&el, "data-sound", &r);
            }
        }
        if tag == "a" {
            for junk in ["href", "target", "onclick"] {
                del_attr(&el, junk);
            }
        }
    }

    // ---- Pass B: pron 块传播 data-sound ----
    for el in iter_all(&root) {
        let ref_ = attr_of(&el, "data-sound");
        let Some(ref_) = ref_ else { continue };
        if ref_.is_empty() {
            continue;
        }
        let Some(p) = parent_of(&el) else { continue };
        if attr_of(&p, "data-sound").is_some() {
            continue;
        }
        let pcls = attr_of(&p, "class").unwrap_or_default().to_lowercase();
        if !pcls.contains("pron") {
            continue;
        }
        let mut refs = std::collections::HashSet::new();
        for s in iter_all(&p) {
            if let Some(v) = attr_of(&s, "data-sound") {
                if !v.is_empty() {
                    refs.insert(v);
                }
            }
        }
        if refs.len() == 1 {
            set_attr(&p, "data-sound", &ref_);
        }
    }

    let mut out = serialize_doc(&root);
    // Blitz 0.2.4 不给 inline 盒算 padding/margin (实测: 对 inline span 设 40px
    // padding-left 或 margin-left, 布局宽度一像素都不变), 所以需要空隙的地方
    // 只能用真实空白字符。下面这些在原词典里靠 CSS 间距/标签分隔:
    // 注意空白要落在"两个盒之间": 落进行内盒尾部会被当行尾空白裁掉 (踩过 [I]to)。
    out = out.replace("</gram-blk><def", "</gram-blk> <def"); // [I]to move → [I] to move
    out = out.replace("</gram-g><def", "</gram-g> <def"); // 同上 (gram-blk 外面还套一层)
    out = out.replace("</infl><pron", "</infl> <pron"); // runBrE → run BrE
    out = out.replace("</pron-g-blk><pron-g-blk>", "</pron-g-blk> <pron-g-blk>"); // /rʌn/NAmE
    out = out.replace(
        r#"</div><div class="cixing_tiaozhuan_part">"#,
        r#"</div> <div class="cixing_tiaozhuan_part">"#, // verb,noun → verb, noun
    );
    // 兜底: 清掉残留的 sound:// 引用 (对应 TS 最后两步 replace)
    out = strip_re(&out, r#"(?is)\s+href\s*=\s*["']?sound://[^\s>"']*"#);
    out = strip_re(&out, r#"(?is)\s+src\s*=\s*["']?sound://[^\s>"']*"#);
    out
}

/// CSS 本地化: url(...) → data URI; 并按 zoom 缩放字号
pub fn localize_css(css: &str, zoom: f64) -> String {
    let css = flatten_table_display(css);
    // 注意: regex crate 不支持反向引用, url() 的三种引号形态各写一条;
    // 无引号条的字符类排除引号, 因此不会二次命中已重写成的 url("data:...")
    let dq = Regex::new(r#"(?is)url\(\s*"([^"]+)"\s*\)"#).unwrap();
    let sq = Regex::new(r"(?is)url\(\s*'([^']+)'\s*\)").unwrap();
    let bare = Regex::new(r#"(?is)url\(\s*([^'")]+)\s*\)"#).unwrap();
    let rewrite = |caps: &regex::Captures| {
        let whole = &caps[0];
        let u = caps[1].trim();
        if u.is_empty() || Regex::new(RES_SKIP).unwrap().is_match(u) {
            return whole.to_string();
        }
        let normalized = u.replace('\\', "/").trim_start_matches('/').to_string();
        match data_uri_for(&normalized) {
            Some(uri) => format!("url(\"{uri}\")"),
            None => whole.to_string(),
        }
    };
    let css = dq.replace_all(&css, rewrite).to_string();
    let css = sq.replace_all(&css, rewrite).to_string();
    let css = bare.replace_all(&css, rewrite).to_string();
    scale_font_sizes(&extra_css_merged(&css), zoom)
}

/// 便携包用户样式补丁: `<exe>/config/extra.css` 存在时追加到词典 CSS 之后。
///
/// 用途: 词典自带的 CSS 在 Blitz 下总有渲染偏差 (表格/浮动/字体), 而又不该
/// 让用户去改 .mdx/.mdd 包里的样式 — 这里给一个可直接编辑的覆盖入口。
/// 只读一次 (OnceLock), 避免每次查词都读盘。
fn extra_css_merged(css: &str) -> String {
    match crate::paths::extra_css_content() {
        Some(extra) => format!("{css}\n/* --- config/extra.css --- */\n{extra}"),
        None => css.to_string(),
    }
}

/// Blitz 0.2.4 的表格布局会越界崩溃 (blitz-dom src/layout/table.rs:333),
/// 而词条里的 table 只是排版技巧 (例如孤立的 display:table-cell 会被包成
/// 匿名表格)。统一降级: table-cell → inline-block, 其余 table* → block。
fn flatten_table_display(css: &str) -> String {
    Regex::new(concat!(
        r"(?i)(display\s*:\s*)(",
        r"inline-table|table-row-group|table-header-group|table-footer-group|",
        r"table-row|table-cell|table-column-group|table-column|table-caption|table)\b"
    ))
    .unwrap()
    .replace_all(css, |c: &regex::Captures| {
        let v = c[2].to_ascii_lowercase();
        let repl = if v == "table-cell" { "inline-block" } else { "block" };
        format!("{}{}", &c[1], repl)
    })
    .to_string()
}

/// 按缩放倍数放大 CSS 中的 px/pt 字号 (Blitz/Stylo 不支持 zoom 属性)
pub fn scale_font_sizes(css: &str, zoom: f64) -> String {
    if (zoom - 1.0).abs() < 0.001 {
        return css.to_string();
    }
    let re = Regex::new(r"(?is)(font-size\s*:\s*)([\d.]+)(px|pt)").unwrap();
    re.replace_all(css, |caps: &regex::Captures| {
        let n: f64 = caps[2].parse().unwrap_or(0.0);
        format!("{}{:.2}{}", &caps[1], n * zoom, &caps[3])
    })
    .to_string()
}

// ---------------------------------------------------------------- 小工具

fn strip_re(s: &str, pattern: &str) -> String {
    Regex::new(pattern).unwrap().replace_all(s, "").to_string()
}

/// 删除 <open ...>...</close> 区间 (大小写不敏感; 用 ASCII 小写保证字节对齐)
fn strip_between(s: &str, open: &str, close: &str) -> String {
    let lower = s.to_ascii_lowercase();
    let open = open.to_ascii_lowercase();
    let close = close.to_ascii_lowercase();
    let mut out = String::with_capacity(s.len());
    let mut pos = 0usize;
    while pos < s.len() {
        match lower[pos..].find(&open) {
            Some(rel) => {
                let i = pos + rel;
                out.push_str(&s[pos..i]);
                let after = i + open.len();
                match lower[after..].find(&close) {
                    Some(rel2) => {
                        let mut end = after + rel2 + close.len();
                        // 吃掉闭合标签后面紧跟的 '>' (处理 </script > 这种写法)
                        if let Some(gt) = s[end..].find('>') {
                            if s[end..end + gt].trim().is_empty() {
                                end += gt + 1;
                            }
                        }
                        pos = end;
                    }
                    None => return out, // 没有闭合标签: 丢掉剩余内容
                }
            }
            None => {
                out.push_str(&s[pos..]);
                return out;
            }
        }
    }
    out
}

fn strip_tag_re(s: &str, pattern: &str) -> String {
    Regex::new(pattern).unwrap().replace_all(s, "").to_string()
}

/// 在线页噪声清理: 抓下来的是整页 HTML, 导航/页头页脚/登录/搜索框/侧栏/分享
/// 这些对阅读没用, 元素层级又因站点而异 → 按标签名 + class/id 关键字整块摘掉。
/// (Blitz 没有 JS, 不能靠脚本删; 用 CSS 藏也得逐个站点写选择器)
const WEB_NOISE_TAGS: &[&str] = &["nav", "header", "footer", "aside", "form", "iframe", "svg"];

/// 站点固定的噪声类名 (整词精确匹配): 有道的结果目录条/返回顶部/投票条/广告位、
/// 列表里的空分隔项 —— 这些词放在正文里也人畜无害, 精确匹配不会伤到正文。
const WEB_NOISE_EXACT: &[&str] = &[
    "result_navigator",
    "sub-catalog",
    "go-top",
    "dict-votebar",
    "topImgAd",
    "split",
];

fn prune_web_noise(html: &str) -> String {
    let Some(root) = parse_html(html) else {
        return html.to_string();
    };
    // 关键字都带词界外的常见前缀, 覆盖 #nav / .header-inner / .doc-search 之类
    let noise = Regex::new(
        r"(?i)^(top)?(nav|header|footer|banner|login|signin|user|toolbar|topbar|menu|search|crumb|sidebar|side|share|comment|related|recommend|qrcode|gotop|go-top|backtop|back-to-top|copyright|promo|advert|adsense|footer|feedback)s?([-_].*)?$",
    )
    .unwrap();
    let mut kill = Vec::new();
    for el in iter_all(&root) {
        let tag = tag_of(&el);
        if WEB_NOISE_TAGS.contains(&tag.as_str()) {
            kill.push(el);
            continue;
        }
        // 在线页的图片是远程 http(s) 的, 渲染端只有 data: URI 通道 → 加载不出来,
        // 但 width/height 属性仍会占位 (必应的 4 张缩略图就留下一大片空白)。
        if tag == "img" {
            let src = attr_of(&el, "src").unwrap_or_default();
            if !src.starts_with("data:") {
                kill.push(el);
                continue;
            }
        }
        // 只按"整个 class/id 词"匹配, 避免 .headerword 这类正文类名被误杀
        let names = class_list(&el);
        let id = attr_of(&el, "id").unwrap_or_default();
        let hit = names.iter().any(|c| noise.is_match(c) || WEB_NOISE_EXACT.contains(&c.as_str()))
            || (!id.is_empty()
                && (noise.is_match(&id) || WEB_NOISE_EXACT.contains(&id.as_str())));
        if hit {
            kill.push(el);
        }
    }
    let n = kill.len();
    for el in kill {
        drop_tree(&el);
    }
    // 摘掉噪声后常留下"空壳"块 (广告位 div 带固定高度、空 li 画个圆点)。
    // 从后往前遍历, 让嵌套空壳能层层塌掉。
    // 只保留"本身就是内容"的元素; td/th/tr 也算空壳 —— 必应词典页是两栏表格,
    // 侧栏被摘掉后右侧空单元格还在, 会把正文挤在左半边、下面留一大片空白。
    let keep_tags = ["br", "img", "hr", "input", "source", "video", "audio"];
    let all = iter_all(&root);
    let mut empty = 0usize;
    for el in all.iter().rev() {
        let tag = tag_of(el);
        if keep_tags.contains(&tag.as_str()) {
            continue;
        }
        if !text_content(el).trim().is_empty() {
            continue;
        }
        if iter_all(el)
            .iter()
            .any(|d| matches!(tag_of(d).as_str(), "img" | "svg" | "video" | "audio" | "iframe"))
        {
            continue;
        }
        drop_tree(el);
        empty += 1;
    }
    // 抓下来的页面常用固定 height/min-height 撑版式 (广告位、清浮动残留),
    // 删掉噪声元素后就变成大片空白 → 只摘掉高度类声明, 其它内联样式保留。
    let mut stripped = 0usize;
    for el in iter_all(&root) {
        let Some(style) = attr_of(&el, "style") else {
            continue;
        };
        let decls: Vec<&str> = style.split(';').filter(|d| !d.trim().is_empty()).collect();
        let kept: Vec<&str> = decls
            .iter()
            .copied()
            .filter(|d| {
                let name = d.split(':').next().unwrap_or("").trim().to_lowercase();
                !(name.ends_with("height") || name == "overflow")
            })
            .collect();
        if kept.len() == decls.len() {
            continue;
        }
        stripped += 1;
        if kept.is_empty() {
            del_attr(&el, "style");
        } else {
            set_attr(&el, "style", &format!("{};", kept.join(";")));
        }
    }

    if crate::paths::dump_enabled() {
        eprintln!("[web] 摘掉 {n} 个导航/无关块, {empty} 个空壳, {stripped} 个内联高度");
    }
    let out = serialize_doc(&root);
    if crate::paths::dump_enabled() {
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let i = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        crate::paths::dump_web(i, &out);
    }
    out
}

/// 在线词典页抓取后的净化 (对应 online 页的清理)
pub fn sanitize_web_html(html: &str, base_url: &str) -> String {
    let mut s = html.to_string();
    s = strip_between(&s, "<script", "</script");
    s = strip_re(&s, r"(?is)<script\b[^>]*/?>");
    s = strip_between(&s, "<style", "</style");
    s = strip_re(&s, r"(?is)<link\b[^>]*>");
    s = strip_between(&s, "<!--", "-->");
    s = strip_re(&s, r"(?is)<noscript\b[^>]*>.*?</noscript\s*>");
    s = strip_re(&s, r#"(?is)\s+on[a-z]+\s*=\s*("[^"]*"|'[^']*'|[^\s>]+)"#);
    // 相对 URL → 绝对 URL
    // 注意: regex crate 不支持反向引用 (如 \2), 单/双引号各写一条
    if let Ok(base) = reqwest::Url::parse(base_url) {
        let dq = Regex::new(r#"(?is)\b(href|src)\s*=\s*"([^"]*)""#).unwrap();
        let sq = Regex::new(r"(?is)\b(href|src)\s*=\s*'([^']*)'").unwrap();
        let absolutize = |caps: &regex::Captures, quote: &str| -> String {
            let v = caps[2].trim();
            // 注意: 协议相对 URL (//host/path) 不跳过, 交给 Url::join 补全 scheme
            if v.is_empty()
                || v.starts_with('#')
                || v.starts_with("data:")
                || v.starts_with("http://")
                || v.starts_with("https://")
                || v.starts_with("mailto:")
            {
                return caps[0].to_string();
            }
            match base.join(v) {
                Ok(abs) => format!("{}={quote}{abs}{quote}", &caps[1]),
                Err(_) => caps[0].to_string(),
            }
        };
        s = dq.replace_all(&s, |c: &regex::Captures| absolutize(c, "\"")).to_string();
        s = sq.replace_all(&s, |c: &regex::Captures| absolutize(c, "'")).to_string();
    }
    prune_web_noise(&s)
}

/// 在线页阅读器样式 (替代原 iframe 方案后的默认排版)
pub fn reader_css() -> &'static str {
    r#"
body { font-family: 'Noto Sans CJK SC', '微软雅黑', sans-serif; font-size: 16px; line-height: 1.6; margin: 0; color: #222; }
img { max-width: 100%; }
table { border-collapse: collapse; }
td, th { padding: 4px 8px; }
a { color: #1a5fb4; }
"#
}

/// 页面标题提取 (展示用; 标题内容是 HTML 源码, 需解码实体后再当纯文本展示)
pub fn extract_title(html: &str) -> Option<String> {
    let re = Regex::new(r"(?is)<title[^>]*>(.*?)</title\s*>").ok()?;
    let caps = re.captures(html)?;
    let t = caps[1].trim();
    if t.is_empty() {
        return None;
    }
    let decoded = crate::dom::parse_html(&format!("<x-t>{t}</x-t>"))
        .map(|root| crate::dom::text_content(&root))
        .unwrap_or_else(|| t.to_string());
    Some(decoded.chars().take(60).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_removes_script_and_events() {
        let html = r#"<div onclick="x()"><script>alert(1)</script><a href="sound://a.spx">play</a><img src="k.gif" srcset="x 2x"></div>"#;
        let out = sanitize_html(html);
        assert!(!out.contains("alert"));
        assert!(!out.contains("onclick"));
        assert!(!out.contains("sound://"));
        assert!(!out.contains("srcset"));
        assert!(out.contains("data-sound"));
    }

    #[test]
    fn strips_doc_level_tags() {
        let html = "<!DOCTYPE html><html><head><link rel=\"stylesheet\" href=\"a.css\"></head><body><p>x</p></body></html>";
        let out = sanitize_html(html);
        assert!(out.contains("<p>x</p>"));
        assert!(!out.contains("<link"));
        assert!(!out.contains("<body"));
    }

    #[test]
    fn scale_font_sizes_multiplies() {
        let css = "a { font-size: 17px; } b { font-size:12pt !important; }";
        let out = scale_font_sizes(css, 1.5);
        assert!(out.contains("25.50px"), "{out}");
        assert!(out.contains("18.00pt"), "{out}");
    }

    #[test]
    fn sanitize_web_absolutizes_urls_and_drops_remote_images() {
        let html = r##"<div><a href="/w/eng/x">x</a><img src="//img.com/a.png"><img src="data:image/png;base64,AAA"></div>"##;
        let out = sanitize_web_html(html, "https://www.bing.com/dict/search?q=x");
        assert!(out.contains("https://www.bing.com/w/eng/x"), "{out}");
        // 远程图在渲染端只有 data: 通道 → 加载不出来, 却按 width/height 占一片空白
        assert!(!out.contains("img.com"), "远程图应被摘掉: {out}");
        assert!(out.contains("data:image/png;base64,AAA"), "{out}");
    }

    /// 在线页要摘掉导航/页头页脚/登录/搜索/目录条, 但正文类名不能被误杀
    #[test]
    fn prune_web_noise_drops_navigation_only() {
        let html = r#"<div id="results">
            <div id="header"><a class="go-top">go top</a></div>
            <div id="result_navigator"><h3>目录</h3></div>
            <ul><li class="split"></li></ul>
            <div class="trans-wrapper"><p class="headerword">苹果</p><p>正文</p></div>
            <footer>版权</footer>
        </div>"#;
        let out = prune_web_noise(html);
        assert!(!out.contains("go top"), "{out}");
        assert!(!out.contains("目录"), "{out}");
        assert!(!out.contains("版权"), "{out}");
        assert!(out.contains("苹果"), "{out}");
        assert!(out.contains("正文"), "{out}");
        assert!(out.contains("headerword"), "正文类名不该被摘掉: {out}");
    }

    #[test]
    fn extract_title_works() {
        assert_eq!(extract_title("<html><title>Hi &amp; bye</title></html>").as_deref(), Some("Hi & bye"));
        assert_eq!(extract_title("<html><title>  </title></html>"), None);
        assert_eq!(extract_title("<html></html>"), None);
    }

    /// Blitz 0.2.4 表格布局会越界崩溃, localize_css 必须把所有 display:table*
    /// 降级掉 (真实牛津 CSS 里 x-wr / xr-gs / audio-wr 都用了表格显示)。
    #[test]
    fn localize_css_flattens_table_display() {
        let css = "x-wr{display:table}\
                   xr-gs[xt=see]{display: table;}\
                   audio-wr{display:table-cell}\
                   xhtml\\:tr{display:table-row}\
                   t{display:inline-table}\
                   ok{display:block}";
        let out = localize_css(css, 1.0);
        let low = out.to_lowercase();
        assert!(!low.contains("display:table"), "{out}");
        assert!(!low.contains("display: table"), "{out}");
        assert!(!low.contains("inline-table"), "{out}");
        assert!(out.contains("x-wr{display:block}"), "{out}");
        assert!(out.contains("audio-wr{display:inline-block}"), "{out}");
        assert!(out.contains("xhtml\\:tr{display:block}"), "{out}");
        assert!(out.contains("ok{display:block}"), "{out}");
    }

    /// localize_css 回归: 曾经 url() 正则用了反向引用 \1, regex crate 不支持,
    /// GUI 一启动就在 Regex::new().unwrap() 上 panic。覆盖三种引号形态 +
    /// data:/http: 跳过 + MDD 资源重写成 data: URI。
    #[test]
    fn localize_css_quote_variants_and_data_uri() {
        let testdata = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata");
        assert!(
            testdata.join("Oxford9.mdx").exists(),
            "缺少测试夹具, 请先运行: python tools/make_fixture.py"
        );
        // 指向 testdata 词典 (本测试是唯一读写该环境变量的测试)
        std::env::set_var("GOLDENDICT_DIR", &testdata);

        // 1) 未解析的引用原样保留 (三种引号形态都不 panic)
        let css = r#"a{background:url("nope-xyz.png")}b{background:url('nope-xyz.png')}c{background:url(nope-xyz.png)}"#;
        assert_eq!(localize_css(css, 1.0), css);

        // 2) data: / http(s): 跳过 (RES_SKIP)
        let skip = r#"a{background:url("data:image/png;base64,AA")}b{background:URL('https://x.com/y.png')}"#;
        assert_eq!(localize_css(skip, 1.0), skip);

        // 3) 命中 MDD 资源 → 重写为 data: URI
        let out = localize_css(r#"a{background:url("\apple.png")}"#, 1.0);
        assert!(out.starts_with(r#"a{background:url("data:image/png;base64,"#), "{out}");
        assert!(out.ends_with(r#"")}"#), "{out}");

        // 4) 单引号 / 无引号 / 前导斜杠同样能重写
        let out2 = localize_css(r#"a{background:url('/apple.png')}"#, 1.0);
        assert!(out2.contains("data:image/png;base64,"), "{out2}");
        let out3 = localize_css("a{background:url(apple.png)}", 1.0);
        assert!(out3.contains("data:image/png;base64,"), "{out3}");
    }
}
