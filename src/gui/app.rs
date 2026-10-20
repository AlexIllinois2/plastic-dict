//! Dioxus Native GUI 应用 (移植自 TS 版 gui.ts + shell.ts 的交互)
//!
//! 与原版的差异:
//!   - WebView(WebKitGTK) + 本地 HTTP 服务 + JS 注入 → Dioxus Native(Blitz) 直渲
//!   - 在线词典 iframe 堆叠 → 服务端 reqwest 抓取 + 净化 + 阅读器排版
//!   - 词条内 audio 点击 (data-sound) → 工具栏发音按钮 (prons)
//!   - Esc 关窗 / Ctrl+= / Ctrl+- / Ctrl+0 缩放 (与原版快捷键一致)

use dioxus::prelude::*;
use keyboard_types::{Key, Modifiers};
use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::config::{self, DicCfg};
use crate::gui::style::{OXFORD9_CSS, SHELL_CSS};
use crate::mdict;
use crate::online;
use crate::prepare;
use crate::translate;
use crate::util::{is_chinese, is_single_word};

// ---------------------------------------------------------------------------
// 视图模型
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq)]
pub enum View {
    /// 空状态 (启动无初始词 / 查询被清空)
    Empty,
    /// 查询中
    Loading { word: String },
    /// 本地词典词条
    Entry(EntryView),
    /// 在线翻译卡片 (本地未收录的单词)
    Translate { word: String, text: String },
    /// 在线词典 (中文/短语/句子, 或未收录时的兜底展示)
    Online { query: String, pages: Vec<PageView> },
}

#[derive(Clone, PartialEq)]
pub struct EntryView {
    pub word: String,
    pub dict_name: String,
    pub html: String,
    pub css: String,
    /// (label, phon, audio_ref)
    pub prons: Vec<(String, String, Option<String>)>,
}

#[derive(Clone, PartialEq)]
pub struct PageView {
    pub name: String,
    pub url: String,
    pub state: PageState,
}

#[derive(Clone, PartialEq)]
pub enum PageState {
    Loading,
    Ready(String),
    Failed(String),
}

/// 词条原始数据 (zoom 变化时重新 prepare)
#[derive(Clone, Default)]
struct RawEntry {
    word: String,
    dict_name: String,
    html: String,
    css: String,
}

// ---------------------------------------------------------------------------
// 应用组件
// ---------------------------------------------------------------------------

