//! 牛津词条 HTML → CLI 纯文本渲染 (移植自 TS 版 src/render.ts)

use crate::dom::*;
use markup5ever_rcdom::{Handle, NodeData};
use std::collections::HashSet;

const NOISE: &[&str] = &[
    "script", "style", "link", "head", "symbol", "xsymb", "img", "hkey",
    "topic", "ftindex", "fthzmark", "fthzindex", "xhtml", "sdsymb",
    "audio-wr", "audio", "audio-gbs-liju", "audio-uss-liju",
    "audio-brs-liju", "audio-ams-liju", "un",
];

const RENDER_NOISE_EXTRA: &[&str] = &[
    "xr-g", "xr-gs", "cf-blk", "cf", "syn-g-blk", "syn-g", "syn-gs",
    "lb-g", "lb", "lmb", "symbol", "un", "unx-g",
];

const SKIP_TAGS: &[&str] = &[
    // phon-blk 只包一层 "/音标/" 斜杠: 里面的 phon 已单独提取到"音标"行,
    // 不跳过 phon-blk 就会在正文里留下两行空的 "//"
    "h", "phon", "phon-blk", "pos", "brelabel", "namelabel", "pron-g",
    "vpform", "infl", "v-g",
];

/// RESET 表: 进入这些标签时重置编号状态
fn reset_flag(tag: &str) -> Option<bool> {
    match tag {
        "pos-g" => Some(true),
        "pv-blk" | "pv-g-blk" | "idm-blk" | "idm-gs-blk" | "subentry-g" | "boxblock" => Some(false),
        _ => None,
    }
}

const POS_CN: &[(&str, &str)] = &[
    ("verb", "动词"), ("noun", "名词"), ("adjective", "形容词"), ("adverb", "副词"),
    ("exclamation", "感叹词"), ("preposition", "介词"), ("conjunction", "连词"),
    ("pronoun", "代词"), ("determiner", "限定词"), ("number", "数词"),
    ("modal verb", "情态动词"), ("auxiliary verb", "助动词"),
    ("v.", "动词"), ("n.", "名词"), ("adj.", "形容词"), ("adv.", "副词"),
    ("excl.", "感叹词"), ("prep.", "介词"), ("conj.", "连词"), ("pron.", "代词"),
    ("det.", "限定词"), ("num.", "数词"), ("modal v.", "情态动词"),
    ("aux. v.", "助动词"),
];

const INFL_CN: &[(&str, &str)] = &[
    ("past participle", "过去分词"), ("past simple", "过去式"),
    ("-ing form", "现在分词"), ("present simple - he", "三单"),
    ("present simple", "原形"), ("plural", "复数"), ("third person", "三单"),
];

fn norm_phon(p: &str) -> String {
    let p = p.trim();
    if p.is_empty() {
        return String::new();
    }
    if !p.starts_with('/') {
        let inner = p.trim_matches('/');
        format!("/{inner}/")
    } else {
        p.to_string()
    }
}

fn infl_cn(name: &str) -> String {
    let low = name.to_lowercase();
    for (k, v) in INFL_CN {
        if low.contains(k) {
            return v.to_string();
        }
    }
    name.to_string()
}

fn pos_cn(p: &str) -> String {
    for (k, v) in POS_CN {
        if *k == p {
            return v.to_string();
        }
    }
    p.to_string()
}

fn has_noise(tag: &str) -> bool {
    NOISE.contains(&tag) || RENDER_NOISE_EXTRA.contains(&tag)
}

struct Renderer {
    out_lines: Vec<String>,
    ind: usize,
    chunks: Vec<String>,
    chn: Vec<String>,
    sn: u64,
    num: bool,
    top: u64,
    sections: HashSet<String>,
    bold: bool,
}

impl Renderer {
    fn b(&self, t: &str) -> String {
        if self.bold {
            format!("\x1b[1m{t}\x1b[0m")
        } else {
            t.to_string()
        }
    }

    fn add(&mut self, s: &str) {
        self.chunks.push(s.to_string());
    }

    fn flush(&mut self) {
        let text = self
            .chunks
            .join("")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let mut t2 = text.clone();
        if !self.chn.is_empty() {
            let cn = self.chn.join("；");
            t2 = if text.is_empty() {
                cn
            } else {
                format!("{cn} {text}")
            };
        }
        if !t2.is_empty() {
            self.out_lines.push(format!("{}{}", " ".repeat(self.ind), t2));
        }
        self.chunks.clear();
        self.chn.clear();
    }

    fn blank(&mut self) {
        if !self.out_lines.is_empty() && self.out_lines.last().map(|s| !s.is_empty()).unwrap_or(false) {
            self.out_lines.push(String::new());
        }
    }

