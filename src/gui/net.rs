//! 自写 Blitz 导航提供者 (net provider 直接用 blitz-shell 的 data-uri feature)
//!
//! 背景: dioxus-native 0.7.10 的 `net` feature 会引入 blitz-net → reqwest
//! 默认特性(native-tls) → Linux 需要 OpenSSL 系统库; 而关闭 `net` 后
//! DioxusNativeNetProvider 对任何请求都是 no-op (连 data: URI 都不解析)。
//! 因此 gui/mod.rs 自写 launch 流程, 并注入:
//!   1. blitz-shell 的 `data-uri` DataUriNetProvider — 只解析 data: URI
//!      (词典图片全部内联), 回调走 BlitzShellNetCallback → ResourceLoad 事件
//!   2. 这里的 DictNavigationProvider — <a href> 点击用系统浏览器打开
//!      (dioxus-native 的 DioxusNativeNavigationProvider 是私有模块, 无法直接复用)

use blitz_traits::navigation::{NavigationOptions, NavigationProvider};
use blitz_traits::net::Method;
use std::sync::Arc;

/// `<a href>` 点击 → 系统浏览器 (http/https/mailto), 逻辑同 dioxus-native
/// 的 DioxusNativeNavigationProvider
pub struct DictNavigationProvider;

impl NavigationProvider for DictNavigationProvider {
    fn navigate_to(&self, options: NavigationOptions) {
        if options.method == Method::GET
            && matches!(options.url.scheme(), "http" | "https" | "mailto")
        {
            if let Err(e) = webbrowser::open(options.url.as_str()) {
                crate::paths::log_err(format!("打开链接失败 {}: {e}", options.url));
            }
        }
    }
}

impl DictNavigationProvider {
    pub fn shared() -> Arc<dyn NavigationProvider> {
        Arc::new(Self) as Arc<dyn NavigationProvider>
    }
}
