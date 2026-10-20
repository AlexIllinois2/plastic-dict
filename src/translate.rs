//! 在线翻译 (Bing Edge 翻译 API, 移植自 TS 版 src/translate.ts)

const BING_URL: &str = "https://edge.microsoft.com/translate/translatetext";
const EDGE_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
    (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 Edg/131.0.0.0";

pub fn user_agent() -> &'static str {
    EDGE_UA
}

/// 翻译文本 (异步, GUI 用)
pub async fn translate_text_async(text: &str, target_lang: &str) -> String {
    if text.trim().is_empty() {
        return String::new();
    }
    let url = format!("{BING_URL}?isEnterpriseClient=false&to={target_lang}");
    let client = match reqwest::Client::builder()
        .user_agent(EDGE_UA)
        .timeout(std::time::Duration::from_secs(15))
        .build()
    {
        Ok(c) => c,
        Err(e) => return format!("网络请求失败: {e}"),
    };
    let resp = match client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!([text]))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return format!("网络请求失败: {e}"),
    };
    if !resp.status().is_success() {
        return format!("网络请求失败: HTTP {}", resp.status().as_u16());
    }
    let result: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => return format!("解析响应失败: {e}"),
    };
    match result
        .get(0)
        .and_then(|r| r.get("translations"))
        .and_then(|t| t.get(0))
        .and_then(|t| t.get("text"))
        .and_then(|t| t.as_str())
    {
        Some(t) => t.trim().to_string(),
        None => "解析响应失败".to_string(),
    }
}

/// 翻译文本 (阻塞, CLI 用)
pub fn translate_text_blocking(text: &str, target_lang: &str) -> String {
    if text.trim().is_empty() {
        return String::new();
    }
    let url = format!("{BING_URL}?isEnterpriseClient=false&to={target_lang}");
    let client = match reqwest::blocking::Client::builder()
        .user_agent(EDGE_UA)
        .timeout(std::time::Duration::from_secs(15))
        .build()
    {
        Ok(c) => c,
        Err(e) => return format!("网络请求失败: {e}"),
    };
    let resp = match client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!([text]))
        .send()
    {
        Ok(r) => r,
        Err(e) => return format!("网络请求失败: {e}"),
    };
    if !resp.status().is_success() {
        return format!("网络请求失败: HTTP {}", resp.status().as_u16());
    }
    let result: serde_json::Value = match resp.json() {
        Ok(v) => v,
        Err(e) => return format!("解析响应失败: {e}"),
    };
    match result
        .get(0)
        .and_then(|r| r.get("translations"))
        .and_then(|t| t.get(0))
        .and_then(|t| t.get("text"))
        .and_then(|t| t.as_str())
    {
        Some(t) => t.trim().to_string(),
        None => "解析响应失败".to_string(),
    }
}