#[component]
pub fn App() -> Element {
    let initial = crate::gui::initial_word().to_string();
    let mut query = use_signal(|| initial.clone());
    let mut view = use_signal(|| View::Empty);
    let zoom = use_signal(|| config::cfg_zoom(&config::load_cfg()));
    let toast = use_signal(|| None::<String>);
    let cur_mdx = use_signal(|| None::<String>);
    let cur_word = use_signal(String::new);
    let raw_entry = use_signal(RawEntry::default);

    // 启动时若有初始查询词, 自动查一次
    let mut started = use_signal(|| false);
    use_future(move || {
        // FnMut 闭包不能把 `initial` 移进 async 块 (会被调用多次), 先克隆一份
        let initial = initial.clone();
        async move {
            if !started() && !initial.is_empty() {
                started.set(true);
                do_lookup(
                    &initial,
                    view,
                    zoom,
                    cur_mdx,
                    cur_word,
                    raw_entry,
                );
            }
        }
    });

    // 工具栏发音按钮 (仅词条视图)
    let prons: Vec<(String, String, Option<String>)> = match &*view.read() {
        View::Entry(e) => e.prons.clone(),
        _ => Vec::new(),
    };
    // 在线站点链接 (随当前查询词生成 URL; 空查询时不显示)
    let q_now = query.read().trim().to_string();
    let site_links: Vec<(String, String)> = if q_now.is_empty() {
        Vec::new()
    } else {
        online::online_site_names()
            .into_iter()
            .zip(online::online_site_urls(&q_now))
            .collect()
    };
    let zoom_pct = format!("{}%", (zoom() * 100.0).round() as i64);

    rsx! {
        div {
            class: "root",
            // 全局快捷键: Esc 关闭 / Ctrl+= Ctrl+- Ctrl+0 缩放 / Enter 查词
            // (blitz-dom 无焦点时键盘事件回退根元素, 有焦点时沿祖先链冒泡, 均可到达此处)
            onkeydown: move |e| handle_key(e, query, view, zoom, raw_entry, toast, cur_mdx, cur_word),

            // 应用外壳样式 (工具栏/内容区/Toast)
            style { "{SHELL_CSS}" }

            // ---- 工具栏 ----
            div { class: "toolbar",
                input {
                    value: "{query}",
                    placeholder: "输入单词后回车…",
                    autocomplete: "off",
                    autofocus: "true",
                    oninput: move |e| query.set(e.value()),
                }
                button {
                    onclick: move |_| {
                        let q = query.read().clone();
                        do_lookup(&q, view, zoom, cur_mdx, cur_word, raw_entry);
                    },
                    "查词"
                }
                // 注意: rsx 的 for 循环体内不允许 let 语句, 因此用辅助函数预生成元素列表
                for el in pron_buttons(&prons, cur_mdx, cur_word, toast) {
                    {el}
                }
                for el in site_anchors(&site_links) {
                    {el}
                }
                button {
                    title: "回到当前词条",
                    onclick: move |_| {
                        let w = cur_word.read().clone();
                        if w.is_empty() {
                            view.set(View::Empty);
                        } else {
                            do_lookup(&w, view, zoom, cur_mdx, cur_word, raw_entry);
                        }
                    },
                    // 用汉字而不是 ⌂/⟲ 之类符号: Blitz 只在 emoji 上做字形回退,
                    // 少见的符号字形会直接画成豆腐块
                    "回"
                }
                button {
                    title: "缩小 (Ctrl+-)",
                    onclick: move |_| apply_zoom(zoom() - 0.1, zoom, raw_entry, view, toast),
                    "−"
                }
                span { class: "zoom-val", "{zoom_pct}" }
                button {
                    title: "放大 (Ctrl+=)",
                    onclick: move |_| apply_zoom(zoom() + 0.1, zoom, raw_entry, view, toast),
                    "+"
                }
                button {
                    title: "关闭 (Esc)",
                    onclick: move |_| quit_app(),
                    "✕"
                }
            }

            // ---- 内容区 ----
            div { class: "content",
                {render_view(&view.read())}
            }

            // ---- Toast ----
            if let Some(msg) = &*toast.read() {
                div { class: "toast", "{msg}" }
            }
        }
    }
}

/// 站点名 → 主机名关键字 (用于从当前 query 生成对应 URL)
// ---------------------------------------------------------------------------
// rsx 辅助函数 (rsx 的 for 循环体内不允许 let 语句/元组解构, 故预生成元素列表)
// ---------------------------------------------------------------------------

/// 工具栏发音按钮 (仅词条视图)
fn pron_buttons(
    prons: &[(String, String, Option<String>)],
    cur_mdx: Signal<Option<String>>,
    cur_word: Signal<String>,
    toast: Signal<Option<String>>,
) -> Vec<Element> {
    prons
        .iter()
        .map(|(label, phon, sref)| {
            let label = label.clone();
            let phon = phon.clone();
            let sref = sref.clone();
            rsx! {
                button {
                    key: "pron-{label}",
                    title: "{phon}",
                    onclick: move |_| play_audio(sref.clone(), cur_mdx, cur_word, toast),
                    "🔊 {label}"
                }
            }
        })
        .collect()
}

/// 工具栏在线词典站点链接
fn site_anchors(links: &[(String, String)]) -> Vec<Element> {
    links
        .iter()
        .map(|(name, url)| {
            rsx! {
                a {
                    key: "site-{name}",
                    href: "{url}",
                    title: "在浏览器中打开{name}",
                    "{name}"
                }
            }
        })
        .collect()
}

/// 在线词典页列表
fn online_pages(pages: &[PageView]) -> Vec<Element> {
    pages
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let name = p.name.clone();
            let url = p.url.clone();
            rsx! {
                div { class: "online-page", key: "page-{i}",
                    h3 { "🌐 {name}" }
                    a { class: "open-link", href: "{url}", "在浏览器中打开 ↗" }
                    {page_state_el(&p.state)}
                }
            }
        })
        .collect()
}

