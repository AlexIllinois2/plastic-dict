//! HTML DOM 工具箱 (基于 html5ever + markup5ever_rcdom, 替代 TS 版 dom.ts)
//!
//! 注意: html5ever 遵循 HTML5 树构建规则, 而原 TS 用的 htmlparser2 更宽松
//! (尊重任意标签的自闭合写法)。`fix_self_closing` 在解析前把非 void 元素的
//! `<foo ... />` 改写为 `<foo ...></foo>`, 以保持与 htmlparser2 一致的结构。

use html5ever::serialize::{SerializeOpts, TraversalScope};
use html5ever::{parse_document, serialize, tendril::TendrilSink};
use markup5ever_rcdom::{Handle, NodeData, RcDom, SerializableHandle};

pub type Dom = RcDom;

const VOID_TAGS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

fn is_void(tag: &str) -> bool {
    let t = tag.rsplit(':').next().unwrap_or(tag);
    VOID_TAGS.contains(&t)
}

fn utf8_len(b: u8) -> usize {
    if b < 0x80 {
        1
    } else if b < 0xE0 {
        2
    } else if b < 0xF0 {
        3
    } else {
        4
    }
}

/// 在 html 字符串中位置 i 处尝试解析一个开始标签, 返回 (标签名, '>' 的位置, 是否自闭合)
fn parse_start_tag(html: &str, i: usize) -> Option<(String, usize, bool)> {
    let bytes = html.as_bytes();
    if bytes.get(i)? != &b'<' {
        return None;
    }
    let mut j = i + 1;
    if !bytes.get(j).map(|b| b.is_ascii_alphabetic()).unwrap_or(false) {
        return None;
    }
    let name_start = j;
    while j < bytes.len() {
        let b = bytes[j];
        if b.is_ascii_alphanumeric() || b == b'-' || b == b':' || b == b'_' {
            j += 1;
        } else {
            break;
        }
    }
    let name = html[name_start..j].to_string();
    let mut self_closing = false;
    let mut quote: Option<u8> = None;
    while j < bytes.len() {
        let b = bytes[j];
        if let Some(q) = quote {
            if b == q {
                quote = None;
            }
        } else if b == b'"' || b == b'\'' {
            quote = Some(b);
        } else if b == b'>' {
            // 检查前一个非空白字符是否为 '/'
            let mut k = j;
            while k > i && (bytes[k - 1] as char).is_whitespace() {
                k -= 1;
            }
            if k > i + 1 && bytes[k - 1] == b'/' {
                self_closing = true;
            }
            return Some((name, j, self_closing));
        }
        j += 1;
    }
    None
}

/// 把非 void 元素的自闭合写法改写为显式闭合对 (模拟 htmlparser2)
pub fn fix_self_closing(html: &str) -> String {
    if !html.contains("/>") {
        return html.to_string();
    }
    let mut out = String::with_capacity(html.len() + 32);
    let bytes = html.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            if let Some((name, gt, self_closing)) = parse_start_tag(html, i) {
                if self_closing && !is_void(&name) {
                    let mut raw = html[i..gt].to_string();
                    while raw.ends_with('/') || raw.ends_with(char::is_whitespace) {
                        raw.pop();
                    }
                    if raw.is_empty() {
                        out.push_str(&html[i..=gt]);
                    } else {
                        out.push_str(&raw);
                        out.push('>');
                        out.push_str(&format!("</{name}>"));
                    }
                } else {
                    out.push_str(&html[i..=gt]);
                }
                i = gt + 1;
                continue;
            }
        }
        let l = utf8_len(bytes[i]);
        out.push_str(&html[i..(i + l).min(html.len())]);
        i += l;
    }
    out
}

