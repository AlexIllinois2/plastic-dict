//! 内嵌字体注册 (修 blitz-dom 0.2.4 的 @font-face 两处缺陷)
//!
//! 牛津词典把 Optima / oalecd9 / coresym 三种字体直接 base64 内嵌在 CSS 的
//! `@font-face` 里。blitz-dom 0.2.4 在两个地方拿不到它们:
//!
//!   1. `document::process_style_element` 处理内联 `<style>` 时**不调用**
//!      `net::fetch_font_face` —— 只有外链 `<link rel=stylesheet>` 走
//!      `CssHandler`/`StylesheetLoader` 才会去取字体。本程序的词条 CSS 全部是
//!      内联 `<style>`, 于是字体永远不会加载。
//!   2. 即使取到了, `handle_resource_load` 里是
//!      `register_fonts(blob, None)` —— 忽略 `@font-face` 的 font-family
//!      描述符, 按字体内部 family 名注册。Optima 四个字形的内部名分别是
//!      "Optima LT Medium"/"Optima LT Bold"/... , CSS 里写的却是 `Optima`,
//!      所以照样匹配不上。
//!
//! 这里绕开 blitz 的字体加载链路: 启动时自己扫词典 CSS, 解出内嵌字体字节,
//! 用 `FontInfoOverride` 把 family 名/字重/字形强制指成 CSS 想要的名字, 再通过
//! `DocumentConfig::font_ctx` 把 FontContext 交给文档。
//!
//! 其余部分与 blitz 的默认路径一致 (FontContext::default() 会加载系统字体);
//! 唯一差别是 blitz 会额外塞一个 moz-bullet-font, 那是 pub(crate) 拿不到,
//! 影响仅限 `list-style: disc` 的原生项目符号 (词典条目用 counter 实现编号)。

use regex::Regex;
use std::sync::Arc;

use blitz_dom::FontContext;
use parley::fontique::{Blob, FontInfoOverride, FontStyle, FontWeight};