    fn walk(&mut self, el: &Handle) {
        let tag = tag_of(el);
        if has_noise(&tag) {
            return;
        }
        let cls = class_list(el);
        if cls.iter().any(|c| c.contains("slash")) {
            return;
        }
        if tag == "top-g" {
            self.top += 1;
            if self.top > 1 {
                self.flush();
                self.blank();
            }
            for c in child_elems(el) {
                self.walk(&c);
            }
            self.flush();
            return;
        }
        if SKIP_TAGS.contains(&tag.as_str()) {
            return;
        }
        if tag == "br" {
            self.flush();
            return;
        }
        if let Some(num) = reset_flag(&tag) {
            self.sn = 0;
            self.num = num;
        }
        match tag.as_str() {
            "pos-g" => {
                let mut p = String::new();
                for c in iter_all(el) {
                    if tag_of(&c) == "pos" {
                        p = text_content(&c).trim().to_lowercase();
                        break;
                    }
                }
                self.flush();
                self.blank();
                if !p.is_empty() {
                    let label = pos_cn(&p);
                    self.out_lines.push(format!("{label} ({p})："));
                    self.sn = 0;
                    self.num = true;
                    self.ind = 4;
                }
            }
            "sn-g" => {
                self.flush();
                self.ind = 4;
                if self.num {
                    self.sn += 1;
                    self.add(&format!("{}. ", self.sn));
                }
            }
            "shcut" => {
                let t = text_content(el).trim().to_string();
                if !t.is_empty() {
                    self.flush();
                    self.blank();
                    self.out_lines.push(format!("  ◆ {}", self.b(&t)));
                    self.ind = 4;
                }
                return;
            }
            "x-g-blk" => {
                self.flush();
                self.ind = 6;
            }
            "pv-blk" | "pv-g-blk" => {
                self.flush();
                self.blank();
                if tag == "pv-blk" && !self.sections.contains("pv") {
                    self.out_lines.push("短语动词 (Phrasal Verbs)：".to_string());
                    self.sections.insert("pv".to_string());
                }
                self.ind = 6;
            }
            "idm-blk" | "idm-gs-blk" => {
                self.flush();
                self.blank();
                if !self.sections.contains("idm") {
                    self.out_lines.push("常用习语 (Idioms)：".to_string());
                    self.sections.insert("idm".to_string());
                }
                self.ind = 6;
            }
            "pv" | "idm" => {
                let t = text_content(el).trim().to_string();
                if !t.is_empty() {
                    self.flush();
                    self.blank();
                    self.out_lines.push(format!("  · {}", self.b(&t)));
                    self.ind = 6;
                }
                return;
            }
            "x-g" => {
                self.flush();
                let mut xs: Vec<String> = Vec::new();
                let mut chns: Vec<String> = Vec::new();
                for sub in iter_all(el) {
                    let t2 = tag_of(&sub);
                    if t2 == "x" {
                        xs.push(text_content(&sub).trim().to_string());
                    } else if t2 == "chn" {
                        chns.push(text_content(&sub).trim().to_string());
                    }
                }
                let eng = xs
                    .iter()
                    .filter(|s| !s.is_empty())
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" ");
                let eng = if eng.is_empty() {
                    text_content(el).trim().to_string()
                } else {
                    eng
                };
                let cn = chns
                    .iter()
                    .filter(|s| !s.is_empty())
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("；");
                self.ind = 6;
                let line = if cn.is_empty() {
                    format!("· {eng}")
                } else {
                    format!("· {eng} （{cn}）")
                };
                self.add(&line);
                self.flush();
                return;
            }
            "chn" => {
                let t = text_content(el).trim().to_string();
                if !t.is_empty() {
                    self.chn.push(t);
                }
                return;
            }
            "gram" => {
                let t = text_content(el).trim().to_string();
                if !t.is_empty() {
                    self.add(&format!("({t}) "));
                }
                return;
            }
            "gram-blk" => {
                for c in child_elems(el) {
                    self.walk(&c);
                }
                return;
            }
            "li" => {
                self.flush();
                self.ind += 2;
                self.add("- ");
                for c in child_elems(el) {
                    self.walk(&c);
                }
                self.flush();
                self.ind -= 2;
                return;
            }
            _ => {}
        }
        for c in children(el) {
            match &c.data {
                NodeData::Text { contents } => self.add(&contents.borrow()),
                _ => {
                    if is_elem(&c) {
                        self.walk(&c);
                    }
                }
            }
        }
    }
}

