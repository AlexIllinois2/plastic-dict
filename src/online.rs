//! 在线词典站点 (移植自 TS 版 src/online.ts)
//!
//! 与原版的差异: 原版在 WebView 里用 iframe 堆叠展示在线词典页;
//! Blitz 没有 JS 引擎/iframe, 改为服务端抓取页面 HTML → 净化 → 阅读器排版渲染,
//! 同时提供"在浏览器中打开"按钮 (<a> 点击由 Dioxus Native 的 navigation
//! provider 自动调用系统浏览器)。

use crate::prepare::sanitize_web_html;
use crate::translate::user_agent;

pub const ONLINE_SITES: &[&str] = &[
    "http://dict.youdao.com/w/eng/%GDWORD%",
    "https://www.bing.com/dict/search?q=%GDWORD%",
];

pub fn online_site_urls(text: &str) -> Vec<String> {
    let q = urlencoding(text.trim());
    ONLINE_SITES
        .iter()
        .filter(|t| t.contains("%GDWORD%"))
        .map(|t| t.replace("%GDWORD%", &q))
        .collect()
}

pub fn online_site_names() -> Vec<String> {
    ONLINE_SITES
        .iter()
        .map(|t| {
            let host = t
                .split("//")
                .nth(1)
                .map(|h| h.split('/').next().unwrap_or("").to_lowercase())
                .unwrap_or_else(|| t.to_lowercase());
            let pairs = [
                ("youdao", "有道"),
                ("baidu", "百度翻译"),
                ("bing", "必应"),
                ("haici", "海词"),
                ("cambridge", "剑桥"),
                ("merriam", "韦氏"),
                ("collins", "柯林斯"),
                ("oxford", "牛津"),
            ];
            for (k, v) in pairs {
                if host.contains(k) {
                    return v.to_string();
                }
            }
            host
        })
        .collect()
}

/// 抓取在线词典页并返回净化后的 HTML (body 内容)
pub async fn fetch_site_page(url: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .user_agent(user_agent())
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("客户端错误: {e}"))?;
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("网络请求失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status().as_u16()));
    }
    let body = resp
        .text()
        .await
        .map_err(|e| format!("读取响应失败: {e}"))?;
    Ok(extract_body(&body, url))
}

/// 提取 <body> 内容并净化
pub fn extract_body(html: &str, base_url: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let body_html = if let Some(i) = lower.find("<body") {
        let start = lower[i..].find('>').map(|g| i + g + 1).unwrap_or(i);
        if let Some(j) = lower[start..].find("</body") {
            &html[start..start + j]
        } else {
            &html[start..]
        }
    } else {
        html
    };
    sanitize_web_html(body_html, base_url)
}

/// 简单 URL 编码 (对应 TS encodeURIComponent 的常用字符集)
fn urlencoding(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn site_urls_and_names() {
        let urls = online_site_urls("hello world");
        assert_eq!(urls[0], "http://dict.youdao.com/w/eng/hello%20world");
        assert_eq!(urls[1], "https://www.bing.com/dict/search?q=hello%20world");
        let names = online_site_names();
        assert_eq!(names, vec!["有道", "必应"]);
    }

    #[test]
    fn urlencoding_cjk() {
        assert_eq!(urlencoding("苹果"), "%E8%8B%B9%E6%9E%9C");
    }

    #[test]
    fn extract_body_finds_body() {
        let html = "<html><head><title>t</title></head><body><p>hi</p></body></html>";
        let out = extract_body(html, "https://x.com/");
        assert!(out.contains("<p>hi</p>"));
        assert!(!out.contains("<title>"));
    }
}
