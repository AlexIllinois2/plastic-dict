//! 配置读写 <exe>/config/config.json (便携包模式; 移植自 TS 版 src/config.ts)

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DicCfg {
    #[serde(default)]
    pub zoom: Option<f64>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    #[serde(default)]
    pub maximized: Option<bool>,
    #[serde(flatten)]
    pub extra: std::collections::HashMap<String, serde_json::Value>,
}

pub fn load_cfg() -> DicCfg {
    let path = crate::paths::cfg_path();
    match fs::read_to_string(&path) {
        Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
        Err(_) => DicCfg::default(),
    }
}

/// 合并写入 (只覆盖非 null 字段, 与 TS saveCfgMerged 语义一致)
pub fn save_cfg_merged(extra: &DicCfg) {
    let mut cfg = load_cfg();
    let v = serde_json::to_value(extra).unwrap_or(serde_json::Value::Null);
    if let serde_json::Value::Object(map) = v {
        for (k, val) in map {
            if !val.is_null() {
                cfg.extra.insert(k, val);
            }
        }
    }
    let path = crate::paths::cfg_path();
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let _ = fs::write(&path, serde_json::to_string_pretty(&cfg).unwrap_or_default());
}

/// 从扁平 map 中取整数并 clamp
pub fn cfg_int(cfg: &DicCfg, key: &str, dft: u32, lo: u32, hi: u32) -> u32 {
    let raw = cfg.extra.get(key).and_then(|v| v.as_f64()).unwrap_or(dft as f64);
    (raw.round() as i64).clamp(lo as i64, hi as i64) as u32
}

pub fn cfg_zoom(cfg: &DicCfg) -> f64 {
    cfg.zoom
        .or_else(|| cfg.extra.get("zoom").and_then(|v| v.as_f64()))
        .unwrap_or(1.0)
        .clamp(0.5, 3.0)
}

pub fn _unused(_: &Path) {}