/// 主渲染入口
pub fn render(html_text: &str, width: usize, bold: bool) -> String {
    let Some(root) = parse_html(html_text) else {
        return html_text.to_string();
    };

    // ---- 1. 提取词形变化表 ----
    let mut infl_rows: Vec<(String, String, String)> = Vec::new();
    for vp in iter_all(&root) {
        if tag_of(&vp) != "vpform" {
            continue;
        }
        let Some(p) = parent_of(&vp) else { continue };
        let form = text_content(&vp).trim().to_string();
        let mut hw = String::new();
        let mut ph = String::new();
        for el in iter_all(&p) {
            let t = tag_of(&el);
            if t == "h" && hw.is_empty() {
                hw = text_content(&el).trim().to_string();
            } else if t == "phon" && ph.is_empty() {
                ph = text_content(&el).trim().to_string();
            }
        }
        if !form.is_empty() && (!hw.is_empty() || !ph.is_empty()) {
            infl_rows.push((infl_cn(&form), hw, norm_phon(&ph)));
        }
        drop_tree(&p);
    }

    // ---- 2. 清噪音 ----
    for el in iter_all(&root) {
        let t = tag_of(&el);
        if has_noise(&t) {
            drop_tree(&el);
        } else if t == "div" && attr_of(&el, "class").map(|c| c.contains("cixing_tiaozhuan")).unwrap_or(false) {
            drop_tree(&el);
        }
    }
    for tag in ["audio-gb", "audio-us", "pron-g", "audio"] {
        for el in iter_all(&root) {
            if tag_of(&el) == tag {
                drop_tag(&el);
            }
        }
    }

    // ---- 3. 词头 + 音标 ----
    let mut word = String::new();
    let mut prons: Vec<(String, String)> = Vec::new();
    for topg in iter_all(&root) {
        if tag_of(&topg) != "top-g" {
            continue;
        }
        let mut label: Option<String> = None;
        for el in iter_all(&topg) {
            let t = tag_of(&el);
            let cls = class_list(&el);
            if t == "h" && word.is_empty() {
                word = text_content(&el).trim().to_string();
            } else if t == "brelabel" || cls.iter().any(|c| c == "bre") {
                label = Some("BrE".to_string());
            } else if t == "namelabel" || cls.iter().any(|c| c == "name") {
                label = Some("NAmE".to_string());
            } else if t == "phon" && label.is_some() {
                let ph = norm_phon(text_content(&el).trim());
                if !ph.is_empty()
                    && !prons.iter().any(|(l, p)| *l == label.clone().unwrap() && *p == ph)
                {
                    prons.push((label.clone().unwrap(), ph));
                }
            }
        }
        break;
    }

    let mut head: Vec<String> = Vec::new();
    if !word.is_empty() {
        head.push(format!("单词：{word}"));
    }
    if !prons.is_empty() {
        head.push(format!(
            "音标：{}",
            prons
                .iter()
                .map(|(lab, ph)| format!("{lab} {ph}"))
                .collect::<Vec<_>>()
                .join(" ｜ ")
        ));
    }
    if !infl_rows.is_empty() {
        head.push("词形变化：".to_string());
        for (name, w, ph) in &infl_rows {
            head.push(format!(
                "  {name}：{w}{}",
                if ph.is_empty() { String::new() } else { format!(" {ph}") }
            ));
        }
    }

    // ---- 4. 正文行走 ----
    let mut r = Renderer {
        out_lines: Vec::new(),
        ind: 4,
        chunks: Vec::new(),
        chn: Vec::new(),
        sn: 0,
        num: true,
        top: 0,
        sections: HashSet::new(),
        bold,
    };
    for c in child_elems(&root) {
        r.walk(&c);
    }
    r.flush();

    // ---- 5. 组装 + 折行 ----
    let mut parts = head.clone();
    if !head.is_empty() && !r.out_lines.is_empty() {
        parts.push(String::new());
    }
    parts.extend(r.out_lines);

    let mut out: Vec<String> = Vec::new();
    for line in parts {
        let ind_len = line.len() - line.trim_start_matches(' ').len();
        let ind = &line[..ind_len];
        if line.chars().count() <= width {
            out.push(line);
            continue;
        }
        let words: Vec<&str> = line.split(' ').collect();
        let mut buf = String::new();
        for w in words {
            let cand_len = if buf.is_empty() {
                w.chars().count()
            } else {
                buf.chars().count() + 1 + w.chars().count()
            };
            if !buf.is_empty() && cand_len > width {
                out.push(buf.clone());
                buf = format!("{ind}    {w}");
            } else if buf.is_empty() {
                buf = format!("{ind}{w}");
            } else {
                buf = format!("{buf} {w}");
            }
        }
        if !buf.is_empty() {
            out.push(buf);
        }
    }
    out.join("\n")
}

/// 去掉重复的 aunbox (对应 TS stripDup)
pub fn strip_dup(html: &str) -> String {
    let Some(root) = parse_html(html) else {
        return html.to_string();
    };
    let mut removed = 0;
    for el in iter_all(&root) {
        if tag_of(&el) != "aunbox" {
            continue;
        }
        let Some(p) = parent_of(&el) else { continue };
        let self_text = text_content(&el);
        let twin = child_elems(&p).iter().any(|s| {
            !std::rc::Rc::ptr_eq(s, &el) && tag_of(s) == "unbox" && text_content(s) == self_text
        });
        if twin {
            drop_tree(&el);
            removed += 1;
        }
    }
    if removed > 0 {
        serialize_doc(&root)
    } else {
        html.to_string()
    }
}

