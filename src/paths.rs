//! 路径与全局常量 (便携包模式)
//!
//! 除发音临时文件外, 所有数据 (词典/配置/缓存/日志) 都放在**本二进制所在目录**下,
//! 整个目录拷走即迁移:
//!   <exe_dir>/dicts/            词典 (.mdx/.mdd/.css; GOLDENDICT_DIR 可覆盖)
//!   <exe_dir>/config/config.json
//!   <exe_dir>/cache/index/      索引缓存 (.pdc)
//!   /tmp/plastic-dict/audio/    发音落地 (一次性小文件, 不放进便携目录)
//!   <exe_dir>/logs/             crash.log / play.log / DIC_DUMP 输出

use std::path::PathBuf;

/// 本二进制所在目录 (便携包模式的数据根)。
/// canonicalize 是为了解析符号链接/短路径拿到真实目录; 但 Windows 上它的
/// 返回值带 `\\?\` 前缀 (功能无碍, 显示难看), 这里去掉。
/// 取不到时退化为当前目录。
pub fn exe_dir() -> PathBuf {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return PathBuf::from("."),
    };
    let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
    let exe = exe.to_string_lossy().to_string();
    let exe = exe.strip_prefix(r"\\?\").unwrap_or(&exe).to_string();
    PathBuf::from(exe)
        .parent()
        .map(|d| d.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// 展开 `~` / `~/` (仅 GOLDENDICT_DIR 覆盖值需要)
fn expand_user(p: &str) -> String {
    if p == "~" {
        return home_dir();
    }
    if let Some(rest) = p.strip_prefix("~/") {
        return join_home(rest);
    }
    p.to_string()
}

fn home_dir() -> String {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| ".".to_string())
}

fn join_home(rest: &str) -> String {
    let home = home_dir();
    let sep = if cfg!(windows) { '\\' } else { '/' };
    if home.ends_with(['/', '\\']) {
        format!("{home}{rest}")
    } else {
        format!("{home}{sep}{rest}")
    }
}

/// 词典目录 (默认 <exe>/dicts, 可用 GOLDENDICT_DIR 覆盖, 支持 ~)
pub fn dict_dir() -> PathBuf {
    match std::env::var("GOLDENDICT_DIR") {
        Ok(v) if !v.trim().is_empty() => PathBuf::from(expand_user(v.trim())),
        _ => exe_dir().join("dicts"),
    }
}

/// 发音音频解压落地目录
///
/// 这些是按词生成的一次性小文件 (每次播放都重新解压写盘), 留在便携目录里
/// 既无意义又会不断累积, 所以 Linux 上放到 /tmp (重启自动清理);
/// Windows 没有 /tmp 约定, 仍放 <exe>/cache/audio。
pub fn audio_dir() -> PathBuf {
    #[cfg(unix)]
    {
        PathBuf::from("/tmp").join(DICT_NAME).join("audio")
    }
    #[cfg(not(unix))]
    {
        exe_dir().join("cache").join("audio")
    }
}

/// 词典索引磁盘缓存目录
pub fn index_cache_dir() -> PathBuf {
    exe_dir().join("cache").join("index")
}

/// 配置文件路径
pub fn cfg_path() -> PathBuf {
    exe_dir().join("config").join("config.json")
}

/// 用户样式补丁: `<exe>/config/extra.css` (可选, 追加到词典 CSS 之后)
pub fn extra_css_path() -> PathBuf {
    exe_dir().join("config").join("extra.css")
}

/// 读取 extra.css 内容 (只读一次; 不存在返回 None)
pub fn extra_css_content() -> Option<&'static str> {
    static CACHE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    CACHE
        .get_or_init(|| {
            let p = extra_css_path();
            match std::fs::read_to_string(&p) {
                Ok(s) if !s.trim().is_empty() => Some(s),
                _ => None,
            }
        })
        .as_deref()
}

/// 启动时确保便携数据目录存在 (失败静默, 如只读介质)
pub fn ensure_data_dirs() {
    let _ = std::fs::create_dir_all(dict_dir());
    let _ = std::fs::create_dir_all(index_cache_dir());
    let _ = std::fs::create_dir_all(audio_dir());
    let _ = std::fs::create_dir_all(exe_dir().join("logs"));
    let _ = std::fs::create_dir_all(exe_dir().join("config"));
}

pub const DICT_NAME: &str = "plastic-dict";

/// 崩溃日志
pub fn crash_log() -> PathBuf {
    exe_dir().join("logs").join("crash.log")
}

/// 播放日志
pub fn play_log() -> PathBuf {
    exe_dir().join("logs").join("play.log")
}

/// 记一条播放记录 —— 播放有一串兜底链 (winmm MCI → mpv/paplay/... → 系统默认程序),
/// 点了喇叭没声音时先看 logs/play.log 判断卡在哪一步
pub fn log_play_entry(path: &std::path::Path, ok: bool) {
    use std::io::Write;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Some(dir) = play_log().parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(play_log())
    {
        let _ = writeln!(
            f,
            "[{ts}] {} {}",
            if ok { "ok" } else { "fail" },
            path.display()
        );
    }
}

/// DIC_DUMP=1 时把最近一次词条 HTML/CSS 写到 <exe>/logs/last.html|css
pub fn dump_enabled() -> bool {
    std::env::var("DIC_DUMP").map(|v| v == "1").unwrap_or(false)
}

pub fn dump_html(html: &str, css: &str) {
    if !dump_enabled() {
        return;
    }
    let dir = exe_dir().join("logs");
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join("last.html"), html);
    let _ = std::fs::write(dir.join("last.css"), css);
}

/// 在线词典页净化后的 HTML (排查在线页排版, 每个站点落一个 last_web_N.html)
pub fn dump_web(site_index: usize, html: &str) {
    if !dump_enabled() {
        return;
    }
    let dir = exe_dir().join("logs");
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join(format!("last_web_{site_index}.html")), html);
}

pub fn log_err(args: impl std::fmt::Display) {
    eprintln!("[plastic-dict] {args}");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 便携包核心不变量: 除词典目录 (可被 GOLDENDICT_DIR 覆盖) 和发音临时目录外,
    /// 所有数据路径都必须落在本二进制所在目录下。
    /// (刻意不测 dict_dir: 它与 prepare 测试争抢 GOLDENDICT_DIR 环境变量)
    #[test]
    fn portable_paths_live_under_exe_dir() {
        let root = exe_dir();
        assert!(root.is_absolute(), "exe_dir 应为绝对路径: {root:?}");
        for p in [index_cache_dir(), crash_log(), play_log()] {
            assert!(p.starts_with(&root), "{p:?} 不在 {root:?} 下");
        }
        assert!(cfg_path().starts_with(&root));
        #[cfg(unix)]
        assert_eq!(audio_dir(), PathBuf::from("/tmp/plastic-dict/audio"));
    }

    #[test]
    fn expand_user_passthrough_and_tilde() {
        assert_eq!(expand_user("/abs/path"), "/abs/path");
        assert_eq!(expand_user("rel/path"), "rel/path");
        // "~" 展开成 home (具体值随环境, 只保证不再是 "~")
        assert_ne!(expand_user("~"), "~");
    }
}