/// 解析 HTML, 返回等效于"片段根"的节点
///
/// html5ever 的文档模式必定生成 `html > head + body` 结构, 并且会把出现在
/// body 之前的元数据元素 (`<style>`/`<title>` 等) 收进 `<head>`; 原 TS 版用
/// htmlparser2 的 `parseDocument`, 片段顶层元素直接挂在 Document 下 (既不会
/// 产生 html/head/body, 也不会重排元素)。为保持行为一致:
///   1. 把 `<head>` 里的子节点按文档序挪回 `<body>` 开头 (head 里的内容必然
///      原本就排在 body 内容之前, 挪回去即恢复原始顺序);
///   2. 把 `<body>` 从 `<html>` 上摘下来作为独立的片段根 —— 这样函数返回后
///      局部 document/html 被 drop 时不会波及 body 的子树。
///      注意: markup5ever_rcdom 的 `Node::drop` 会清空整个子树的 children
///      (用于打断 parent↔children 环), 不摘下来 body 的内容会被一起清空。
///   3. 于是 `child_elems(root)` 就是片段的顶层元素, `iter_all(root)` 也不会
///      多出 html/head 两个节点。
pub fn parse_html(html: &str) -> Option<Handle> {
    let fixed = fix_self_closing(html);
    let dom: Dom = parse_document(RcDom::default(), Default::default())
        .one(fixed.as_str());
    let doc = dom.document;

    let html_el = children(&doc)
        .into_iter()
        .find(|k| tag_of(k) == "html");
    let Some(html_el) = html_el else {
        // 没有 html 包装: document 由调用方持有, 整树保持存活
        return Some(doc);
    };
    let kids = children(&html_el);
    let head = kids.iter().find(|k| tag_of(k) == "head").cloned();
    let body = kids.iter().find(|k| tag_of(k) == "body").cloned();
    let Some(body) = body else {
        // 没有 body: 返回 html 元素 (同样由调用方持有)
        return Some(html_el);
    };
    if let Some(head) = head {
        let head_kids = children(&head);
        if !head_kids.is_empty() {
            let mut body_kids = body.children.borrow_mut();
            for (i, c) in head_kids.into_iter().enumerate() {
                c.parent.set(Some(std::rc::Rc::downgrade(&body)));
                body_kids.insert(i, c);
            }
        }
    }
    // 把 body 从 html 上摘下, 成为独立的片段根 (见上面注释第 2 条)
    let Some(parent) = take_parent(&body) else {
        return Some(body);
    };
    parent
        .children
        .borrow_mut()
        .retain(|c| !std::rc::Rc::ptr_eq(c, &body));
    Some(body)
}

/// 序列化节点为 HTML 字符串 (含节点自身的标签)
pub fn serialize_node(node: &Handle) -> String {
    let mut out: Vec<u8> = Vec::new();
    // 注意: SerializeOpts 默认的 traversal_scope 是 ChildrenOnly(只输出子节点),
    // 这里必须显式指定 IncludeNode, 否则元素自身的开/闭标签会丢掉
    let opts = SerializeOpts {
        traversal_scope: TraversalScope::IncludeNode,
        ..Default::default()
    };
    let handle: SerializableHandle = node.clone().into();
    let _ = serialize(&mut out, &handle, opts);
    String::from_utf8_lossy(&out).into_owned()
}

/// 序列化所有子节点并拼接
pub fn serialize_children(node: &Handle) -> String {
    node.children
        .borrow()
        .iter()
        .map(|c| serialize_node(c))
        .collect::<Vec<_>>()
        .join("")
}

/// 序列化整个 document: 若树构建器包了 html/body, 取 body 内容;
/// 否则直接序列化 document 的子节点 (对应 TS serializeDoc)
pub fn serialize_doc(doc: &Handle) -> String {
    for k in children(doc) {
        if tag_of(&k) == "html" {
            for c in children(&k) {
                if tag_of(&c) == "body" {
                    return serialize_children(&c);
                }
            }
            return serialize_children(&k);
        }
        if tag_of(&k) == "body" {
            return serialize_children(&k);
        }
    }
    serialize_children(doc)
}

pub fn is_elem(node: &Handle) -> bool {
    matches!(node.data, NodeData::Element { .. })
}

/// 标签名 (去掉命名空间前缀, 对应 TS tagOf)
pub fn tag_of(node: &Handle) -> String {
    match &node.data {
        NodeData::Element { name, .. } => {
            let local = name.local.as_ref();
            match local.rfind(':') {
                Some(i) => local[i + 1..].to_string(),
                None => local.to_string(),
            }
        }
        _ => String::new(),
    }
}

pub fn children(node: &Handle) -> Vec<Handle> {
    node.children.borrow().clone()
}

