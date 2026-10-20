//! 通用工具函数 (移植自 TS 版 src/utils.ts)

/// 单个单词才走本地词典；短语/句子走在线站点。
pub fn is_single_word(text: &str) -> bool {
    let text = text.trim();
    if text.is_empty() || text.split_whitespace().count() != 1 {
        return false;
    }
    !text
        .chars()
        .any(|c| matches!(c, '.' | '!' | '?' | ',' | ';' | ':' | '。' | '！' | '？' | '，' | '、' | '；' | '：'))
}

/// 是否含中文 (CJK 统一表意文字, 含扩展 A 区与兼容表意文字)
pub fn is_chinese(text: &str) -> bool {
    text.chars().any(|c| matches!(c, '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}' | '\u{f900}'..='\u{faff}'))
}

pub fn esc_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// esc_html + 单引号 (用于属性值)
pub fn e2(s: &str, quote: bool) -> String {
    let mut out = s
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    if quote {
        out = out.replace('"', "&quot;").replace('\'', "&#39;");
    }
    out
}

/// HTML 实体反转义 (header 解析用, 对应 TS unescapeEntities)
pub fn unescape_entities(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
}

/// FNV-1a 64 位哈希 — 用于缓存文件命名 (对应 TS 的 sha1(path), 仅需稳定)
pub fn fnv1a_hex(data: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in data.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}
