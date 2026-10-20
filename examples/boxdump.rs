//! 无头布局探针: 把 dump 出来的词条 HTML/CSS 重新组文档, 跑一次布局, 打印节点盒子。
//!
//! 用途: GUI 里看到可疑的错位/溢出时, 不用去截图猜, 直接问布局引擎每个节点在哪。
//!
//! 运行:
//!   cargo +stable-x86_64-pc-windows-msvc run --release --example boxdump -- \
//!       target/release/logs/last.html target/release/logs/last.css 760 860
//!
//! 参数: <html> <css> [viewport_w] [viewport_h] [--min-x=N] [--depth=D]
//!
//! 说明: 为了复刻 GUI 的真实结构, 这里按 app.rs 的 rsx 搭一层
//!   div.root > (style SHELL_CSS) + div.toolbar + div.content > (style css) + div[词条]
//! 图片不加载 (无 net provider), 因此图片尺寸为 0 — 对水平方向的定位问题无影响。

use blitz_dom::DocumentConfig;
use blitz_html::HtmlDocument;
use blitz_traits::shell::{ColorScheme, Viewport};
use std::io::Write;

// 复用真实外壳样式 (本 crate 是纯 bin, example 无法 use crate::, 故按路径引模块;
// 该文件只有两个 pub const &str, 无其它依赖)
#[path = "../src/gui/style.rs"]
mod style;
use style::{OXFORD9_CSS, SHELL_CSS};

fn main() {
    let mut args = std::env::args().skip(1);
    let html_path = args.next().expect("缺少参数: <html>");
    let css_path = args.next().expect("缺少参数: <css>");
    let mut vw: u32 = 760;
    let mut vh: u32 = 860;
    let mut min_x: f32 = 0.0;
    let mut max_depth = usize::MAX;

    let mut nums = Vec::new();
    for a in args {
        if let Some(v) = a.strip_prefix("--min-x=") {
            min_x = v.parse().expect("--min-x 需要数字");
        } else if let Some(v) = a.strip_prefix("--depth=") {
            max_depth = v.parse().expect("--depth 需要数字");
        } else if let Ok(v) = a.parse::<u32>() {
            nums.push(v);
        }
    }
    if nums.len() >= 1 {
        vw = nums[0];
    }
    if nums.len() >= 2 {
        vh = nums[1];
    }

    let html = std::fs::read_to_string(&html_path).expect("读 html 失败");
    let css = std::fs::read_to_string(&css_path).expect("读 css 失败");

    // 与 app.rs 一致: .content 里先放 <style>{词典CSS}</style>, 再放词条 div
    let doc_html = format!(
        r#"<!DOCTYPE html><html><head><meta charset="utf-8"></head><body>
<div class="root">
  <style>{SHELL_CSS}</style>
  <div class="toolbar"><input value="apple"></div>
  <div class="content">
    <style>{css}{OXFORD9_CSS}</style>
    <div>{html}</div>
  </div>
</div>
</body></html>"#
    );

    let cfg = DocumentConfig {
        viewport: Some(Viewport::new(vw, vh, 1.0, ColorScheme::Light)),
        ..Default::default()
    };
    let mut doc = HtmlDocument::from_html(&doc_html, cfg);
    doc.resolve(0.0);

    let mut out = std::io::stdout().lock();
    writeln!(out, "viewport {vw}x{vh}  min_x={min_x}").unwrap();
    writeln!(
        out,
        "{:>6} {:<6} {:>8} {:>8} {:>8} {:>8}  {}",
        "id", "depth", "x", "y", "w", "h", "node"
    )
    .unwrap();

    // 按 (y, x) 排序输出, 便于按视觉顺序读
    let mut rows: Vec<(f32, f32, String)> = Vec::new();
    for (id, node) in doc.tree().iter() {
        let l = &node.final_layout;
        if l.size.width <= 0.0 || l.size.height <= 0.0 {
            continue;
        }
        let x = l.location.x;
        let y = l.location.y;
        if x < min_x {
            continue;
        }
        let depth = depth_of(&doc, id);
        if depth > max_depth {
            continue;
        }
        let tag = match &node.data {
            blitz_dom::node::NodeData::Element(e) => e.name.local.to_string(),
            blitz_dom::node::NodeData::AnonymousBlock(_) => "<anon>".to_string(),
            blitz_dom::node::NodeData::Text(_) => "#text".to_string(),
            blitz_dom::node::NodeData::Comment => "#comment".to_string(),
            blitz_dom::node::NodeData::Document => "#document".to_string(),
        };
        let text = node.text_content();
        let text = text.trim();
        let text = if text.is_empty() {
            String::new()
        } else {
            let mut t: String = text.chars().take(40).collect();
            if text.chars().count() > 40 {
                t.push('…');
            }
            format!("  {:?}", t)
        };
        rows.push((
            y,
            x,
            format!(
                "{:>6} {:<6} {:>8.1} {:>8.1} {:>8.1} {:>8.1}  <{}>{}",
                id, depth, x, y, l.size.width, l.size.height, tag, text
            ),
        ));
    }
    rows.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap().then(a.1.partial_cmp(&b.1).unwrap()));
    for (_, _, line) in rows {
        writeln!(out, "{line}").unwrap();
    }
}

fn depth_of(doc: &HtmlDocument, id: usize) -> usize {
    let mut d = 0;
    let mut cur = id;
    while let Some(n) = doc.get_node(cur) {
        match n.parent {
            Some(p) => {
                d += 1;
                cur = p;
            }
            None => break,
        }
    }
    d
}