pub fn child_elems(node: &Handle) -> Vec<Handle> {
    node.children
        .borrow()
        .iter()
        .filter(|c| is_elem(c))
        .cloned()
        .collect()
}

/// 自身 + 所有后代元素 (对应 TS iterAll)
pub fn iter_all(node: &Handle) -> Vec<Handle> {
    let mut out = Vec::new();
    if is_elem(node) {
        out.push(node.clone());
    }
    for c in node.children.borrow().iter() {
        out.extend(iter_all(c));
    }
    out
}

pub fn text_content(node: &Handle) -> String {
    let mut out = String::new();
    fn visit(n: &Handle, out: &mut String) {
        match &n.data {
            NodeData::Text { contents } => out.push_str(&contents.borrow()),
            _ => {
                for c in n.children.borrow().iter() {
                    visit(c, out);
                }
            }
        }
    }
    visit(node, &mut out);
    out
}

pub fn attr_of(node: &Handle, name: &str) -> Option<String> {
    let NodeData::Element { attrs, .. } = &node.data else {
        return None;
    };
    attrs
        .borrow()
        .iter()
        .find(|a| a.name.local.as_ref() == name)
        .map(|a| a.value.as_ref().to_string())
}

pub fn class_list(node: &Handle) -> Vec<String> {
    attr_of(node, "class")
        .map(|c| c.split_whitespace().map(|s| s.to_string()).collect())
        .unwrap_or_default()
}

pub fn set_attr(node: &Handle, name: &str, value: &str) {
    use html5ever::{namespace_url, Attribute, LocalName, QualName};
    let NodeData::Element { attrs, .. } = &node.data else {
        return;
    };
    let mut attrs = attrs.borrow_mut();
    if let Some(a) = attrs.iter_mut().find(|a| a.name.local.as_ref() == name) {
        a.value = value.into();
        return;
    }
    let qn = QualName::new(
        None,
        // 注意: 必须用空命名空间 — HTML 解析器给属性挂的就是无命名空间
        // (元素才在 xhtml ns); 若挂 xhtml ns, 序列化器会编造
        // "unknown_namespace:" 前缀 (曾经踩坑: data-sound 被写成
        // unknown_namespace:data-sound)
        namespace_url!(""),
        LocalName::from(name),
    );
    attrs.push(Attribute {
        name: qn,
        value: value.into(),
    });
}

pub fn del_attr(node: &Handle, name: &str) {
    let NodeData::Element { attrs, .. } = &node.data else {
        return;
    };
    attrs.borrow_mut().retain(|a| a.name.local.as_ref() != name);
}

/// 只读地取父节点强引用, 取完立刻放回 (Cell<Option<Weak<Node>>> 非 Copy)
pub fn parent_of(node: &Handle) -> Option<Handle> {
    let p = node.parent.take();
    let up = p.as_ref().and_then(|w| w.upgrade());
    node.parent.set(p);
    up
}

/// 取出父节点强引用, 同时把 node.parent 清空 (Cell<Option<Weak<Node>>> 非 Copy, 必须 take/set)
fn take_parent(node: &Handle) -> Option<Handle> {
    node.parent.take().and_then(|w| w.upgrade())
}

/// 从父节点移除整个子树 (对应 TS dropTree)
pub fn drop_tree(node: &Handle) {
    let Some(parent) = take_parent(node) else {
        return;
    };
    let mut kids = parent.children.borrow_mut();
    if let Some(i) = kids.iter().position(|c| std::ptr::eq(&**c, &**node)) {
        kids.remove(i);
    }
}

/// 用子节点替换自身 (对应 TS dropTag)
pub fn drop_tag(node: &Handle) {
    let Some(parent) = take_parent(node) else {
        return;
    };
    let kids: Vec<Handle> = node.children.borrow().clone();
    let mut pk = parent.children.borrow_mut();
    if let Some(i) = pk.iter().position(|c| std::ptr::eq(&**c, &**node)) {
        pk.remove(i);
        for (off, c) in kids.iter().enumerate() {
            c.parent.set(Some(std::rc::Rc::downgrade(&parent)));
            pk.insert(i + off, c.clone());
        }
    }
    node.children.borrow_mut().clear();
}