fn page_state_el(state: &PageState) -> Element {
    match state {
        PageState::Loading => rsx! { p { class: "hint", "⏳ 加载中…" } },
        PageState::Ready(html) => rsx! {
            div { dangerous_inner_html: "{html}" }
        },
        PageState::Failed(err) => rsx! {
            p { class: "hint", "⛔ 加载失败: {err}" }
        },
    }
}

// ---------------------------------------------------------------------------
// 视图渲染
// ---------------------------------------------------------------------------

fn render_view(view: &View) -> Element {
    match view {
        View::Empty => rsx! {
            div { class: "hint",
                "🔍 plastic-dict (Rust + Blitz)\n\n"
                "· 输入英文单词后回车 — 查询本地 MDX 词典\n"
                "· 输入中文 / 短语 / 句子 — 在线词典\n"
                "· Esc 关闭窗口 · Ctrl+= / Ctrl+- / Ctrl+0 缩放\n"
                "· 工具栏 🔊 按钮播放发音"
            }
        },
        View::Loading { word } => rsx! {
            div { class: "hint", "⏳ 正在查询 “{word}” …" }
        },
        View::Entry(e) => rsx! {
            style { "{e.css}" }
            div { dangerous_inner_html: "{e.html}" }
        },
        View::Translate { word, text } => rsx! {
            div { class: "hint",
                p { b { "{word}" } " （本地未收录，在线翻译）" }
                p { "{text}" }
            }
        },
        View::Online { pages, .. } => rsx! {
            style { "{prepare::reader_css()}" }
            for el in online_pages(pages) {
                {el}
            }
            if pages.is_empty() {
                div { class: "hint", "⛔ 没有可用的在线词典站点" }
            }
        },
    }
}

// ---------------------------------------------------------------------------
// 查词主流程
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn do_lookup(
    word: &str,
    mut view: Signal<View>,
    zoom: Signal<f64>,
    mut cur_mdx: Signal<Option<String>>,
    mut cur_word: Signal<String>,
    mut raw_entry: Signal<RawEntry>,
) {
    let word = word.trim().to_string();
    if word.is_empty() {
        view.set(View::Empty);
        return;
    }
    view.set(View::Loading { word: word.clone() });
    spawn(async move {
        // 中文 / 非单词 → 在线词典
        if is_chinese(&word) || !is_single_word(&word) {
            let names = online::online_site_names();
            let urls = online::online_site_urls(&word);
            let pages: Vec<PageView> = names
                .into_iter()
                .zip(urls.iter().cloned())
                .map(|(name, url)| PageView {
                    name,
                    url,
                    state: PageState::Loading,
                })
                .collect();
            view.set(View::Online {
                query: word.clone(),
                pages,
            });
            // 顺序抓取 (dioxus-native 无 net feature, 嵌套 spawn 不可靠)
            for (i, url) in urls.iter().enumerate() {
                let idx = i;
                match online::fetch_site_page(url).await {
                    Ok(html) => set_page(&mut view, idx, PageState::Ready(html)),
                    Err(e) => set_page(&mut view, idx, PageState::Failed(e)),
                }
            }
            return;
        }

        // 本地 MDX 词典
        let z = zoom();
        let r = mdict::lookup(&word, true);
        if r.found {
            if let Some(html) = r.html {
                let css = format!("{}\n{}", r.css, OXFORD9_CSS);
                let prep = prepare::prepare_entry(&html, &css, z);
                crate::paths::dump_html(&prep.html, &prep.css);
                cur_mdx.set(r.mdx);
                cur_word.set(r.word.clone());
                raw_entry.set(RawEntry {
                    word: r.word.clone(),
                    dict_name: r.dict.clone().unwrap_or_default(),
                    html,
                    css,
                });
                view.set(View::Entry(EntryView {
                    word: r.word,
                    dict_name: r.dict.unwrap_or_default(),
                    html: prep.html,
                    css: prep.css,
                    prons: prep.prons,
                }));
                return;
            }
        }

        // 未收录 → Bing 在线翻译卡片
        view.set(View::Translate {
            word: word.clone(),
            text: String::new(),
        });
        let t = translate::translate_text_async(&word, "zh-Hans").await;
        // 只有仍停留在同一单词的翻译视图时才写入, 避免竞态覆盖
        let same = matches!(&*view.read(), View::Translate { word: w, .. } if *w == word);
        if same {
            view.set(View::Translate { word, text: t });
        }
    });
}