/// 一条 @font-face 的解析结果
#[derive(Debug, Clone, PartialEq)]
pub struct FontFace {
    /// CSS 里的 font-family (去掉引号)
    pub family: String,
    pub style: FaceStyle,
    pub weight: u16,
    /// base64 解码后的字体字节
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FaceStyle {
    Normal,
    Italic,
    Oblique,
}

/// 扫 CSS 里所有 @font-face, 逐个解出字体字节
///
/// `resolve` 把 src 里的普通路径 (如 `/font/Optima LT Medium.ttf`) 变成字体字节
/// (牛津的 Optima 字体只存在 MDD 里); 内嵌 `data:font/...;base64,` 的源直接解码,
/// 不经过 resolve。
pub fn parse_font_faces(css: &str, resolve: impl Fn(&str) -> Option<Vec<u8>>) -> Vec<FontFace> {
    let block = Regex::new(r"(?is)@font-face\s*\{([^{}]*)\}").unwrap();
    let re_family = Regex::new(r#"(?is)font-family\s*:\s*([^;}]+)"#).unwrap();
    let re_weight = Regex::new(r"(?is)font-weight\s*:\s*([^;}]+)").unwrap();
    let re_style = Regex::new(r"(?is)font-style\s*:\s*([^;}]+)").unwrap();
    let re_url = Regex::new(r#"(?is)url\(\s*["']?([^"')]+?)["']?\s*\)"#).unwrap();
    let re_data = Regex::new(r#"(?is)data:font/[a-z0-9]+[^"')]*"#).unwrap();

    let mut out = Vec::new();
    for caps in block.captures_iter(css) {
        let body = &caps[1];
        let Some(fam) = re_family.captures(body) else {
            continue;
        };
        let family = unquote(&fam[1]);
        if family.is_empty() {
            continue;
        }
        // src 列表按书写顺序取第一个拿得到的 (与浏览器行为一致):
        // 内嵌 base64 直接解; 普通路径交给 resolve 从 MDD 里取
        let mut data: Option<Vec<u8>> = None;
        for u in re_url.captures_iter(body) {
            let src = u[1].trim();
            if src.is_empty() {
                continue;
            }
            if let Some(d) = re_data.find(src) {
                if let Some((_, payload)) = d.as_str().split_once("base64,") {
                    if let Ok(bytes) = decode_b64(payload) {
                        if !bytes.is_empty() {
                            data = Some(bytes);
                            break;
                        }
                    }
                }
            } else if !src.starts_with("http") {
                if let Some(bytes) = resolve(src) {
                    if !bytes.is_empty() {
                        data = Some(bytes);
                        break;
                    }
                }
            }
        }
        let Some(data) = data else {
            continue;
        };
        let weight = re_weight
            .captures(body)
            .map(|c| parse_weight(&c[1]))
            .unwrap_or(400);
        let style = re_style
            .captures(body)
            .map(|c| parse_style(&c[1]))
            .unwrap_or(FaceStyle::Normal);
        out.push(FontFace {
            family: family.to_ascii_lowercase(),
            style,
            weight,
            data,
        });
    }
    out
}

fn unquote(s: &str) -> String {
    s.trim()
        .trim_matches(|c| c == '"' || c == '\'')
        .trim()
        .to_string()
}

fn parse_weight(s: &str) -> u16 {
    let v = s.trim().to_ascii_lowercase();
    match v.as_str() {
        "normal" => 400,
        "bold" | "bolder" => 700,
        "lighter" => 300,
        _ => v
            .split_whitespace()
            .next()
            .and_then(|n| n.parse::<f32>().ok())
            .map(|n| n.clamp(1.0, 1000.0) as u16)
            .unwrap_or(400),
    }
}

fn parse_style(s: &str) -> FaceStyle {
    let v = s.trim().to_ascii_lowercase();
    if v.starts_with("italic") {
        FaceStyle::Italic
    } else if v.starts_with("oblique") {
        FaceStyle::Oblique
    } else {
        FaceStyle::Normal
    }
}

fn decode_b64(s: &str) -> Result<Vec<u8>, ()> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(s.trim())
        .map_err(|_| ())
}

/// 建一个带词典内嵌字体的 FontContext; 没扫到任何字体时返回 None
/// (交回 blitz 的默认路径, 连 bullet font 也不丢)
pub fn build_font_ctx() -> Option<FontContext> {
    let t0 = std::time::Instant::now();
    let faces = collect_dict_faces();
    let ms = t0.elapsed().as_millis();
    if faces.is_empty() {
        return None;
    }
    let mut ctx = FontContext::default();
    for f in &faces {
        let over = FontInfoOverride {
            family_name: Some(&f.family),
            width: None,
            style: Some(match f.style {
                FaceStyle::Normal => FontStyle::Normal,
                FaceStyle::Italic => FontStyle::Italic,
                FaceStyle::Oblique => FontStyle::Oblique(None),
            }),
            weight: Some(FontWeight::new(f.weight as f32)),
            axes: None,
        };
        ctx.collection
            .register_fonts(Blob::new(Arc::new(f.data.clone()) as _), Some(over));
    }
    log_faces(&faces, ms);
    Some(ctx)
}

/// 扫 <exe>/dicts/ 下所有 CSS 的 @font-face。
///
/// 词典 CSS 里 Optima 四个字形写的是 MDD 里的路径 (`/font/Optima LT Medium.ttf`),
/// 所以要把路径解析成字节。这里直接按路径查 MDD —— 不用 mdict::dicres_find,
/// 因为后者查不到时还会退化成"扫全部键名按文件名匹配", 那是几百毫秒级的开销,
/// 而字体路径都是规整的绝对路径, 一次命中。
fn collect_dict_faces() -> Vec<FontFace> {
    let mut raw = String::new();
    for p in crate::mdict::list_css_files(&crate::paths::dict_dir()) {
        if let Ok(css) = std::fs::read_to_string(&p) {
            raw.push_str(&css);
            raw.push('\n');
        }
    }
    if raw.trim().is_empty() {
        return Vec::new();
    }
    // 只留 @font-face 块: 整份 CSS 上百个 url() 全走一遍没必要
    let faces_css = extract_font_face_blocks(&raw);
    parse_font_faces(&faces_css, resolve_font_ref)
}

/// 只留下所有 @font-face{...} 块
fn extract_font_face_blocks(css: &str) -> String {
    let re = Regex::new(r"(?is)@font-face\s*\{[^{}]*\}").unwrap();
    let mut out = String::new();
    for m in re.find_iter(css) {
        out.push_str(m.as_str());
        out.push('\n');
    }
    out
}

/// 从 MDD 里按路径取资源 (与 prepare::dicres_find 的路径归一化一致: 去首斜杠)
fn resolve_font_ref(ref_: &str) -> Option<Vec<u8>> {
    let norm = ref_.replace('\\', "/").trim_start_matches('/').to_string();
    for mdx in crate::mdict::list_mdx_files() {
        if let Some(d) = crate::mdict::multi_mdd_lookup(&mdx.to_string_lossy(), &norm) {
            if !d.is_empty() {
                return Some(d);
            }
        }
    }
    None
}

fn log_faces(faces: &[FontFace], ms: u128) {
    let mut fams: Vec<&str> = faces.iter().map(|f| f.family.as_str()).collect();
    fams.sort_unstable();
    fams.dedup();
    let kb: usize = faces.iter().map(|f| f.data.len()).sum::<usize>() / 1024;
    eprintln!(
        "[font] 注册内嵌字体 {} 个 / {} 个 family / {kb} KB ({ms} ms): {}",
        faces.len(),
        fams.len(),
        fams.join(", ")
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    const CSS: &str = r#"
@font-face{font-family:Optima;src:url("data:font/ttf;base64,AAECAwQ="),local("optima")}
@font-face{font-family:"Optima";src:url("data:font/ttf;base64,AAECAwQ=");font-style:italic;font-weight:600}
@font-face{font-family:coresym;src:url('data:font/ttf;charset=utf-8;base64,BQYH')}
@font-face{font-family:Remote;src:url('/font/x.ttf')}
@font-face{font-family:Woff;src:url("data:font/woff2;base64,CAkK")}
body{color:red}
"#;

    #[test]
    fn parses_inline_faces_and_skips_remote() {
        let faces = parse_font_faces(CSS, |_| None);
        assert_eq!(faces.len(), 4, "取不到路径的 Remote 应被跳过: {faces:#?}");
        assert_eq!(faces[0].family, "optima");
        assert_eq!(faces[0].style, FaceStyle::Normal);
        assert_eq!(faces[0].weight, 400);
        assert_eq!(faces[0].data, vec![0, 1, 2, 3, 4]);
        assert_eq!(faces[1].style, FaceStyle::Italic);
        assert_eq!(faces[1].weight, 600);
        assert_eq!(faces[2].family, "coresym");
        assert_eq!(faces[2].data, vec![5, 6, 7]);
        assert_eq!(faces[3].data, vec![8, 9, 10], "woff2 也要认");
    }

    #[test]
    fn resolves_path_sources_via_callback() {
        // 模拟 /font/x.ttf 在 MDD 里命中: 回退到 resolover
        let got = parse_font_faces(CSS, |r| {
            assert_eq!(r, "/font/x.ttf");
            Some(vec![9, 9, 9])
        });
        assert_eq!(got.len(), 5, "{got:#?}");
        assert_eq!(got[3].family, "remote");
        assert_eq!(got[3].data, vec![9, 9, 9]);
    }

    #[test]
    fn weight_and_style_keywords() {
        assert_eq!(parse_weight("bold"), 700);
        assert_eq!(parse_weight("Normal"), 400);
        assert_eq!(parse_weight(" 800 "), 800);
        assert_eq!(parse_weight("bogus"), 400);
        assert_eq!(parse_style("italic"), FaceStyle::Italic);
        assert_eq!(parse_style("oblique 10deg"), FaceStyle::Oblique);
        assert_eq!(parse_style("normal"), FaceStyle::Normal);
    }
}