/// 把子节点从父节点上摘下
pub fn detach(node: &Handle) {
    drop_tree(node);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_closing_custom_tags_fixed() {
        let out = fix_self_closing(r#"<top-g><phon a="1"/>text</top-g>"#);
        assert_eq!(out, r#"<top-g><phon a="1"></phon>text</top-g>"#);
    }

    #[test]
    fn void_tags_untouched() {
        let html = r#"<div><br/><img src="x.png"/></div>"#;
        assert_eq!(fix_self_closing(html), html);
    }

    #[test]
    fn parse_and_attrs() {
        let doc = parse_html(r#"<div class="a b"><span data-x="1">hi</span></div>"#).unwrap();
        // parse_html 返回 body 元素 (html5ever 文档模式会包 html/head/body)
        assert_eq!(tag_of(&doc), "body");
        let all = iter_all(&doc);
        assert_eq!(all.len(), 3); // body + div + span
        let div = &all[1];
        assert_eq!(tag_of(div), "div");
        assert_eq!(class_list(div), vec!["a", "b"]);
        let span = &all[2];
        assert_eq!(attr_of(span, "data-x").as_deref(), Some("1"));
        assert_eq!(text_content(span), "hi");
        set_attr(span, "data-y", "2");
        assert_eq!(attr_of(span, "data-y").as_deref(), Some("2"));
        del_attr(span, "data-x");
        assert_eq!(attr_of(span, "data-x"), None);
    }

    #[test]
    fn leading_style_stays_in_body() {
        // 垫 <body> 的效果: 开头的 style 不会被收进 head
        let doc = parse_html(r#"<style>.a{color:red}</style><p>x</p>"#).unwrap();
        let s = serialize_node(&doc);
        assert!(s.contains("<style>"), "{s}");
        assert!(s.contains("<p>x</p>"), "{s}");
        let tags: Vec<String> = iter_all(&doc).iter().map(tag_of).collect();
        assert_eq!(tags, vec!["body", "style", "p"]);
    }

    #[test]
    fn serialize_roundtrip() {
        let doc = parse_html(r#"<p title="a&amp;b">x &lt; y</p>"#).unwrap();
        let s = serialize_node(&doc);
        assert!(s.contains("title=\"a&b\"") || s.contains("title=\"a&amp;b\""));
        assert!(s.contains("x &lt; y"));
    }

    /// set_attr 回归: 新建属性必须序列化成干净的名字。
    /// 曾经给属性挂了 xhtml 命名空间, 序列化器编造出 unknown_namespace: 前缀。
    #[test]
    fn set_attr_serializes_clean_name() {
        let doc = parse_html(r#"<phon data-src-mp3="a.spx">/x/</phon>"#).unwrap();
        let all = iter_all(&doc);
        let phon = &all[1];
        set_attr(phon, "data-sound", "a.spx");
        let s = serialize_node(&doc);
        assert!(s.contains(r#"data-sound="a.spx""#), "属性名被污染: {s}");
        assert!(!s.contains("unknown_namespace"), "出现编造前缀: {s}");
        // 覆盖已有属性只改值, 不新增
        set_attr(phon, "data-sound", "b.spx");
        assert_eq!(attr_of(phon, "data-sound").as_deref(), Some("b.spx"));
    }

    #[test]
    fn drop_tree_and_tag() {
        let doc = parse_html("<div><b>bold</b>tail</div>").unwrap();
        let all = iter_all(&doc);
        let b = all.iter().find(|n| tag_of(n) == "b").unwrap().clone();
        drop_tag(&b);
        let s = serialize_node(&doc);
        assert!(!s.contains("<b>"));
        assert!(s.contains("bold"));
        assert!(s.contains("tail"));
    }

    #[test]
    fn tag_of_strips_prefix() {
        let doc = parse_html("<xhtml:div>hi</xhtml:div>").unwrap();
        let all = iter_all(&doc);
        assert!(all.iter().any(|n| tag_of(n) == "div"));
    }

    #[test]
    fn utf8_safe_scan() {
        let out = fix_self_closing("<p>中文<a href=\"x\" b='/'/>尾</p>");
        assert!(out.contains("中文"));
        assert!(out.contains("</a>"));
    }
}