fn set_page(view: &mut Signal<View>, idx: usize, state: PageState) {
    let mut v = view.write();
    if let View::Online { pages, .. } = &mut *v {
        if let Some(p) = pages.get_mut(idx) {
            p.state = state;
        }
    }
}

// ---------------------------------------------------------------------------
// 发音
// ---------------------------------------------------------------------------

fn play_audio(
    sref: Option<String>,
    cur_mdx: Signal<Option<String>>,
    cur_word: Signal<String>,
    toast: Signal<Option<String>>,
) {
    let word = cur_word.read().clone();
    let mdx = cur_mdx.read().clone();
    let mut audio_path: Option<PathBuf> = None;
    if let (Some(m), Some(r)) = (mdx.as_deref(), sref.as_deref()) {
        audio_path = mdict::extract_audio(m, r);
    }
    // 词条兜底: gb / us 变体 (对应 TS apiPlay 的 fallback)
    if audio_path.is_none() {
        for variant in ["gb", "us"] {
            if let Some(p) = mdict::get_playable(&word, variant) {
                audio_path = Some(p);
                break;
            }
        }
    }
    match audio_path {
        Some(p) => {
            let ok = play_with_external_player(&p);
            let name = p
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            show_toast(
                toast,
                if ok {
                    format!("🔊 {name}")
                } else {
                    "⛔ 播放失败 (未找到可用的播放器)".to_string()
                },
            );
        }
        None => show_toast(toast, "⛔ 找不到音频".to_string()),
    }
}

/// 外部播放器 (移植自 TS playAudio: mpv/paplay/aplay/ffplay/cvlc)
///
/// 这几条都是 Linux 上的播放器。Windows 上一个都没有, 而"交给系统默认程序"
/// 会把 mp3 甩给浏览器 (默认关联可能是 Edge), 弹窗口很难受 —— 所以 Windows
/// 优先走 winmm 的 MCI 播放 (进程内, 无窗口, 不依赖第三方播放器)。
fn play_with_external_player(path: &Path) -> bool {
    let ok = play_audio_file(path);
    crate::paths::log_play_entry(path, ok);
    ok
}

