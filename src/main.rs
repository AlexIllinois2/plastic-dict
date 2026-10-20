//! plastic-dict — 丐版 GoldenDict for Linux
//! Rust + Dioxus Native(Blitz) 重写版
//!
//! 用法:
//!   plastic-dict "<content>"     英文单词→GUI 本地词典; 中文/短语/句子→GUI 在线词典
//!   plastic-dict -g "<content>"  同上 (显式 GUI)
//!   plastic-dict -w "<content>"  强制 GUI (兼容旧参数)
//!   plastic-dict -t "<content>"  命令行模式 (中文/短语时弹 GUI 在线词典窗口)
//!   plastic-dict --purge-cache   清空词典索引磁盘缓存
//!   plastic-dict -h              帮助

// 移植期 API 面: 部分函数/字段对应 TS 版能力, 当前分支尚未接线 (如调试 dump、
// 备用查询路径), 暂允许 dead_code 噪音, 接线后自然消除。
#![allow(dead_code)]

mod config;
mod dom;
mod gui;
mod mdict;
mod online;
mod paths;
mod prepare;
mod textrender;
mod translate;
mod util;

use std::io::Write;

/// 写 stdout 并吞掉写入错误。
/// Windows 没有 SIGPIPE, `println!` 撞上已关闭的管道 (如 `plastic-dict -t run | head`)
/// 会 panic: failed printing to stdout: 管道正在被关闭 (os error 232)。踩过。
macro_rules! outln {
    ($($arg:tt)*) => {{
        let _ = writeln!(std::io::stdout(), $($arg)*);
    }};
}

fn print_usage() {
    let msg = [
        "用法:",
        "  plastic-dict \"<content>\"   英文单词→GUI 本地词典; 中文/短语/句子→GUI 在线词典",
        "  plastic-dict -g \"<content>\" 同上",
        "  plastic-dict -w \"<content>\" 强制 GUI (与默认一致, 保留参数兼容)",
        "  plastic-dict -t \"<content>\" 命令行模式 (中文/短语时弹 GUI 在线词典窗口)",
        "  plastic-dict --purge-cache  清空词典索引磁盘缓存",
        "",
        "便携包: 词典/配置/缓存/日志都在本二进制所在目录 (dicts/ config/ cache/ logs/)",
        "配置: config/config.json (zoom/width/height/maximized)",
    ]
    .join("\n");
    outln!("{msg}");
}

/// 安装 panic hook: 崩溃信息写入 <exe>/logs/crash.log (对应 TS uncaughtException)
fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if let Some(dir) = paths::crash_log().parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(paths::crash_log())
        {
            let loc = info
                .location()
                .map(|l| format!("{}:{}", l.file(), l.line()))
                .unwrap_or_default();
            let payload = info
                .payload()
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| info.payload().downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic".to_string());
            let _ = writeln!(f, "[{now}] panic at {loc}: {payload}");
            let bt = std::backtrace::Backtrace::capture();
            let _ = writeln!(f, "backtrace status: {:?}", bt.status());
        }
        default(info);
    }));
}

/// 命令行纯文本模式 (对应 TS runCli)
fn run_cli(content: &str) -> i32 {
    let content = content.trim();
    if content.is_empty() {
        return 1;
    }
    // 中文输入 / 非单词 → 打开 GUI 在线词典窗口
    if util::is_chinese(content) || !util::is_single_word(content) {
        outln!("🌐 中文/非单词输入: 打开在线词典窗口…");
        gui::run(Some(content.to_string()));
        return 0;
    }
    let r = mdict::lookup(content, true);
    if !r.found {
        let out = translate::translate_text_blocking(content, "zh-Hans");
        outln!("{out}");
        return if out.is_empty() { 1 } else { 0 };
    }
    outln!("--- {} | {}\n", r.dict.as_deref().unwrap_or(""), r.word);
    if let Some(t) = &r.text {
        outln!("{t}");
    }
    for variant in ["gb", "us"] {
        if let Some(p) = mdict::get_playable(&r.word, variant) {
            outln!("  🎧 {p:?}");
        }
    }
    0
}

fn main() {
    install_panic_hook();
    paths::ensure_data_dirs();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("用法: plastic-dict [-g|-w|-t] \"<content>\"");
        std::process::exit(1);
    }
    match args[0].as_str() {
        "--native" => {
            eprintln!("Qt 原生窗口已移除, 请直接使用默认模式或 -w (Dioxus Native GUI)。");
            let content = args[1..].join(" ");
            if !content.trim().is_empty() {
                gui::run(Some(content));
            }
        }
        "--purge-cache" => {
            let n = mdict::cache::purge_all_cache();
            outln!(
                "已清理 {n} 个索引缓存文件 ({})",
                paths::index_cache_dir().display()
            );
        }
        "--web" => {
            let content = args[1..].join(" ");
            if !content.trim().is_empty() {
                gui::run(Some(content));
            }
        }
        "-g" | "--gui" | "-w" | "--webview" => {
            let content = args[1..].join(" ");
            if content.trim().is_empty() {
                eprintln!("用法: plastic-dict {} \"<content>\"", args[0]);
                std::process::exit(1);
            }
            gui::run(Some(content));
        }
        "-t" | "--text" => {
            let content = args[1..].join(" ");
            if content.trim().is_empty() {
                eprintln!("用法: plastic-dict -t \"<content>\"");
                std::process::exit(1);
            }
            std::process::exit(run_cli(&content));
        }
        "-h" | "--help" => print_usage(),
        other => {
            let content = args.join(" ");
            if content.trim().is_empty() {
                eprintln!("用法: plastic-dict [-g|-w|-t] \"<content>\"");
                std::process::exit(1);
            }
            let _ = other;
            gui::run(Some(content));
        }
    }
}