/// 词条的中文词头 (对应 TS headwordChn)
pub fn headword_chn(html: &str) -> String {
    let Some(root) = parse_html(html) else {
        return String::new();
    };
    for el in iter_all(&root) {
        if tag_of(&el) != "chn" {
            continue;
        }
        let mut p = parent_of(&el);
        let mut inside = false;
        while let Some(cur) = p {
            let t = tag_of(&cur);
            if t == "unbox" || t == "aunbox" {
                inside = true;
                break;
            }
            p = parent_of(&cur);
        }
        if inside {
            continue;
        }
        let t = text_content(&el).trim().to_string();
        if !t.is_empty() {
            return t;
        }
    }
    String::new()
}

/// 供 GUI 工具栏: 提取 (label, phon) 音标列表 (对应 TS render.ts 步骤 3 的逻辑)
pub fn extract_prons_from_html(html: &str) -> Vec<(String, String)> {
    let Some(root) = parse_html(html) else {
        return Vec::new();
    };
    let mut prons: Vec<(String, String)> = Vec::new();
    for topg in iter_all(&root) {
        if tag_of(&topg) != "top-g" {
            continue;
        }
        let mut label: Option<String> = None;
        for el in iter_all(&topg) {
            let t = tag_of(&el);
            let cls = class_list(&el);
            if t == "brelabel" || cls.iter().any(|c| c == "bre") {
                label = Some("BrE".to_string());
            } else if t == "namelabel" || cls.iter().any(|c| c == "name") {
                label = Some("NAmE".to_string());
            } else if t == "phon" && label.is_some() {
                let ph = norm_phon(text_content(&el).trim());
                if !ph.is_empty()
                    && !prons.iter().any(|(l, p)| *l == label.clone().unwrap() && *p == ph)
                {
                    prons.push((label.clone().unwrap(), ph));
                }
            }
        }
        break;
    }
    prons
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<top-g><h>apple</h><brelabel>BrE</brelabel><phon>/ˈæp.əl/</phon><namelabel>NAmE</namelabel><phon>/ˈæp.əl/</phon></top-g>
<pos-g><pos>noun</pos></pos-g>
<sn-g><x-g><x>a round fruit</x><chn>苹果</chn></x-g></sn-g>
<v-g><vpform>past simple</vpform><h>appled</h><phon>/ˈæp.əld/</phon></v-g>"#;

    #[test]
    fn renders_head_and_pron() {
        let out = render(SAMPLE, 76, false);
        assert!(out.contains("单词：apple"), "{out}");
        assert!(out.contains("音标：BrE /ˈæp.əl/ ｜ NAmE /ˈæp.əl/"), "{out}");
        assert!(out.contains("名词 (noun)："), "{out}");
        // 义项编号与释义各占一行 (x-g 分支先 flush, 与 TS 版行为一致)
        assert!(out.contains("1."), "{out}");
        assert!(out.contains("· a round fruit （苹果）"), "{out}");
        assert!(out.contains("词形变化："), "{out}");
        assert!(out.contains("过去式：appled /ˈæp.əld/"), "{out}");
        // brelabel/namelabel 在 SKIP_TAGS 里, 标签文本不应泄漏进正文
        assert!(!out.contains("BrENAmE"), "{out}");
        // vpform 行不应出现在正文
        assert!(!out.contains("vpform"), "{out}");
    }

    #[test]
    fn strip_dup_removes_twin() {
        let html = "<div><unbox>same</unbox><aunbox>same</aunbox></div>";
        let out = strip_dup(html);
        assert_eq!(out.matches("same").count(), 1, "{out}");
    }

    #[test]
    fn headword_extraction() {
        assert_eq!(headword_chn("<div><chn>苹果</chn></div>"), "苹果");
        assert_eq!(headword_chn("<unbox><chn>skip</chn></unbox>"), "");
    }

    #[test]
    fn prons_for_toolbar() {
        let prons = extract_prons_from_html(SAMPLE);
        assert_eq!(prons.len(), 2);
        assert_eq!(prons[0].0, "BrE");
        assert_eq!(prons[1].0, "NAmE");
    }

    #[test]
    fn wrapping_respects_width() {
        let html = "<div>aaaaaaaaaa bbbbbbbbbb cccccccccc dddddddddd eeeeeeeeee ffffffffff</div>";
        let out = render(html, 20, false);
        for line in out.lines() {
            assert!(line.chars().count() <= 24, "line too long: {line}");
        }
    }
}