/// 真正的播放尝试链 (上面那层只负责记日志)
fn play_audio_file(path: &Path) -> bool {
    #[cfg(windows)]
    if win_audio::play(path) {
        return true;
    }

    let players: [(&str, &[&str]); 5] = [
        ("mpv", &["--no-video", "--really-quiet"]),
        ("paplay", &[]),
        ("aplay", &[]),
        ("ffplay", &["-nodisp", "-autoexit", "-loglevel", "quiet"]),
        ("cvlc", &["--play-and-exit", "--intf", "dummy"]),
    ];
    for (bin, args) in players {
        let mut cmd = std::process::Command::new(bin);
        cmd.args(args)
            .arg(path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .stdin(Stdio::null());
        if cmd.spawn().is_ok() {
            return true;
        }
    }
    // 最后兜底: 系统默认程序 (macOS open / Linux xdg-open / Windows ShellExecute)
    webbrowser::open(&path.to_string_lossy()).is_ok()
}

/// Windows 原生音频播放 (winmm MCI, 支持 mp3/wav/wma)
#[cfg(windows)]
mod win_audio {
    use std::ffi::c_void;
    use std::path::Path;

    #[link(name = "winmm")]
    extern "system" {
        fn mciSendStringW(
            cmd: *const u16,
            ret: *mut u16,
            ret_len: u32,
            hwnd: *mut c_void,
        ) -> u32;
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn send(cmd: &str) -> u32 {
        let c = wide(cmd);
        // SAFETY: c 以 NUL 结尾, 指针在调用期间有效; ret 传空表示不要返回值
        unsafe { mciSendStringW(c.as_ptr(), std::ptr::null_mut(), 0, std::ptr::null_mut()) }
    }

    pub fn play(path: &Path) -> bool {
        // 关掉上一次的别名, 否则 MCI 设备会一直累积
        send("close pd_audio");
        let p = path.to_string_lossy();
        // 不同 Windows 版本对 mp3 的设备名不同, 依次试
        let mut opened = false;
        for dev in ["mpegvideo", "MPEGVideo", ""] {
            let cmd = if dev.is_empty() {
                format!("open \"{p}\" alias pd_audio")
            } else {
                format!("open \"{p}\" type {dev} alias pd_audio")
            };
            if send(&cmd) == 0 {
                opened = true;
                break;
            }
        }
        if !opened {
            return false;
        }
        send("play pd_audio") == 0
    }
}

// ---------------------------------------------------------------------------
// 缩放
// ---------------------------------------------------------------------------

fn apply_zoom(
    z: f64,
    mut zoom: Signal<f64>,
    raw_entry: Signal<RawEntry>,
    mut view: Signal<View>,
    toast: Signal<Option<String>>,
) {
    let z = z.clamp(0.5, 3.0);
    if (z - zoom()).abs() < 1e-9 {
        return;
    }
    zoom.set(z);
    // 持久化 (对应 TS saveCfgMerged({zoom}))
    let mut cfg = DicCfg::default();
    cfg.zoom = Some(z);
    config::save_cfg_merged(&cfg);
    // 词条视图: 用新 zoom 重新生成 CSS
    let raw = raw_entry.read().clone();
    if !raw.word.is_empty() {
        let css = format!("{}\n{}", raw.css, OXFORD9_CSS);
        let prep = prepare::prepare_entry(&raw.html, &css, z);
        view.set(View::Entry(EntryView {
            word: raw.word,
            dict_name: raw.dict_name,
            html: prep.html,
            css: prep.css,
            prons: prep.prons,
        }));
    }
    show_toast(toast, format!("🔍 缩放 {}%", (z * 100.0).round() as i64));
}

// ---------------------------------------------------------------------------
// 键盘
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn handle_key(
    e: KeyboardEvent,
    query: Signal<String>,
    view: Signal<View>,
    zoom: Signal<f64>,
    raw_entry: Signal<RawEntry>,
    toast: Signal<Option<String>>,
    cur_mdx: Signal<Option<String>>,
    cur_word: Signal<String>,
) {
    let modifiers = e.modifiers();
    let key = e.key();
    if modifiers.contains(Modifiers::CONTROL) {
        match key {
            Key::Character(c) if c == "=" || c == "+" => {
                e.prevent_default();
                apply_zoom(zoom() + 0.1, zoom, raw_entry, view, toast);
            }
            Key::Character(c) if c == "-" => {
                e.prevent_default();
                apply_zoom(zoom() - 0.1, zoom, raw_entry, view, toast);
            }
            Key::Character(c) if c == "0" => {
                e.prevent_default();
                apply_zoom(1.0, zoom, raw_entry, view, toast);
            }
            _ => {}
        }
        return;
    }
    match key {
        Key::Enter => {
            // prevent_default 阻止 blitz 的默认按键处理 (表单提交等)
            e.prevent_default();
            let q = query.read().clone();
            do_lookup(&q, view, zoom, cur_mdx, cur_word, raw_entry);
        }
        Key::Escape => {
            e.prevent_default();
            quit_app();
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// 退出
// ---------------------------------------------------------------------------

/// 关闭窗口并退出 (独立成函数: 避免 `std::process::exit` 的 `!` 返回类型
/// 在事件处理器里触发 never type fallback lint)
fn quit_app() {
    std::process::exit(0);
}

// ---------------------------------------------------------------------------
// Toast
// ---------------------------------------------------------------------------

fn show_toast(mut toast: Signal<Option<String>>, msg: String) {
    toast.set(Some(msg));
    spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(3500)).await;
        toast.set(None);
    });
}
