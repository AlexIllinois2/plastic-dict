//! GUI 启动流程 (Dioxus Native + Blitz)
//!
//! 为什么自写 launch 而不用 `dioxus_native::launch_cfg`:
//!   dioxus-native 0.7.10 开启 `net` feature 会拉入 blitz-net → reqwest 默认特性
//!   (native-tls) → Linux 需要 OpenSSL 系统库; 而关闭 `net` 后其内置的
//!   DioxusNativeNetProvider 对任何请求都是 no-op (data: URI 也不解析)。
//!   因此这里复刻 launch_cfg 的流程, 注入 blitz-shell `data-uri` feature 的
//!   DataUriNetProvider (回调走 BlitzShellNetCallback → ResourceLoad 事件),
//!   词典图片全部以 data: URI 内联, 网络请求走本 crate 自己的 reqwest(rustls)。

pub mod app;
pub mod fonts;
pub mod net;
pub mod style;

use blitz_dom::net::Resource;
use blitz_dom::DocumentConfig;
use blitz_html::HtmlProvider;
use blitz_shell::{
    create_default_event_loop, BlitzShellEvent, BlitzShellNetCallback, DataUriNetProvider,
    WindowConfig,
};
use blitz_traits::net::NetCallback;
use dioxus::dioxus_core::{Element, VirtualDom};
use dioxus_native::{
    DioxusNativeApplication, DioxusNativeWindowRenderer, DioxusDocument, LogicalSize,
    WindowAttributes,
};
use std::sync::Arc;
use std::sync::OnceLock;

/// 启动时传入的初始查询词 (对应 TS 版 initialText)
static INITIAL: OnceLock<String> = OnceLock::new();

/// 启动 GUI (阻塞, 直到窗口关闭)
pub fn run(initial: Option<String>) {
    if let Some(w) = initial {
        let _ = INITIAL.set(w);
    }

    // tokio runtime: reqwest 异步客户端需要 reactor/timer
    // (dioxus-native 未开 net feature, 没有内置 runtime; 这里自建并在主线程 enter,
    //  使 dioxus::spawn 里的异步网络请求能复用该 runtime)
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime 创建失败");
    let _guard = rt.enter();

    let cfg = crate::config::load_cfg();
    let _zoom0 = crate::config::cfg_zoom(&cfg);
    let w = crate::config::cfg_int(&cfg, "width", 760, 320, 7680);
    let h = crate::config::cfg_int(&cfg, "height", 860, 300, 4320);
    let maximized = cfg.maximized.unwrap_or(false);

    let mut attrs = WindowAttributes::default()
        .with_title(crate::paths::DICT_NAME)
        .with_inner_size(LogicalSize::new(w as f64, h as f64));
    if maximized {
        attrs = attrs.with_maximized(true);
    }

    launch(app::App, attrs);
}

pub fn initial_word() -> &'static str {
    INITIAL.get().map(|s| s.as_str()).unwrap_or("")
}

/// launch_cfg 的等价复刻 (注入 data-uri net provider, 不开 net feature)
fn launch(app: fn() -> Element, attrs: WindowAttributes) {
    let event_loop = create_default_event_loop::<BlitzShellEvent>();

    // 虚拟 DOM
    let vdom = VirtualDom::new(app);

    // 资源提供者: HTML 解析 (dangerous_inner_html) + data URI 图片 + 链接导航
    let proxy = event_loop.create_proxy();
    let res_callback: Arc<dyn NetCallback<Resource>> = BlitzShellNetCallback::shared(proxy.clone());
    let html_parser_provider = Some(std::sync::Arc::new(HtmlProvider) as _);
    let net_provider = Some(DataUriNetProvider::shared(res_callback));
    let navigation_provider = Some(net::DictNavigationProvider::shared());

    // base_url 设为哑值: 即使有漏网的相对 URL 也能解析 (不会 panic),
    // 且非 data: 请求会被 DataUriNetProvider 忽略 (不会发起网络请求)
    let doc = DioxusDocument::new(
        vdom,
        DocumentConfig {
            net_provider,
            html_parser_provider,
            navigation_provider,
            // 词典 CSS 内嵌的 webfont 只能自己注册 (blitz 0.2.4 对内联 <style>
            // 不加载 @font-face), 见 gui/fonts.rs
            font_ctx: fonts::build_font_ctx(),
            base_url: Some("https://dict.local/".to_string()),
            ..Default::default()
        },
    );
    let renderer = DioxusNativeWindowRenderer::with_features_and_limits(None, None);
    let config = WindowConfig::with_attributes(Box::new(doc) as _, renderer, attrs);

    let mut application = DioxusNativeApplication::new(event_loop.create_proxy(), config);
    event_loop.run_app(&mut application).unwrap();
}
