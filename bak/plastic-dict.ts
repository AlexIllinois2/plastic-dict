#!/usr/bin/env bun
/**
 * plastic-dict — 单文件整合版（Bun + TypeScript 重写版）
 *
 * 原 Python 版（pywebview + PySide6/FreeSimpleGUI + mdict-mquery + lxml）的移植。
 * 去掉了 Qt/FreeSimpleGUI 原生窗口与 GTK 层钩子，只保留系统 WebKitGTK 窗口
 * （webview-bun → 系统 libwebkitgtk-6.0 / GTK4）+ CLI 两条路径。
 *
 * 用法:
 *   ./plastic-dict.ts "<content>"     自动: 单词→WebView 本地词典; 短语/句子→WebView 在线词典
 *   ./plastic-dict.ts -t "<content>"  命令行查词/翻译 (短语时弹 WebView 在线词典窗口)
 *   ./plastic-dict.ts -w "<content>"  强制 WebView (与默认一致, 保留参数兼容)
 *   ./plastic-dict.ts --purge-cache   清空词典索引磁盘缓存
 *
 * 快捷键: bash -lc 'bun ~/.local/app/plastic-dict/plastic-dict.ts "$(/usr/bin/wl-paste -n -p)"'
 *
 * 依赖:
 *   1) Bun >= 1.1
 *   2) 系统 WebKitGTK(GTK4) + glib —— webview-bun 的预编译库运行时直接链接:
 *      Debian 13 / Ubuntu 24.04+: sudo apt install libwebkitgtk-6.0-4
 *      Fedora 39+:                sudo dnf install webkitgtk6.0
 *      CJK 字体不装中文会变豆腐块: fonts-noto-cjk / google-noto-sans-cjk-ttc-fonts
 *   3) npm 依赖(见 package.json): webview-bun / js-mdict / htmlparser2 /
 *      domhandler / dom-serializer —— bun install 即可
 *   4) 音频(可选): mpv / ffplay / cvlc / paplay / aplay 任一即可
 *   5) 离线词典: https://github.com/yanyingwang/goldendict 的百度网盘
 *      dicts/Oxford9 移到 ~/.local/share/goldendict/Oxford9
 *      (可用环境变量 GOLDENDICT_DIR 覆盖)
 *
 * 配置文件 ~/.config/plastic-dict/config.json (自动创建, 可手工编辑):
 *   zoom      缩放(0.5~3.0, Ctrl+=/-/0 自动保存)
 *   width     窗口宽(320~7680, 默认 760)
 *   height    窗口高(300~4320, 默认 860)
 *   maximized 窗口最大化(true/false)
 *
 * 说明:
 *   - 词典索引磁盘缓存: ~/.cache/plastic-dict/index/ (按文件大小+mtime 失效,
 *     首次启动构建, 之后 CLI/GUI 冷启动无需重新解析全量词条)
 *   - 原版 dicres:// 协议改为内置 127.0.0.1 随机端口本地 HTTP 服务
 *   - DIC_DUMP=1 时把最近一次词条 HTML/CSS 存到 /tmp/plastic-dict/last.html|css
 *   - 崩溃日志: /tmp/plastic-dict/crash.log
 */

import { MDX, MDD } from "js-mdict";
import { parseDocument } from "htmlparser2";
import { Element, Text, Document, type AnyNode } from "domhandler";
import * as DomSerializer from "dom-serializer";
import * as fs from "node:fs";
import * as path from "node:path";
import * as os from "node:os";
import * as crypto from "node:crypto";

// ============================================================
// 基础路径 / 常量（原 dictapi.py + cfg patch）
// ============================================================
function expandUser(p: string): string {
  if (p === "~") return os.homedir();
  if (p.startsWith("~/")) return path.join(os.homedir(), p.slice(2));
  return p;
}

const DICT_DIR = expandUser(
  process.env.GOLDENDICT_DIR || "~/.local/share/goldendict/Oxford9",
);
const AUDIO_DIR = "/tmp/plastic-dict/audio";
const INDEX_CACHE_DIR = path.join(os.homedir(), ".cache", "plastic-dict", "index");
const CFG_PATH = path.join(os.homedir(), ".config", "plastic-dict", "config.json");
const DICT_NAME = "plastic-dict";

function logErr(...a: unknown[]) {
  console.error("[plastic-dict]", ...a);
}

// ============================================================
// 渲染层（原 render.py）
// ============================================================

// 噪音标签: 直接丢弃整个子树
const NOISE = new Set([
  "script", "style", "link", "head", "symbol", "xsymb", "img", "hkey",
  "topic", "ftindex", "fthzmark", "fthzindex", "xhtml", "sdsymb",
  "audio-wr", "audio", "audio-gbs-liju", "audio-uss-liju",
  "audio-brs-liju", "audio-ams-liju", "un",
]);

// 在 NOISE 基础上追加要丢弃的交叉引用/同义词噪音
// (源脚本定义了两次, 此处取最终生效的集合)
const RENDER_NOISE_EXTRA = new Set([
  "xr-g", "xr-gs", "cf-blk", "cf", "syn-g-blk", "syn-g", "syn-gs",
  "lb-g", "lb", "lmb", // language bank 语料库
  "symbol", "un", "unx-g",
]);

// 正文行走时直接跳过的标签(内容已单独提取或纯噪音)
const SKIP_TAGS = new Set([
  "h", "phon", "pos", "brelabel", "namelabel", "pron-g",
  "vpform", "infl", "v-g",
]);

const RESET: Record<string, boolean> = {
  "pos-g": true,
  "pv-blk": false,
  "pv-g-blk": false,
  "idm-blk": false,
  "idm-gs-blk": false,
  "subentry-g": false,
  boxblock: false,
};

const POS_CN: Record<string, string> = {
  verb: "动词", noun: "名词", adjective: "形容词", adverb: "副词",
  exclamation: "感叹词", preposition: "介词", conjunction: "连词",
  pronoun: "代词", determiner: "限定词", number: "数词",
  "modal verb": "情态动词", "auxiliary verb": "助动词",
  "v.": "动词", "n.": "名词", "adj.": "形容词", "adv.": "副词",
  "excl.": "感叹词", "prep.": "介词", "conj.": "连词", "pron.": "代词",
  "det.": "限定词", "num.": "数词", "modal v.": "情态动词",
  "aux. v.": "助动词",
};

const POS: Record<string, string> = {};

const INFL_CN: Array<[string, string]> = [
  ["past participle", "过去分词"],
  ["past simple", "过去式"],
  ["-ing form", "现在分词"],
  ["present simple - he", "三单"],
  ["present simple", "原形"],
  ["plural", "复数"],
  ["third person", "三单"],
];

// ------------------------------------------------------------
// htmlparser2 DOM 小工具 (对应 lxml 用法)
// ------------------------------------------------------------
function isElem(n: AnyNode): n is Element {
  return n.type === "tag" || n.type === "script" || n.type === "style";
}

function tagOf(n: AnyNode): string {
  if (!isElem(n)) return "";
  const name = n.name;
  const i = name.indexOf(":");
  return i >= 0 ? name.slice(i + 1) : name;
}

/** 深度优先遍历全部元素 (含自身), 对应 lxml root.iter() */
function* iterAll(n: AnyNode): Generator<Element> {
  if (isElem(n)) yield n;
  const kids = (n as Element).children;
  if (kids) for (const c of kids) yield* iterAll(c);
}

/** 直接子元素 (对应 python 的 for c in el) */
function childElems(n: AnyNode): Element[] {
  const kids = (n as Element).children ?? [];
  return kids.filter(isElem) as Element[];
}

/** 子树内全部文本, 对应 lxml text_content()/itertext() */
function textContent(n: AnyNode | null | undefined): string {
  if (!n) return "";
  let out = "";
  const visit = (x: AnyNode): void => {
    if (x.type === "text") out += (x as Text).data;
    const kids = (x as Element).children;
    if (kids) for (const c of kids) visit(c);
  };
  visit(n);
  return out;
}

function attrOf(el: Element, name: string): string | undefined {
  if (!el.attribs) return undefined;
  return el.attribs[name];
}

function setAttr(el: Element, name: string, value: string): void {
  if (!el.attribs) el.attribs = {};
  el.attribs[name] = value;
}

function delAttr(el: Element, name: string): void {
  if (el.attribs) delete el.attribs[name];
}

/** 对应 lxml drop_tree(): 连同子树从父节点移除 */
function dropTree(el: Element): void {
  const p = el.parent as Element | Document | null;
  if (!p || !p.children) return;
  const i = p.children.indexOf(el);
  if (i >= 0) p.children.splice(i, 1);
}

/** 对应 lxml drop_tag(): 用子节点替换自身 */
function dropTag(el: Element): void {
  const p = el.parent as Element | Document | null;
  if (!p || !p.children) return;
  const i = p.children.indexOf(el);
  if (i < 0) return;
  const kids = el.children ?? [];
  p.children.splice(i, 1, ...kids);
  for (const c of kids) c.parent = p;
  el.children = [];
}

function parseHtml(html: string): Document | null {
  try {
    return parseDocument(html, { decodeEntities: true });
  } catch {
    return null;
  }
}

function serializeDoc(doc: Document): string {
  const kids = doc.children ?? [];
  return kids
    .map((k) => DomSerializer.render(k as Element, { decodeEntities: true }))
    .join("");
}

function _normPhon(p: string): string {
  /** 音标补斜杠: 这个包里斜杠是独立节点, phon 取出来是裸 IPA */
  p = (p || "").trim();
  if (p && !p.startsWith("/")) p = "/" + p.replace(/^\/+|\/+$/g, "") + "/";
  return p;
}

function _inflCn(name: string): string {
  const low = name.toLowerCase();
  for (const [k, v] of INFL_CN) if (low.includes(k)) return v;
  return name;
}

export function render(htmlText: string, width = 76, bold = true): string {
  const root = parseHtml(htmlText);
  if (!root) return htmlText;

  // ---- 1. 提取词形变化表: 以 vpform 为锚, 父节点即一条变形记录 ----
  const inflRows: Array<[string, string, string]> = [];
  for (const vp of [...iterAll(root)]) {
    if (tagOf(vp) !== "vpform") continue;
    const p = vp.parent as Element | null;
    if (!p) continue;
    const form = textContent(vp).trim();
    let hw = "", ph = "";
    for (const el of iterAll(p)) {
      const t = tagOf(el);
      if (t === "h" && !hw) hw = textContent(el).trim();
      else if (t === "phon" && !ph) ph = textContent(el).trim();
    }
    if (form && (hw || ph)) inflRows.push([_inflCn(form), hw, _normPhon(ph)]);
    dropTree(p);
  }

  // ---- 2. 清噪音 ----
  for (const el of [...iterAll(root)]) {
    const t = tagOf(el);
    if (NOISE.has(t) || RENDER_NOISE_EXTRA.has(t)) {
      dropTree(el);
    } else if (
      t === "div" &&
      (attrOf(el, "class") || "").includes("cixing_tiaozhuan")
    ) {
      dropTree(el);
    }
  }

  for (const tag of ["audio-gb", "audio-us", "pron-g", "audio"]) {
    for (const el of [...iterAll(root)]) {
      if (tagOf(el) === tag) dropTag(el);
    }
  }

  // ---- 3. 词头 + 音标 (只取第一个 top-g) ----
  let word = "", prons: Array<[string, string]> = [];
  for (const topg of iterAll(root)) {
    if (tagOf(topg) !== "top-g") continue;
    let label: string | null = null;
    for (const el of iterAll(topg)) {
      const t = tagOf(el);
      const cls = (attrOf(el, "class") || "").split(/\s+/);
      if (t === "h" && !word) word = textContent(el).trim();
      else if (t === "brelabel" || cls.includes("bre")) label = "BrE";
      else if (t === "namelabel" || cls.includes("name")) label = "NAmE";
      else if (t === "phon" && label) {
        const ph = _normPhon(textContent(el).trim());
        if (ph && !prons.some(([l, p]) => l === label && p === ph)) {
          prons.push([label, ph]);
        }
      }
    }
    break;
  }

  const head: string[] = [];
  if (word) head.push(`单词：${word}`);
  if (prons.length)
    head.push(
      "音标：" +
        prons.map(([lab, ph]) => `${lab} ${ph}`).join(" ｜ "),
    );
  if (inflRows.length) {
    head.push("词形变化：");
    for (const [name, w, ph] of inflRows)
      head.push(`  ${name}：${w}${ph ? " " + ph : ""}`);
  }

  // ---- 4. 正文行走 ----
  const outLines: string[] = [];
  const cur = { ind: 4, chunks: [] as string[], chn: [] as string[] };
  const st = { sn: 0, num: true, top: 0 };
  const sections = new Set<string>(); // 已打印过的栏目标题, 防重复

  const noise = new Set([...NOISE, ...RENDER_NOISE_EXTRA]);

  const B = (t: string): string => (bold ? `\x1b[1m${t}\x1b[0m` : t);
  const add = (s: string): void => {
    cur.chunks.push(s);
  };
  const flush = (): void => {
    const text = cur.chunks.join("").replace(/\s+/g, " ").trim();
    let t2 = text;
    if (cur.chn.length) {
      const cn = cur.chn.join("；");
      t2 = cn + (text ? " " + text : "");
    }
    if (t2) outLines.push(" ".repeat(cur.ind) + t2);
    cur.chunks = [];
    cur.chn = [];
  };
  const blank = (): void => {
    if (outLines.length && outLines[outLines.length - 1] !== "") outLines.push("");
  };

  const walk = (el: Element): void => {
    const tag = tagOf(el);
    if (noise.has(tag)) return;
    // 斜杠等纯符号节点(独立 span), 跳过
    const cls = (attrOf(el, "class") || "").split(/\s+/);
    if (cls.some((c) => c.includes("slash"))) return;
    if (tag === "top-g") {
      st.top += 1;
      if (st.top > 1) {
        flush();
        blank();
      }
      for (const c of childElems(el)) walk(c);
      flush();
      return;
    }
    if (SKIP_TAGS.has(tag)) return;
    if (tag === "br") {
      flush();
      return;
    }
    if (tag in RESET) {
      st.sn = 0;
      st.num = RESET[tag];
    }
    if (tag === "pos-g") {
      let p = "";
      for (const c of iterAll(el)) {
        if (tagOf(c) === "pos") {
          p = textContent(c).trim().toLowerCase();
          break;
        }
      }
      flush();
      blank();
      if (p) {
        const key = p in POS ? POS[p] : p;
        outLines.push(`${POS_CN[key] ?? key} (${key})：`);
        st.sn = 0;
        st.num = true;
        cur.ind = 4;
      }
      return;
    }
    if (tag === "sn-g") {
      flush();
      cur.ind = 4;
      if (st.num) {
        st.sn += 1;
        add(`${st.sn}. `);
      }
      // 注意: 不 return, 继续走子节点取释义/例句
    } else if (tag === "shcut") {
      // 主题小标题
      const t = textContent(el).trim();
      if (t) {
        flush();
        blank();
        outLines.push("  ◆ " + B(t));
        cur.ind = 4;
      }
      return;
    } else if (tag === "x-g-blk") {
      flush();
      cur.ind = 6;
    } else if (tag === "pv-blk" || tag === "pv-g-blk") {
      flush();
      blank();
      if (tag === "pv-blk" && !sections.has("pv")) {
        outLines.push("短语动词 (Phrasal Verbs)：");
        sections.add("pv");
      }
      cur.ind = 6;
      // 不 return, 继续走子节点
    } else if (tag === "idm-blk" || tag === "idm-gs-blk") {
      flush();
      blank();
      if (!sections.has("idm")) {
        outLines.push("常用习语 (Idioms)：");
        sections.add("idm");
      }
      cur.ind = 6;
      // 不 return
    } else if (tag === "pv" || tag === "idm") {
      // 短语/习语词条头
      const t = textContent(el).trim();
      if (t) {
        flush();
        blank();
        outLines.push("  · " + B(t));
        cur.ind = 6;
      }
      return; // 头已用 text_content 提取, 跳过自身子树
    } else if (tag === "x-g") {
      // 例句: 英文在前, 全译括号在后
      flush();
      const xs: string[] = [], chns: string[] = [];
      for (const sub of iterAll(el)) {
        const t2 = tagOf(sub);
        if (t2 === "x") xs.push(textContent(sub).trim());
        else if (t2 === "chn") chns.push(textContent(sub).trim());
      }
      const eng = xs.filter(Boolean).join(" ") || textContent(el).trim();
      const cn = chns.filter(Boolean).join("；");
      cur.ind = 6;
      add("· " + eng + (cn ? ` （${cn}）` : ""));
      flush();
      return;
    } else if (tag === "chn") {
      // 释义中文: 缓存, flush 时前置
      const t = textContent(el).trim();
      if (t) cur.chn.push(t);
      return;
    } else if (tag === "gram") {
      const t = textContent(el).trim();
      if (t) add(`(${t}) `);
      return;
    } else if (tag === "gram-blk") {
      for (const c of childElems(el)) walk(c);
      return;
    } else if (tag === "li") {
      flush();
      cur.ind += 2;
      add("- ");
      for (const c of childElems(el)) walk(c);
      flush();
      cur.ind -= 2;
      return;
    }

    // 文本节点与子节点: htmlparser2 里文本都是子节点,
    // 天然覆盖 lxml 的 el.text / c.tail 语义
    for (const c of el.children ?? []) {
      if (c.type === "text") add((c as Text).data);
      else if (isElem(c)) walk(c);
    }
  };

  for (const c of childElems(root)) walk(c);
  flush();

  // ---- 5. 组装 + 折行(保留缩进) ----
  const parts = [
    ...head,
    ...(head.length && outLines.length ? [""] : []),
    ...outLines,
  ];
  const out: string[] = [];
  for (const line of parts) {
    const ind = (line.match(/^( *)/) ?? ["", ""])[1];
    if (line.length <= width) {
      out.push(line);
      continue;
    }
    const words = line.split(" ");
    let buf = "";
    for (const w of words) {
      if (buf && buf.length + 1 + w.length > width) {
        out.push(buf);
        buf = ind + "    " + w;
      } else {
        buf = buf ? buf + " " + w : ind + w;
      }
    }
    if (buf) out.push(buf);
  }
  return out.join("\n");
}

function stripDup(html: string): string {
  /** OALD9 的 aunbox 是 unbox 的锚点副本(GoldenDict 靠 CSS 隐藏)。
   * 只删除确有相同文本 unbox 孪生的 aunbox, 孤立 aunbox 保留。 */
  const root = parseHtml(html);
  if (!root) return html;
  let removed = 0;
  for (const el of [...iterAll(root)]) {
    if (tagOf(el) !== "aunbox") continue;
    const p = el.parent as Element | null;
    if (!p) continue;
    const twin = childElems(p).some(
      (s) =>
        s !== el && tagOf(s) === "unbox" && textContent(s) === textContent(el),
    );
    if (twin) {
      dropTree(el);
      removed += 1;
    }
  }
  return removed ? serializeDoc(root) : html;
}

function headwordChn(html: string): string {
  /** 取第一个不在 unbox/aunbox 内的 chn 文本, 作为词头中文摘要 */
  const root = parseHtml(html);
  if (!root) return "";
  for (const el of iterAll(root)) {
    if (tagOf(el) !== "chn") continue;
    let p = el.parent as Element | null;
    let inside = false;
    while (p) {
      const t = tagOf(p);
      if (t === "unbox" || t === "aunbox") {
        inside = true;
        break;
      }
      p = p.parent as Element | null;
    }
    if (inside) continue;
    const t = textContent(el).trim();
    if (t) return t;
  }
  return "";
}

// ============================================================
// 多分卷 MDD 查询层（原 multimdd.py）
// ============================================================
function escapeRegExp(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

const _mddCache = new Map<string, any>();

function getMdd(mddFile: string): any {
  let inst = _mddCache.get(mddFile);
  if (!inst) {
    inst = loadMdictWithCache(mddFile, "mdd");
    _mddCache.set(mddFile, inst);
  }
  return inst;
}

class MultiMDD {
  vols: string[] = [];
  private mounted: string | null = null;
  private inst: any = null;

  constructor(mdxFile: string) {
    const stem = mdxFile.replace(/\.(mdx|MDX)$/, "");
    const dir = path.dirname(stem);
    const base = path.basename(stem);
    // <词干>.mdd 与 <词干>.N.mdd, 按 realpath 去重(兼容符号链接)
    const names = [base + ".mdd"];
    try {
      const re = new RegExp("^" + escapeRegExp(base) + "\\.\\d+\\.mdd$");
      for (const f of fs.readdirSync(dir).sort()) if (re.test(f)) names.push(f);
    } catch {
      /* ignore */
    }
    const seen = new Set<string>();
    for (const n of names) {
      const p = path.join(dir, n);
      try {
        if (!fs.statSync(p).isFile()) continue;
        const rp = fs.realpathSync(p);
        if (!seen.has(rp)) {
          seen.add(rp);
          this.vols.push(p);
        }
      } catch {
        /* ignore */
      }
    }
  }

  private mount(vol: string): any {
    if (this.mounted === vol) return this.inst;
    this.inst = getMdd(vol);
    this.mounted = vol;
    return this.inst;
  }

  lookup(ref: string, candidates: (r: string) => string[]): Uint8Array | null {
    // 第一轮: 精确候选, 逐卷尝试
    for (const vol of this.vols) {
      const inst = this.mount(vol);
      for (const cand of candidates(ref)) {
        try {
          const res = inst.locate(cand);
          if (res && res.definition) {
            const buf = Buffer.from(String(res.definition), "base64");
            if (buf.length) return new Uint8Array(buf);
          }
        } catch {
          continue;
        }
      }
    }
    // 第二轮: 后缀兜底, 每卷扫全量 key
    const suffix = "/" + String(ref).replace(/\\/g, "/").replace(/^\/+/, "");
    for (const vol of this.vols) {
      const inst = this.mount(vol);
      let keys: string[] = [];
      try {
        keys = (inst.keywordList ?? []).map((k: any) => String(k.keyText));
      } catch {
        continue;
      }
      for (const k of keys) {
        if (k.replace(/\\/g, "/").endsWith(suffix)) {
          try {
            const res = inst.locate(k);
            if (res && res.definition) {
              const buf = Buffer.from(String(res.definition), "base64");
              if (buf.length) return new Uint8Array(buf);
            }
          } catch {
            continue;
          }
        }
      }
    }
    return null;
  }

  /** 所有卷的全量 key (原 builder.get_mdd_keys(); 用于 basename 兜底) */
  allKeys(): string[] {
    const out: string[] = [];
    for (const vol of this.vols) {
      try {
        const inst = this.mount(vol);
        for (const k of inst.keywordList ?? []) out.push(String(k.keyText));
      } catch {
        /* ignore */
      }
    }
    return out;
  }
}

const _mddPool = new Map<string, MultiMDD>();

function multiMddLookup(
  mdx: string,
  ref: string,
  candidates: (r: string) => string[],
): Uint8Array | null {
  return getMultiMDD(mdx).lookup(ref, candidates);
}

function getMultiMDD(mdx: string): MultiMDD {
  let pool = _mddPool.get(mdx);
  if (!pool) {
    pool = new MultiMDD(mdx);
    _mddPool.set(mdx, pool);
  }
  return pool;
}

// ============================================================
// 词典门面（原 dictapi.py）+ 磁盘索引缓存
// ============================================================
const AUDIO_PATTERNS: RegExp[] = [
  /sound:\/\/([^"'\s>\\]+)/g,
  /data-src-mp3=["']([^"']+)/g,
  /href=["']([^"']*?\.mp3)/g,
];

const builders = new Map<string, any>();

function fileStamp(p: string): { size: number; mtimeMs: number } | null {
  try {
    const st = fs.statSync(p);
    return { size: st.size, mtimeMs: Math.floor(st.mtimeMs) };
  } catch {
    return null;
  }
}

function indexCachePath(file: string): string {
  const h = crypto
    .createHash("sha1")
    .update(path.resolve(file))
    .digest("hex");
  return path.join(INDEX_CACHE_DIR, `${path.basename(file)}.${h}.json`);
}

const CACHE_FORMAT = 1;

function readIndexCache(file: string): any | null {
  const stamp = fileStamp(file);
  if (!stamp) return null;
  try {
    const raw = JSON.parse(fs.readFileSync(indexCachePath(file), "utf8"));
    if (
      raw?.v !== CACHE_FORMAT ||
      raw?.size !== stamp.size ||
      raw?.mtimeMs !== stamp.mtimeMs ||
      !Array.isArray(raw?.keywordList) ||
      !Array.isArray(raw?.recordInfoList)
    ) {
      return null;
    }
    return raw;
  } catch {
    return null;
  }
}

function saveIndexCache(inst: any, file: string): void {
  const stamp = fileStamp(file);
  if (!stamp) return;
  try {
    fs.mkdirSync(INDEX_CACHE_DIR, { recursive: true });
    const meta = inst.meta ?? {};
    const data = {
      v: CACHE_FORMAT,
      lib: "js-mdict",
      size: stamp.size,
      mtimeMs: stamp.mtimeMs,
      meta: {
        ext: meta.ext,
        encoding: meta.encoding,
        numWidth: meta.numWidth,
        version: meta.version,
        encrypt: meta.encrypt ?? -1,
        numFmt: meta.numFmt ?? null,
      },
      header: inst.header ?? {},
      keyHeader: inst.keyHeader ?? {},
      recordHeader: inst.recordHeader ?? {},
      keywordList: inst.keywordList ?? [],
      keyInfoList: inst.keyInfoList ?? [],
      recordInfoList: inst.recordInfoList ?? [],
      recordBlockStartOffset: inst._recordBlockStartOffset ?? 0,
      keyBlockStartOffset: inst._keyBlockStartOffset ?? 0,
    };
    fs.writeFileSync(indexCachePath(file), JSON.stringify(data));
  } catch (e) {
    logErr("索引缓存写入失败(不影响功能):", (e as Error).message);
  }
}

function purgeIndexCache(file: string): void {
  try {
    fs.rmSync(indexCachePath(file), { force: true });
  } catch {
    /* ignore */
  }
}

function purgeAllCache(): number {
  let n = 0;
  try {
    for (const f of fs.readdirSync(INDEX_CACHE_DIR)) {
      try {
        fs.rmSync(path.join(INDEX_CACHE_DIR, f), { force: true });
        n += 1;
      } catch {
        /* ignore */
      }
    }
  } catch {
    /* ignore */
  }
  return n;
}

/** 轻量文件扫描器: 只需实现 js-mdict 内部用到的 readBuffer/close */
function makeScanner(file: string) {
  const fd = fs.openSync(file, "r");
  return {
    offset: 0,
    filepath: file,
    fd,
    close() {
      try {
        fs.closeSync(fd);
      } catch {
        /* ignore */
      }
    },
    readBuffer(offset: number | bigint, length: number): Uint8Array {
      const buf = new Uint8Array(Number(length));
      if (length > 0) {
        const n = fs.readSync(fd, buf, 0, Number(length), Number(offset));
        return n === buf.length ? buf : buf.subarray(0, n);
      }
      return buf;
    },
    readNumber(offset: number, length: number): DataView {
      const raw = this.readBuffer(offset, length);
      return new DataView(raw.buffer, raw.byteOffset, raw.byteLength);
    },
  };
}

/** 从磁盘缓存恢复 js-mdict 实例(跳过全量 key 解析), 失败返回 null */
function restoreMdict(file: string, kind: "mdx" | "mdd"): any | null {
  const c = readIndexCache(file);
  if (!c) return null;
  let inst: any = null;
  try {
    const proto = kind === "mdd" ? MDD.prototype : MDX.prototype;
    inst = Object.create(proto);
    const enc = String(c.meta?.encoding || "UTF-8");
    let decoder: TextDecoder;
    try {
      decoder = new TextDecoder(
        enc.toUpperCase() === "UTF-16" ? "utf-16le" : enc.toLowerCase(),
      );
    } catch {
      decoder = new TextDecoder("utf-8"); // 未知编码回退
    }
    inst.meta = {
      fname: file,
      passcode: undefined,
      ext: c.meta?.ext ?? kind,
      version: c.meta?.version ?? 2,
      numWidth: c.meta?.numWidth ?? 8,
      numFmt: c.meta?.numFmt ?? { begin: 0, end: 0, step: 0 },
      encoding: c.meta?.encoding ?? "UTF-8",
      decoder,
      encrypt: c.meta?.encrypt ?? -1,
    };
    inst.options = {
      passcode: undefined,
      debug: false,
      resort: true,
      isStripKey: true,
      isCaseSensitive: false,
      encryptType: -1,
    };
    inst.header = c.header ?? {};
    inst.keyHeader = c.keyHeader ?? {};
    inst.recordHeader = c.recordHeader ?? {};
    inst.keywordList = c.keywordList;
    inst.keyInfoList = c.keyInfoList ?? [];
    inst.recordInfoList = c.recordInfoList;
    inst._recordBlockStartOffset = c.recordBlockStartOffset ?? 0;
    inst._keyBlockStartOffset = c.keyBlockStartOffset ?? 0;
    inst._keyHeaderEndOffset = 0;
    inst._keyBlockInfoStartOffset = 0;
    inst._keyBlockInfoEndOffset = 0;
    inst._recordHeaderStartOffset = 0;
    inst._recordHeaderEndOffset = 0;
    inst._recordInfoStartOffset = 0;
    inst._recordInfoEndOffset = 0;
    inst._recordBlockEndOffset = 0;
    inst.scanner = makeScanner(file);
    // 冒烟验证: 用第一个 key 做一次真实读取, 确认缓存完整可用
    const first = inst.keywordList?.[0]?.keyText;
    if (typeof first === "string" && first) {
      if (kind === "mdd") inst.locate(first);
      else inst.lookup(first);
    } else if ((inst.keywordList?.length ?? 0) !== 0) {
      return null;
    }
    return inst;
  } catch {
    try {
      inst?.scanner?.close?.();
    } catch {
      /* ignore */
    }
    return null;
  }
}

function loadMdictWithCache(file: string, kind: "mdx" | "mdd"): any {
  const restored = restoreMdict(file, kind);
  if (restored) return restored;
  logErr(`  [构建] 索引缓存(一次性): ${path.basename(file)}`);
  const inst: any = kind === "mdd" ? new MDD(file) : new MDX(file);
  saveIndexCache(inst, file);
  return inst;
}

function getBuilder(mdx: string): any {
  let b = builders.get(mdx);
  if (!b) {
    try {
      b = loadMdictWithCache(mdx, "mdx");
    } catch (e) {
      // 缓存损坏 → 清理重建
      purgeIndexCache(mdx);
      logErr(
        `  [构建] 索引(一次性): ${path.basename(mdx)} (${(e as Error).message})`,
      );
      b = new MDX(mdx);
      saveIndexCache(b, mdx);
    }
    builders.set(mdx, b);
  }
  return b;
}

function toStr(x: unknown): string {
  if (typeof x === "string") return x;
  if (x instanceof Uint8Array || Buffer.isBuffer(x)) {
    const buf = Buffer.from(x as Uint8Array);
    return buf.toString("utf-8");
  }
  return String(x);
}

function candidates(ref: string): string[] {
  const r = ref.replace(/\\/g, "/").replace(/^\/+/, "");
  return ["/" + r, "\\" + r, r];
}

function saveAudioData(data: Uint8Array, ref: string): string {
  fs.mkdirSync(AUDIO_DIR, { recursive: true });
  const out = path.join(AUDIO_DIR, path.basename(ref.replace(/\\/g, "/")));
  const exists =
    fs.existsSync(out) && fs.statSync(out).size === data.byteLength;
  if (!exists) fs.writeFileSync(out, data);
  return out;
}

function listMdxFiles(): string[] {
  const out: string[] = [];
  const walkDir = (dir: string): void => {
    let entries: fs.Dirent[];
    try {
      entries = fs.readdirSync(dir, { withFileTypes: true });
    } catch {
      return;
    }
    for (const e of entries.sort((a, b) => a.name.localeCompare(b.name))) {
      const p = path.join(dir, e.name);
      if (e.isDirectory()) walkDir(p);
      else if (e.isFile() && e.name.toLowerCase().endsWith(".mdx")) out.push(p);
    }
  };
  walkDir(DICT_DIR);
  return out.sort();
}

function listCssFiles(dir: string): string[] {
  const out: string[] = [];
  const walkDir = (d: string): void => {
    let entries: fs.Dirent[];
    try {
      entries = fs.readdirSync(d, { withFileTypes: true });
    } catch {
      return;
    }
    for (const e of entries.sort((a, b) => a.name.localeCompare(b.name))) {
      const p = path.join(d, e.name);
      if (e.isDirectory()) walkDir(p);
      else if (e.isFile() && e.name.toLowerCase().endsWith(".css")) out.push(p);
    }
  };
  walkDir(dir);
  return out.sort();
}

export function lookup(word: string, bold = true): any {
  const mdxFiles = listMdxFiles();
  const r: any = {
    word,
    found: false,
    text: null,
    audio_refs: [] as string[],
    dict: null,
    _mdx: null,
    _html: null,
    css: "",
  };
  if (!mdxFiles.length) return r;
  for (const mdx of mdxFiles) {
    const builder = getBuilder(mdx);
    let def: string | null = null;
    try {
      const res = builder.lookup(word);
      def = res && res.definition ? String(res.definition) : null;
      if (!def) {
        const res2 = builder.lookup(word.toLowerCase()); // 大小写兜底
        def = res2 && res2.definition ? String(res2.definition) : null;
      }
    } catch {
      // 恢复的缓存可能损坏 → 删缓存重建一次
      purgeIndexCache(mdx);
      builders.delete(mdx);
      try {
        const fresh: any = new MDX(mdx);
        saveIndexCache(fresh, mdx);
        builders.set(mdx, fresh);
        const res = fresh.lookup(word);
        def = res && res.definition ? String(res.definition) : null;
      } catch {
        continue;
      }
    }
    if (!def) continue;
    const html = toStr(def).replace(/\x00+$/, "").replace(/\s+$/, "");
    r.found = true;
    r.dict = path.basename(mdx);
    r._mdx = mdx;
    r.text = render(stripDup(html), 76, bold);
    const cssParts: string[] = [];
    for (const cp of listCssFiles(path.dirname(mdx))) {
      try {
        cssParts.push(fs.readFileSync(cp, "utf8"));
      } catch {
        /* ignore */
      }
    }
    r.css = cssParts.join("\n\n");
    r._html = stripDup(html);
    let hw = headwordChn(html);
    if (hw) {
      hw = hw
        .replace(/（[^）]*）|\([^)]*\)/g, "")
        .replace(/^[ ；;，,]+/, "")
        .replace(/[ ；;，,]+$/, "");
      if (hw.length > 30) hw = hw.slice(0, 30) + "…";
    }
    if (hw) {
      const lines = String(r.text).split("\n");
      lines[0] = lines[0].replace(/\s+$/, "") + " 【" + hw + "】";
      r.text = lines.join("\n");
    }
    r.audio_refs = parseAudioRefs(html); // 只记引用, 不解压
    break; // 命中第一个词典即止
  }
  return r;
}

function parseAudioRefs(html: string): string[] {
  const seen = new Set<string>();
  const refs: string[] = [];
  for (const pat of AUDIO_PATTERNS) {
    pat.lastIndex = 0;
    let m: RegExpExecArray | null;
    while ((m = pat.exec(html))) {
      let ref = (m[1] || "").trim();
      if (ref.startsWith("sound://")) ref = ref.slice("sound://".length);
      if (!ref || seen.has(ref)) continue;
      const base = path.basename(ref);
      if (
        base.startsWith("_") ||
        (base.includes("__") &&
          ["gbs", "uss", "brs", "ams"].some((t) => base.includes(t)))
      ) {
        continue; // 例句音包未下载, 跳过
      }
      seen.add(ref);
      refs.push(ref);
    }
  }
  return refs;
}

export function extractAudio(mdx: string, ref: string): string | null {
  /** 点播放才解压单个 ref; 目录里只保留当前这一个。
   * 精确路径失败时, 按 basename 在所有 MDD 卷里兜底匹配。 */
  ref = String(ref).replace(/\\/g, "/").trim().replace(/^\/+/, "");
  let data = multiMddLookup(mdx, ref, candidates);
  if (!data) {
    const base = path.basename(ref).toLowerCase();
    let keys: string[] = [];
    try {
      keys = getMultiMDD(mdx).allKeys();
    } catch {
      keys = [];
    }
    for (const k of keys) {
      if (path.basename(k.replace(/\\/g, "/")).toLowerCase() === base) {
        data = multiMddLookup(mdx, k, candidates);
        if (data) break;
      }
    }
  }
  if (!data) return null;
  fs.mkdirSync(AUDIO_DIR, { recursive: true });
  const name = path.basename(ref.replace(/\\/g, "/"));
  const out = path.join(AUDIO_DIR, name);
  if (fs.existsSync(out) && fs.statSync(out).size === data.byteLength) {
    return out;
  }
  try {
    for (const f of fs.readdirSync(AUDIO_DIR)) {
      if (f !== name) {
        try {
          fs.rmSync(path.join(AUDIO_DIR, f), { force: true });
        } catch {
          /* ignore */
        }
      }
    }
  } catch {
    /* ignore */
  }
  fs.writeFileSync(out, data);
  return out;
}

export function getPlayable(word: string, variant: "gb" | "us"): string | null {
  /** 查词并解压指定发音变体("gb"英音/"us"美音), 返回 mp3 路径或 null
   * 不走 render(), 纯查索引, 开销极小 */
  for (const mdx of listMdxFiles()) {
    const b = getBuilder(mdx);
    let def: string | null = null;
    try {
      const res = b.lookup(word);
      def = res && res.definition ? String(res.definition) : null;
      if (!def) {
        const res2 = b.lookup(word.toLowerCase());
        def = res2 && res2.definition ? String(res2.definition) : null;
      }
    } catch {
      continue;
    }
    if (!def) continue;
    const html = toStr(def);
    for (const ref of parseAudioRefs(html)) {
      if (path.basename(ref).toLowerCase().includes(`_${variant}_`)) {
        return extractAudio(mdx, ref);
      }
    }
  }
  return null;
}

// ============================================================
// Bing 在线翻译（免 token 端点）
// ============================================================
const BING_URL = "https://edge.microsoft.com/translate/translatetext";
const EDGE_UA =
  "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 " +
  "(KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 Edg/131.0.0.0";

export async function translateText(
  text: string,
  targetLang = "zh-Hans",
): Promise<string> {
  if (!text || !text.trim()) return "";
  const url = `${BING_URL}?isEnterpriseClient=false&to=${targetLang}`;
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
    "User-Agent": EDGE_UA,
  };
  try {
    // body 是纯字符串数组 [text]，非对象；source 不传，服务端自动检测
    const resp = await fetch(url, {
      method: "POST",
      headers,
      body: JSON.stringify([text]),
      signal: AbortSignal.timeout(15000),
    });
    if (!resp.ok) return `网络请求失败: HTTP ${resp.status}`;
    const result = await resp.json();
    if (
      Array.isArray(result) &&
      result.length &&
      result[0] &&
      "translations" in result[0]
    ) {
      return String(result[0].translations[0].text ?? "").trim();
    }
    return "解析响应失败";
  } catch (e) {
    return `网络请求失败: ${(e as Error).message}`;
  }
}

// ============================================================
// GoldenDict 风格在线词典/翻译站
// 非单词输入时在同一 WebView 窗口内打开(像本地查词一样渲染);
// 模板里的 %GDWORD% 会被替换成 URL 编码后的查询文本, 按需增删即可
// ============================================================
const ONLINE_SITES = [
  "http://dict.youdao.com/w/eng/%GDWORD%",
  "https://www.bing.com/dict/search?q=%GDWORD%",
];

function onlineSiteUrls(text: string): string[] {
  /** 按模板生成完整 URL 列表 (%GDWORD% -> urlencode(text)) */
  const q = encodeURIComponent((text || "").trim());
  return ONLINE_SITES.filter((t) => t.includes("%GDWORD%")).map((t) =>
    t.replace("%GDWORD%", q),
  );
}

function onlineSiteNames(): string[] {
  /** ONLINE_SITES 的按钮短名, 未识别域名显示 host */
  const names: string[] = [];
  for (const t of ONLINE_SITES) {
    let host = "";
    try {
      host = new URL(t).hostname.toLowerCase();
    } catch {
      host = t.toLowerCase();
    }
    let name: string | null = null;
    for (const [k, v] of [
      ["youdao", "有道"],
      ["baidu", "百度翻译"],
      ["bing", "必应"],
      ["haici", "海词"],
      ["cambridge", "剑桥"],
      ["merriam", "韦氏"],
      ["collins", "柯林斯"],
      ["oxford", "牛津"],
    ] as const) {
      if (host.includes(k)) {
        name = v;
        break;
      }
    }
    names.push(name ?? host);
  }
  return names;
}

function e2(s: string, quote = false): string {
  let out = s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
  if (quote) out = out.replace(/"/g, "&quot;").replace(/'/g, "&#39;");
  return out;
}

function onlineStackedHtml(query: string): string {
  /** GoldenDict 式拼接页: 所有在线站点以 iframe 上下排列。
   * (dic scroll patch) 跨域 iframe 滚到边不会接力滚动外层,
   * 提供右侧悬浮 ▲▼⇈⇟ / Ctrl+↓↑ 等滚动外层的方式。 */
  const names = onlineSiteNames();
  const urls = onlineSiteUrls(query || "");
  const rows: string[] = [];
  urls.forEach((u, i) => {
    const nm = i < names.length ? names[i] : u;
    rows.push(
      '<div class="site"><div class="hd">' +
        `<iframe id="f${i}" src="${e2(u, true)}" referrerpolicy="no-referrer"></iframe></div>`,
    );
  });
  return (
    '<div id="stack-nav">' +
    '<button onclick="window.scrollTo(0,0)" title="回顶部">⇈</button>' +
    '<button onclick="window.scrollBy(0,-Math.round(' +
    'window.innerHeight*0.9))" title="上一站/Ctrl+↑">▲</button>' +
    '<button onclick="window.scrollBy(0,Math.round(' +
    'window.innerHeight*0.9))" title="下一站/Ctrl+↓">▼</button>' +
    '<button onclick="window.scrollTo(0,document.body.scrollHeight)"' +
    ' title="到底部">⇟</button>' +
    "</div>" +
    "<style>" +
    "#stack-nav{position:fixed;right:10px;top:50%;" +
    "transform:translateY(-50%);display:flex;flex-direction:column;" +
    "gap:6px;z-index:2147483000;}" +
    "#stack-nav button{width:38px;height:38px;font-size:16px;" +
    "border:1px solid #c5cfdd;border-radius:8px;" +
    "background:rgba(255,255,255,.95);cursor:pointer;" +
    "box-shadow:0 1px 4px rgba(0,0,0,.18);}" +
    "#stack-nav button:active{background:#e8eef7;}" +
    ".site{margin:0 0 10px 0;}" +
    ".site .hd{display:flex;gap:8px;align-items:center;padding:8px 14px;" +
    "background:#eef3fa;border-top:1px solid #dbe5f0;" +
    "border-bottom:1px solid #dbe5f0;font-size:14px;}" +
    ".site .hd .u{color:#888;font-size:12px;overflow:hidden;" +
    "text-overflow:ellipsis;white-space:nowrap;flex:1;}" +
    ".site .hd button{padding:2px 8px;font-size:12px;cursor:pointer;}" +
    ".site iframe{width:100%;height:82vh;border:0;display:block;" +
    "background:#fff;}" +
    "</style>" +
    rows.join("")
  );
}

// ============================================================
// dic 配置持久化(缩放/窗口大小/最大化)
// ============================================================
interface DicCfg {
  zoom?: number;
  width?: number;
  height?: number;
  maximized?: boolean;
  [k: string]: unknown;
}

function loadCfg(): DicCfg {
  try {
    const data = JSON.parse(fs.readFileSync(CFG_PATH, "utf8"));
    return data && typeof data === "object" ? data : {};
  } catch {
    return {};
  }
}

function saveCfgMerged(extra: DicCfg): void {
  const cfg = loadCfg();
  for (const [k, v] of Object.entries(extra)) {
    if (v !== null && v !== undefined) cfg[k] = v;
  }
  try {
    fs.mkdirSync(path.dirname(CFG_PATH), { recursive: true });
    fs.writeFileSync(CFG_PATH, JSON.stringify(cfg, null, 1));
  } catch {
    /* ignore */
  }
}

function cfgInt(
  cfg: DicCfg,
  key: string,
  dft: number,
  lo: number,
  hi: number,
): number {
  try {
    return Math.max(lo, Math.min(hi, Math.round(Number(cfg[key] ?? dft))));
  } catch {
    return dft;
  }
}

// ============================================================
// (dic img patch) 本地资源服务: 原版 dicres:// 协议改为本地 HTTP
// ============================================================
const RES_SKIP = /^(https?:|data:|file:|about:|\/\/)/i;
const CSS_URL_RE = /url\(\s*(['"]?)([^'")]+)\1\s*\)/g;

function cssLocalize(cssText: string, resBase: string): string {
  /** 词典 CSS 里的 url(/x.png) → url(${resBase}/res/x.png) */
  return (cssText || "").replace(CSS_URL_RE, (m0, _q, u: string) => {
    const u2 = (u || "").trim();
    if (!u2 || RES_SKIP.test(u2)) return m0;
    return `url(${resBase}/res/${u2.replace(/\\/g, "/").replace(/^\/+/, "")})`;
  });
}

const MIME_BY_EXT: Record<string, string> = {
  ".png": "image/png", ".jpg": "image/jpeg", ".jpeg": "image/jpeg",
  ".gif": "image/gif", ".svg": "image/svg+xml", ".webp": "image/webp",
  ".ico": "image/x-icon", ".bmp": "image/bmp",
  ".css": "text/css", ".js": "application/javascript",
  ".mp3": "audio/mpeg", ".wav": "audio/wav", ".ogg": "audio/ogg",
  ".oga": "audio/ogg", ".aac": "audio/aac", ".m4a": "audio/mp4",
  ".spx": "audio/speex", ".opus": "audio/opus", ".wma": "audio/x-ms-wma",
  ".ttf": "font/ttf", ".otf": "font/otf", ".woff": "font/woff",
  ".woff2": "font/woff2", ".html": "text/html", ".htm": "text/html",
};

function dicresFind(ref: string): Uint8Array | null {
  /** 按精确候选 → 全卷 basename 兜底, 在所有 MDD 卷里找资源 */
  for (const mdx of listMdxFiles()) {
    const data = multiMddLookup(mdx, ref, candidates);
    if (data) return data;
  }
  // basename 兜底
  const base = path.basename(ref.replace(/\\/g, "/")).toLowerCase();
  for (const mdx of listMdxFiles()) {
    let keys: string[] = [];
    try {
      keys = getMultiMDD(mdx).allKeys();
    } catch {
      continue;
    }
    for (const k of keys) {
      if (path.basename(k.replace(/\\/g, "/")).toLowerCase() === base) {
        const data = multiMddLookup(mdx, k, candidates);
        if (data) return data;
      }
    }
  }
  return null;
}

// ============================================================
// 通用判断 / 工具
// ============================================================
function isSingleWord(text: string): boolean {
  /** 单个单词才使用 WebView 词典；短语/句子使用在线词典站。 */
  text = (text || "").trim();
  if (!text || text.split(/\s+/).length !== 1) return false;
  return !/[.!?,;:?。！？，、；：！？]/.test(text);
}

function escHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function _playAudio(audioPath: string): boolean {
  /** 使用系统播放器播放本地音频。 */
  const players: Array<[string, string[]]> = [
    ["mpv", ["mpv", "--no-video", "--really-quiet", audioPath]],
    ["paplay", ["paplay", audioPath]],
    ["aplay", ["aplay", audioPath]],
    ["ffplay", ["ffplay", "-nodisp", "-autoexit", "-loglevel", "quiet", audioPath]],
    ["cvlc", ["cvlc", "--play-and-exit", "--intf", "dummy", audioPath]],
  ];
  for (const [bin, args] of players) {
    if (!Bun.which(bin)) continue;
    try {
      Bun.spawn(args, {
        stdout: "ignore",
        stderr: "ignore",
        stdin: "ignore",
      });
      return true;
    } catch {
      /* ignore */
    }
  }
  return false;
}

// ============================================================
// WebView 词典窗口（原 run_gui_web + WEB_SHELL）
// ============================================================
// 原版 pywebview 的 JS 桥(pywebview.api.X)统一替换为 webview.bind 注入的
// dicApi("X", ...) —— 本地页面与远程页面均可用。

function webShell(word: string, content: string): string {
  return `<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<style>
body { font-family: 'Noto Sans CJK SC', '微软雅黑', sans-serif; font-size: 17px; margin: 0; line-height: 1.55; }
#bar { position: sticky; top: 0; background: #f5f5f5; border-bottom: 1px solid #ddd; padding: 8px; display: flex; gap: 6px; z-index: 99999; }
#q { flex: 1; font-size: 15px; padding: 6px 10px; border: 1px solid #ccc; border-radius: 6px; }
button { padding: 6px 12px; border: 1px solid #ccc; border-radius: 6px; background: #fff; cursor: pointer; }
button:hover { background: #eee; }
#content { padding: 14px 18px; }
#content img { max-width: 100%; }
[data-sound] { cursor: pointer; }
</style>
<style id="dict-css"></style>
</head>
<body>
<div id="bar">
<input id="q" value="${e2(word, true)}" placeholder="输入单词后回车…" autocomplete="off" autofocus>
<button id="lookup-button">查词</button>
</div>
<div id="content">${content}</div>
<script>
var MEDIA_EXT = /\\.(mp3|wav|ogg|oga|aac|m4a|spx|opus|wma)(\\?|#|$)/i;
function flash(msg) {
  var t = document.getElementById('dic-toast');
  if (!t) {
    t = document.createElement('div');
    t.id = 'dic-toast';
    t.style.cssText = 'position:fixed;right:12px;top:52px;background:#333;color:#fff;padding:6px 10px;border-radius:6px;font-size:12px;z-index:999999;max-width:70%;word-break:break-all;display:none;white-space:pre-wrap;';
    document.body.appendChild(t);
  }
  t.textContent = msg;
  t.style.display = 'block';
  clearTimeout(t._tm);
  t._tm = setTimeout(function(){ t.style.display = 'none'; }, 3500);
}
function stripScheme(s) {
  return String(s).replace(/^[a-z][a-z0-9+.\\-]*:\\/\\//i, '');
}
function soundRef(el) {
  if (!el || el.nodeType !== 1) return '';
  var attrs = ['data-sound', 'data-src-mp3', 'src-mp3'];
  for (var i = 0; i < attrs.length; i++) {
    var v = el.getAttribute(attrs[i]);
    if (v) return stripScheme(v);
  }
  if ((el.tagName || '').toLowerCase() === 'a') {
    var h = String(el.getAttribute('href') || '').trim();
    if (!h) return '';
    var low = h.toLowerCase();
    if (low.indexOf('sound://') === 0 || MEDIA_EXT.test(low)) return stripScheme(h);
  }
  return '';
}
function callPlay(ref) {
  dicApi('play', ref).then(function(r) {
    if (r) flash(String(r));
  }).catch(function(err) { flash('⛔ play: ' + err); });
}
function handler(e) {
  if (e.__dicHandled) return;
  var _sel = window.getSelection && window.getSelection();
  if (_sel && _sel.type === 'Range' && String(_sel).length) { return; }
  e.__dicHandled = true;
  var node = e.target;
  if (!node || node.nodeType !== 1) return;
  while (node && node !== document) {
    if (node.nodeType === 1) {
      var ref = soundRef(node);
      if (ref) {
        e.preventDefault(); e.stopPropagation();
        if (e.stopImmediatePropagation) e.stopImmediatePropagation();
        flash('🔊 ' + ref);
        callPlay(ref);
        return;
      }
      if ((node.tagName || '').toLowerCase() === 'a') {
        var h = String(node.getAttribute('href') || '').trim();
        if (h && !/^(https?:|#|javascript:|mailto:|about:)/i.test(h)) {
          e.preventDefault(); e.stopPropagation();
          if (e.stopImmediatePropagation) e.stopImmediatePropagation();
          if (MEDIA_EXT.test(h.toLowerCase()) || h.toLowerCase().indexOf('sound://') === 0) {
            flash('🔊 ' + stripScheme(h));
            callPlay(stripScheme(h));
          } else {
            flash('⛔ 非音频链接: ' + h + '\\n' + String(node.outerHTML || '').substring(0, 120));
          }
          return;
        }
        if (h) return; // 普通网页链接放行
        // 无 href 的 <a>: 继续向上找
      }
    }
    node = node.parentNode;
  }
}
['click', 'auxclick'].forEach(function(ev) {
  window.addEventListener(ev, handler, true);
  document.addEventListener(ev, handler, true);
});
['mousedown', 'pointerdown'].forEach(function(ev) {
  window.addEventListener(ev, function(e) {
    var node = e.target;
    while (node && node !== document) {
      if (node.nodeType === 1 && soundRef(node) && node === e.target) {
        e.preventDefault();
        return;
      }
      node = node.parentNode;
    }
  }, true);
});
function normalizeSounds(root) {
  if (!root || !root.querySelectorAll) return;
  var list = root.querySelectorAll('a[href]');
  for (var i = 0; i < list.length; i++) {
    var el = list[i];
    var h = String(el.getAttribute('href') || '').trim();
    var low = h.toLowerCase();
    if (low.indexOf('sound://') === 0 || MEDIA_EXT.test(low)) {
      el.setAttribute('data-sound', stripScheme(h));
      el.removeAttribute('href');
      el.removeAttribute('target');
      el.removeAttribute('onclick');
    }
  }
  var es = root.querySelectorAll('[data-src-mp3],[src-mp3]');
  for (var j = 0; j < es.length; j++) {
    var e2 = es[j];
    var ref = e2.getAttribute('data-src-mp3') || e2.getAttribute('src-mp3') || '';
    if (ref && !e2.getAttribute('data-sound')) {
      e2.setAttribute('data-sound', stripScheme(ref));
    }
  }
}
function fixSound(root) { /* dic sound v10 */
  if (!root || !root.querySelectorAll) return;
  var els = root.querySelectorAll('[data-sound]');
  for (var i = 0; i < els.length; i++) {
    var a = els[i];
    if (a.__dicDone) continue;
    a.__dicDone = true;
    var ref = a.getAttribute('data-sound') || '';
    if (!ref) continue;
    if (!String(a.textContent).trim() && !a.querySelector('img')) {
      // 罕见空锚点兜底: 绑到词典 CSS 画的图标元素(audio[name=xxx])上
      var base = ref.replace(/\\.mp3$/i, ''), au = null;
      try { au = root.querySelector('audio[name="' + base + '"]'); } catch (err) {}
      if (au && au !== a) {
        au.setAttribute('data-sound', ref);
        au.__dicDone = true;
        au.style.cursor = 'pointer';
        if (!au.style.display) au.style.display = 'inline-block';
        continue;
      }
      var sib = a.nextElementSibling;
      if (sib && !sib.getAttribute('data-sound') && String(sib.textContent).trim()) {
        sib.setAttribute('data-sound', ref);
        sib.__dicDone = true;
      }
      continue;
    }
    // 整行提升: pron 容器优先, 否则短音标行提升父容器
    var done = false, p = a.parentElement;
    for (var up = 0; p && p !== root && up < 2; up++, p = p.parentElement) {
      var tn = String(p.tagName || '') + ' ' + String(p.getAttribute('class') || '');
      if (tn.toLowerCase().indexOf('pron') >= 0) {
        var kids = p.querySelectorAll('[data-sound]');
        var all = kids.length > 0;
        for (var k2 = 0; k2 < kids.length; k2++) {
          if (String(kids[k2].getAttribute('data-sound')) !== ref) { all = false; break; }
        }
        if (all && !p.getAttribute('data-sound')) {
          p.setAttribute('data-sound', ref);
          done = true;
        }
        break;
      }
    }
    if (!done) {
      var par = a.parentElement;
      if (par && par !== root && !par.getAttribute('data-sound')) {
        var ks = par.querySelectorAll('[data-sound]');
        if (ks.length === 1 && String(par.textContent).replace(/\\s+/g, '').length <= 40) {
          par.setAttribute('data-sound', ref);
        }
      }
    }
  }
}
function normalizeBoth(root) {
  normalizeSounds(root);
  try { fixSound(root); } catch (err) {}
}
var observer = new MutationObserver(function() {
  normalizeBoth(document.getElementById('content'));
});
observer.observe(document.documentElement, {childList: true, subtree: true});
function applyCss(css) {
  var st = document.getElementById('dict-css');
  if (st) st.textContent = css || '';
}
function doLookup() {
  var word = document.getElementById('q').value.trim();
  if (!word) return;
  fetch('/api/lookup?q=' + encodeURIComponent(word))
    .then(function(r) { return r.json(); })
    .then(function(r) {
      if (!r || r.switch) return;
      document.getElementById('content').innerHTML = r.html || '';
      applyCss(r.css || '');
      normalizeBoth(document.getElementById('content'));
    }).catch(function(err) { flash('⛔ ' + err); });
}
document.getElementById('lookup-button').addEventListener('click', doLookup);
document.getElementById('q').addEventListener('keydown', function(e) {
  if (e.key === 'Enter') { e.preventDefault(); doLookup(); return; }
  if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); try { dicApi('close'); } catch (err) {} }
}, true);
normalizeBoth(document.getElementById('content'));
</script>
</body>
</html>`;
}

const OXFORD9_CSS = `
pron-g-blk br {
    display: none !important;
}

[data-sound] {
    cursor: pointer;
    text-decoration: none;
}

#content {
    font-size: 17px !important;
    line-height: 1.65 !important;
}

/* dic select patch */
html, body, #content, #content * {
    -webkit-user-select: text !important;
    user-select: text !important;
}

[data-sound], [data-sound] * {
    cursor: pointer;
}
`;

/** 远程页/所有页注入: Esc / Ctrl+Shift+I / 全局缩放 / 滚动补丁 / 站点切换工具条 */
const INIT_SCRIPT = `
(function(){
  function flash(msg) {
    var t = document.getElementById('dic-toast');
    if (!t) {
      t = document.createElement('div');
      t.id = 'dic-toast';
      t.style.cssText = 'position:fixed;right:12px;top:52px;background:#333;color:#fff;padding:6px 10px;border-radius:6px;font-size:12px;z-index:2147483600;max-width:70%;word-break:break-all;display:none;white-space:pre-wrap;';
      (document.body || document.documentElement).appendChild(t);
    }
    t.textContent = msg;
    t.style.display = 'block';
    clearTimeout(t._tm);
    t._tm = setTimeout(function(){ t.style.display = 'none'; }, 3500);
  }
  window.flash = flash;
  function applyZoom(z) {
    try { document.documentElement.style.zoom = z; } catch (err) {}
  }
  var DIC_ZOOM = 1.0;
  if (window.dicApi) {
    try { dicApi('getZoom').then(function(z){ DIC_ZOOM = Number(z) || 1.0; applyZoom(DIC_ZOOM); }); } catch (err) {}
  }
  document.addEventListener('keydown', function(e) {
    if (e.ctrlKey && e.shiftKey && (e.key === 'I' || e.key === 'i')) {
      e.preventDefault();
      try { dicApi('devtools'); } catch (err) {}
      return;
    }
    if (e.key === 'Escape') {
      e.preventDefault();
      try { dicApi('close'); } catch (err) {}
      return;
    }
    if (e.ctrlKey && !e.altKey && !e.metaKey) {
      var k = e.key;
      if (k === '=' || k === '+') { DIC_ZOOM = Math.min(3, +(DIC_ZOOM + 0.1).toFixed(2)); }
      else if (k === '-') { DIC_ZOOM = Math.max(0.5, +(DIC_ZOOM - 0.1).toFixed(2)); }
      else if (k === '0') { DIC_ZOOM = 1.0; }
      else if (k === 'ArrowDown') { window.scrollBy(0, Math.round(window.innerHeight*0.9)); e.preventDefault(); return; }
      else if (k === 'ArrowUp') { window.scrollBy(0, -Math.round(window.innerHeight*0.9)); e.preventDefault(); return; }
      else if (k === 'PageDown') { window.scrollBy(0, Math.round(window.innerHeight*0.9)); e.preventDefault(); return; }
      else if (k === 'PageUp') { window.scrollBy(0, -Math.round(window.innerHeight*0.9)); e.preventDefault(); return; }
      else if (k === 'Home') { window.scrollTo(0,0); e.preventDefault(); return; }
      else if (k === 'End') { window.scrollTo(0,document.body.scrollHeight); e.preventDefault(); return; }
      else { return; }
      e.preventDefault(); e.stopPropagation();
      applyZoom(DIC_ZOOM);
      try { dicApi('saveCfg', { zoom: DIC_ZOOM }); } catch (err) {}
      flash('🔍 缩放 ' + Math.round(DIC_ZOOM * 100) + '%');
    }
  }, true);
  document.addEventListener('DOMContentLoaded', function() {
    if (!window.dicApi) return;
    try {
      dicApi('onlineBarData').then(function(d) {
        if (!d || !d.sites || !d.sites.length) return;
        var b = document.getElementById('dic-online-bar');
        if (b) b.remove();
        b = document.createElement('div');
        b.id = 'dic-online-bar';
        b.style.cssText = 'position:fixed;top:0;left:0;right:0;height:36px;background:#eef3fa;border-bottom:1px solid #dbe5f0;display:flex;align-items:center;gap:6px;padding:0 8px;z-index:2147483000;font-size:13px;';
        function mk(label, fn) {
          var btn = document.createElement('button');
          btn.textContent = label;
          btn.style.cssText = 'padding:2px 8px;font-size:12px;cursor:pointer;';
          btn.onclick = fn;
          return btn;
        }
        d.sites.forEach(function(it, i) {
          b.appendChild(mk(it.n, function(){ dicApi('onlineOpen', i); }));
        });
        b.appendChild(mk('⌂', function(){ dicApi('onlineHome'); }));
        b.appendChild(mk('✕', function(){ dicApi('close'); }));
        document.body.appendChild(b);
        document.body.style.paddingTop = '44px';
      });
    } catch (err) {}
  });
})();
`;

// ============================================================
// GUI 主入口 (原 run_gui_web)
// ============================================================
export async function runGuiWeb(
  initialText?: string,
  opts?: { headless?: boolean },
): Promise<void> {
  logErr(`webview pid=${process.pid}`);
  const initial = (initialText || "").trim(); // 原版: initial = (initial_text or "").strip()

  let Webview: any;
  if (!opts?.headless) {
    try {
      ({ Webview } = await import("webview-bun"));
    } catch (e) {
      console.error("缺少 webview-bun，请先在脚本目录执行: bun install");
      logErr((e as Error).message);
      process.exit(1);
    }
  }

  const state = {
    mdx: null as string | null,
    word: null as string | null,
    online: false,
    last_query: "",
  };

  function gotoOnline(text: string) {
    /** (dic stack patch) GoldenDict 式: 所有在线站点上下拼接在同一页面 */
    text = (text || "").trim();
    state.last_query = text;
    state.online = false; // 本地拼接页, 不注入远程工具条
    return { online: true, html: onlineStackedHtml(text), css: "" };
  }

  function prepare(htmlText: string, resBase: string): string {
    htmlText = htmlText
      .replaceAll("<xhtml:", "<")
      .replaceAll("</xhtml:", "</");
    // 词典自带 JS 会重新接管发音链接, 一律剥离; 内联 on* 事件一并清除
    htmlText = htmlText.replace(/<script\b[\s\S]*?<\/script\s*>/gi, "");
    htmlText = htmlText.replace(/<script\b[^>]*\/?>/gi, "");
    htmlText = htmlText.replace(/\s+on[a-z]+\s*=\s*("[^"]*"|'[^']*'|[^\s>]+)/gi, "");
    // dic select patch: 清理禁选属性/内联样式, 允许选中复制
    htmlText = htmlText.replace(/\s+unselectable\s*=\s*['"]?(on|true)['"]?/gi, "");
    htmlText = htmlText.replace(
      /(style\s*=\s*['"][^'"]*?)user-select\s*:\s*none\s*;?/gi,
      "$1",
    );
    const root = parseHtml(htmlText);
    if (root) {
      const snd = /^sound:\/\/|\.mp3(\?|#|$)/i;
      for (const el of [...iterAll(root)]) {
        const tag = tagOf(el);
        if (tag === "img") {
          // (dic img patch) 词典图片 → 本地资源服务直读
          const v = (attrOf(el, "src") || "").trim();
          if (v && !RES_SKIP.test(v)) {
            setAttr(
              el,
              "src",
              `${resBase}/res/` + v.replace(/\\/g, "/").replace(/^\/+/, ""),
            );
          }
          delAttr(el, "srcset");
        }
        let ref = "";
        for (const attr of Object.keys({ ...el.attribs })) {
          const a = attr.toLowerCase();
          if (
            !["href", "src", "data-src-mp3", "src-mp3", "data-sound"].includes(a)
          )
            continue;
          const v = (attrOf(el, attr) || "").trim();
          if (a === "data-sound") {
            ref = ref || v;
          } else if (snd.test(v) || a === "data-src-mp3" || a === "src-mp3") {
            if (!ref) ref = v.replace(/^sound:\/\//i, "");
            delAttr(el, attr);
          }
        }
        if (ref) setAttr(el, "data-sound", ref);
        if (tag === "a") {
          for (const junk of ["href", "target", "onclick"]) delAttr(el, junk);
        }
      }
      // dic sound-propagate patch: 把发音引用提升到 pron 容器,
      // 点击音标文字/整行也能发音 (GoldenDict 风格)
      for (const el of [...iterAll(root)]) {
        const ref = attrOf(el, "data-sound");
        if (!ref) continue;
        const p = el.parent as Element | null;
        if (!p || attrOf(p, "data-sound")) continue;
        if (!(attrOf(p, "class") || "").toLowerCase().includes("pron")) continue;
        const refs = new Set<string>();
        for (const s of iterAll(p)) {
          const v = attrOf(s, "data-sound");
          if (v) refs.add(v);
        }
        if (refs.size === 1) setAttr(p, "data-sound", ref);
      }
      // dic nested-a patch: HTML 不允许 <a> 嵌套, WebKit 解析时会截断
      // 外层 data-sound 锚点(音标/图标失去可点性); 把内层 <a> 改名 span
      for (const a of [...iterAll(root)]) {
        if (tagOf(a) !== "a") continue;
        let p = a.parent as Element | null;
        while (p) {
          if (tagOf(p) === "a") {
            a.name = "span";
            break;
          }
          p = p.parent as Element | null;
        }
      }
      htmlText = serializeDoc(root);
    }
    // 兜底: 清掉一切残留的可导航 sound:// 引用(含未加引号写法)
    htmlText = htmlText.replace(/\s+href\s*=\s*["']?sound:\/\/[^\s>"']*/gi, "");
    htmlText = htmlText.replace(/\s+src\s*=\s*["']?sound:\/\/[^\s>"']*/gi, "");
    return htmlText;
  }

  function dump(htmlText: string, css: string): void {
    if (!process.env.DIC_DUMP) return;
    try {
      fs.writeFileSync("/tmp/plastic-dict/last.html", htmlText);
      fs.writeFileSync("/tmp/plastic-dict/last.css", css);
    } catch {
      /* ignore */
    }
  }

  async function apiLookup(word: string): Promise<any> {
    word = (word || "").trim();
    if (!word) return { html: "", css: "" };
    // dic online patch: 短语/句子 -> 本窗口打开在线词典
    if (!isSingleWord(word)) return gotoOnline(word);
    const resBase = `http://127.0.0.1:${serverPort}`;
    const r = lookup(word);
    if (!r.found) {
      const t = await translateText(word);
      return {
        html: `<p><b>${escHtml(word)}</b>（本地未收录，在线翻译）</p><p>${escHtml(t)}</p>`,
        css: "",
      };
    }
    state.mdx = r._mdx;
    state.word = r.word;
    const htmlText = prepare(r._html, resBase);
    const css = cssLocalize(r.css, resBase) + "\n" + OXFORD9_CSS;
    dump(htmlText, css);
    // 不加载词典自带 JS，避免其重新接管发音链接。
    return { html: htmlText, css };
  }

  function playLog(...a: unknown[]): void {
    try {
      fs.appendFileSync(
        "/tmp/plastic-dict/play.log",
        new Date().toTimeString().slice(0, 8) +
          " " +
          a.map(String).join(" ") +
          "\n",
      );
    } catch {
      /* ignore */
    }
  }

  async function apiPlay(ref: string): Promise<string> {
    const word = state.word || "";
    logErr("[plastic-dict.play]", ref, "| word:", word);
    playLog("play called:", JSON.stringify(ref), "| word:", JSON.stringify(word));
    if (!state.mdx || !ref) return "⛔ play: 内部状态缺失";
    ref = String(ref).trim();
    ref = ref.replace(/^[a-z][a-z0-9+.-]+:\/\//i, "");
    let audioPath: string | null = null;
    try {
      audioPath = extractAudio(state.mdx, ref);
    } catch (e) {
      playLog("提取异常:", String(e));
    }
    if (!audioPath) {
      playLog(`未命中 ${JSON.stringify(ref)}, 词条兜底: ${JSON.stringify(word)}`);
      for (const variant of ["gb", "us"] as const) {
        try {
          audioPath = word ? getPlayable(word, variant) : null;
        } catch {
          audioPath = null;
        }
        if (audioPath) {
          playLog("兜底命中:", variant);
          break;
        }
      }
    }
    if (!audioPath) {
      playLog("最终无音频:", ref);
      return `⛔ 找不到音频: ${ref}`;
    }
    const played = _playAudio(audioPath);
    playLog("播放:", audioPath, played ? "成功" : "失败");
    return played ? `🔊 ${path.basename(audioPath)}` : "⛔ 播放失败";
  }

  function entryPage(word: string, body: string, css: string): string {
    return webShell(word, body).replace(
      '<style id="dict-css"></style>',
      '<style id="dict-css">' + css + "</style>",
    );
  }

  // ---- 配置 ----
  const cfgd = loadCfg();
  let zoom0 = 1.0;
  try {
    zoom0 = Math.min(3.0, Math.max(0.5, Number(cfgd.zoom || 1.0)));
  } catch {
    zoom0 = 1.0;
  }
  const w0 = cfgInt(cfgd, "width", 760, 320, 7680);
  const h0 = cfgInt(cfgd, "height", 860, 300, 4320);

  // ---- 本地资源/页面服务 (替代原 dicres:// 自定义协议) ----
  let serverPort = 0;
  let server: ReturnType<typeof Bun.serve> | null = null;

  async function handleReq(req: Request): Promise<Response> {
    const u = new URL(req.url);
    const p = u.pathname;
    try {
      if (p === "/" || p === "/entry") {
        const initial = (u.searchParams.get("q") || "").trim();
        const auto = u.searchParams.get("auto");
        let body = "";
        let word0 = "";
        let css0 = OXFORD9_CSS;
        const resBase = `http://127.0.0.1:${serverPort}`;
        if (initial && isSingleWord(initial)) {
          word0 = initial;
          const r = lookup(initial);
          if (r.found) {
            state.mdx = r._mdx;
            state.word = r.word;
            body = prepare(r._html, resBase);
            css0 = cssLocalize(r.css, resBase) + "\n" + OXFORD9_CSS;
            dump(body, css0);
          } else {
            body = `<p>${escHtml(await translateText(initial))}</p>`;
          }
        } else if (initial) {
          // (dic stack patch) 非单词 -> 直接把拼接式在线词典作为初始内容
          state.last_query = initial;
          state.online = false;
          body = onlineStackedHtml(initial);
        }
        if (auto && word0) {
          // onlineHome 回家时自动重查当前词
          body +=
            '<script>(function w(){if(window.dicApi){try{doLookup();}catch(e){}}else{setTimeout(w,200);}})();</script>';
        }
        return new Response(entryPage(word0, body, css0), {
          headers: { "Content-Type": "text/html; charset=utf-8" },
        });
      }
      if (p === "/api/lookup") {
        const word = u.searchParams.get("q") || "";
        return new Response(JSON.stringify(await apiLookup(word)), {
          headers: { "Content-Type": "application/json; charset=utf-8" },
        });
      }
      if (p === "/api/onlineBar") {
        const sites = state.online
          ? onlineSiteUrls(state.last_query).map((uu, i) => ({
              n: onlineSiteNames()[i] ?? uu,
              u: uu,
            }))
          : [];
        return new Response(
          JSON.stringify({ sites, q: state.online ? state.last_query : "" }),
          { headers: { "Content-Type": "application/json; charset=utf-8" } },
        );
      }
      if (p.startsWith("/res/")) {
        const ref = decodeURIComponent(p.slice("/res/".length));
        const data = dicresFind(ref);
        if (data) {
          const ext = path.extname(ref).toLowerCase();
          return new Response(Buffer.from(data), {
            headers: {
              "Content-Type": MIME_BY_EXT[ext] ?? "application/octet-stream",
            },
          });
        }
        return new Response("", { status: 404 });
      }
      return new Response("not found", { status: 404 });
    } catch (e) {
      logErr("请求处理失败:", p, e);
      return new Response("internal error", { status: 500 });
    }
  }

  server = Bun.serve({
    hostname: "127.0.0.1",
    port: 0, // 随机可用端口, 仅监听本机
    fetch: handleReq,
  });
  serverPort = server.port ?? 0;
  const baseUrl = `http://127.0.0.1:${serverPort}`;

  // 无头模式: 只起本地服务(自测用), 不创建窗口
  if (opts?.headless) {
    console.log(`[plastic-dict] headless server: ${baseUrl}`);
    await new Promise<never>(() => {}); // 挂住直到被 kill
    return;
  }

  // ---- 创建 webview ----
  const wv = new Webview(true /* debug: 允许右键检查元素 */, {
    width: w0,
    height: h0,
  });
  wv.title = DICT_NAME;

  let running = true;
  let windowGone = false;
  wv.bind("dicApi", (method: string, ...args: any[]) => {
    return (async () => {
      switch (method) {
        case "play":
          return apiPlay(String(args[0] ?? ""));
        case "onlineOpen": {
          let idx = parseInt(String(args[0] ?? "0"), 10);
          if (Number.isNaN(idx)) idx = 0;
          const urls = onlineSiteUrls(state.last_query);
          if (!urls.length) return "⛔ 无查询内容";
          if (idx < 0 || idx >= urls.length) idx = 0;
          state.online = true;
          try {
            wv.navigate(urls[idx]);
            return "🌐 " + onlineSiteNames()[idx];
          } catch (e) {
            return `⛔ 打开失败: ${(e as Error).message}`;
          }
        }
        case "onlineHome": {
          /** dic online patch: 返回本地词典查询页 */
          state.online = false;
          const w = state.word || "";
          const auto = w ? "1" : "";
          wv.navigate(
            `${baseUrl}/entry?q=${encodeURIComponent(w)}${auto ? "&auto=1" : ""}`,
          );
          return true;
        }
        case "close":
          running = false;
          return true;
        case "saveCfg":
          try {
            saveCfgMerged((args[0] ?? {}) as DicCfg);
          } catch {
            /* ignore */
          }
          return true;
        case "getZoom":
          return zoom0;
        case "onlineBarData": {
          /** 远程站点页注入工具条用; 走 bind 通道以绕过站点 CSP */
          const sites = state.online
            ? onlineSiteUrls(state.last_query).map((uu, i) => ({
                n: onlineSiteNames()[i] ?? uu,
                u: uu,
              }))
            : [];
          return { sites, q: state.online ? state.last_query : "" };
        }
        case "devtools":
          return "请右键页面 → 检查元素 (WebInspector)";
        default:
          return { error: "unknown method: " + method };
      }
    })();
  });

  wv.init(INIT_SCRIPT);

  const url0 = initial
    ? `${baseUrl}/entry?q=${encodeURIComponent(initial)}`
    : `${baseUrl}/entry?q=`;
  wv.navigate(url0);

  // ---- 手动驱动 GTK 主循环 (替代 webview.run(), 保持 Bun 事件循环可用) ----
  const { dlopen, FFIType } = await import("bun:ffi");
  let glib: any = null;
  let gtk4: any = null;
  try {
    glib = dlopen("libglib-2.0.so.0", {
      g_main_context_iteration: {
        args: [FFIType.ptr, FFIType.i32],
        returns: FFIType.i32,
      },
    });
  } catch (e) {
    console.error(
      "无法加载 libglib-2.0.so.0 (系统 webview 依赖), 请安装 GTK4 运行时:",
      (e as Error).message,
    );
    process.exit(1);
  }
  try {
    gtk4 = dlopen("libgtk-4.so.1", {
      gtk_window_present: { args: [FFIType.ptr], returns: FFIType.void },
      gtk_window_maximize: { args: [FFIType.ptr], returns: FFIType.void },
      gtk_widget_get_visible: { args: [FFIType.ptr], returns: FFIType.i32 },
    });
  } catch {
    gtk4 = null; // 退化: 无最大化/窗口关闭检测, 依赖 JS 的 Esc/✕
  }

  const gtkw = wv.window; // GtkWindow* (GTK4)
  try {
    if (gtkw && gtk4) {
      gtk4.symbols.gtk_window_present(gtkw);
      if (cfgd.maximized) gtk4.symbols.gtk_window_maximize(gtkw);
    }
  } catch (e) {
    logErr("窗口呈现失败:", (e as Error).message);
  }

  try {
    while (running) {
      // 处理一批 GTK 事件, 然后让 Bun 事件循环跑一拍(服务请求/定时器)
      glib.symbols.g_main_context_iteration(null, 0);
      if (gtk4 && gtkw && !windowGone) {
        let visible = 1;
        try {
          visible = gtk4.symbols.gtk_widget_get_visible(gtkw);
        } catch {
          visible = 1;
        }
        if (!visible) {
          windowGone = true; // 窗口被窗口管理器关闭
          break;
        }
      }
      await Bun.sleep(2);
    }
  } finally {
    try {
      server?.stop(true);
    } catch {
      /* ignore */
    }
    for (const inst of [...builders.values(), ..._mddCache.values()]) {
      try {
        inst?.scanner?.close?.();
      } catch {
        /* ignore */
      }
    }
    if (!windowGone) {
      try {
        wv.destroy();
      } catch {
        /* ignore */
      }
    }
  }
}

// ============================================================
// CLI (原 run_cli)
// ============================================================
async function runCli(content: string): Promise<number> {
  content = (content || "").trim();
  if (!content) return 1;
  if (!isSingleWord(content)) {
    // dic online patch: 短语/句子 -> 弹出 WebView 在线词典窗口
    console.log("🌐 非单词输入: 打开在线词典窗口…");
    await runGuiWeb(content);
    return 0;
  }
  const r = lookup(content);
  if (!r.found) {
    const out = await translateText(content);
    console.log(out);
    return out ? 0 : 1;
  }
  console.log(`--- ${r.dict} | ${r.word}\n`);
  console.log(r.text);
  for (const variant of ["gb", "us"] as const) {
    const p = getPlayable(r.word, variant);
    if (p) console.log(`  🎧 ${p}`);
  }
  return 0;
}

// ============================================================
// 入口 (原 main)
// ============================================================
function printUsage(): number {
  console.log(
    [
      "用法:",
      '  ./plastic-dict.ts "<content>"   单词→WebView 本地词典; 短语/句子→WebView 在线词典',
      '  ./plastic-dict.ts -g "<content>" 同上',
      '  ./plastic-dict.ts -w "<content>" 强制 WebView (不做自动切换, 现已与默认一致)',
      '  ./plastic-dict.ts -t "<content>" 命令行模式 (短语时弹 WebView 在线词典窗口)',
      "  ./plastic-dict.ts --purge-cache  清空词典索引磁盘缓存",
      "",
      "配置: ~/.config/plastic-dict/config.json (zoom/width/height/maximized)",
    ].join("\n"),
  );
  return 0;
}

async function main(): Promise<number> {
  const args = process.argv.slice(2);
  if (!args.length) {
    console.error('用法: ./plastic-dict.ts [-g|-w|-t] "<content>"');
    return 1;
  }
  if (args[0] === "--native") {
    console.error("Qt 原生窗口已移除, 请直接使用默认模式或 -w (系统 WebView)。");
    const content = args.slice(1).join(" ").trim();
    if (content) await runGuiWeb(content);
    return 0;
  }
  if (args[0] === "--purge-cache") {
    const n = purgeAllCache();
    console.log(`已清理 ${n} 个索引缓存文件 (${INDEX_CACHE_DIR})`);
    return 0;
  }
  if (args[0] === "--web") {
    const content = args.slice(1).join(" ").trim();
    if (content) await runGuiWeb(content);
    return 0;
  }
  if (args[0] === "-g" || args[0] === "--gui") {
    const content = args.slice(1).join(" ").trim();
    if (!content) {
      console.error('用法: ./plastic-dict.ts -g "<content>"');
      return 1;
    }
    // 单词→本地词典, 短语/句子→在线词典, 均在 WebView 内
    await runGuiWeb(content);
    return 0;
  }
  if (args[0] === "-w" || args[0] === "--webview") {
    const content = args.slice(1).join(" ").trim();
    if (!content) {
      console.error('用法: ./plastic-dict.ts -w "<content>"');
      return 1;
    }
    await runGuiWeb(content);
    return 0;
  }
  if (args[0] === "-t" || args[0] === "--text") {
    const content = args.slice(1).join(" ").trim();
    if (!content) {
      console.error('用法: ./plastic-dict.ts -t "<content>"');
      return 1;
    }
    return runCli(content);
  }
  if (args[0] === "-h" || args[0] === "--help") {
    return printUsage();
  }
  const content = args.join(" ").trim();
  if (!content) return 1;
  // 单词→本地词典, 短语/句子→在线词典, 均在 WebView 内
  await runGuiWeb(content);
  return 0;
}

// ---- 全局崩溃日志 (原 __main__ 兜底) ----
process.on("uncaughtException", (err) => {
  try {
    fs.appendFileSync(
      "/tmp/plastic-dict/crash.log",
      `[${new Date().toISOString()}] ${err.stack ?? err}\n`,
    );
  } catch {
    /* ignore */
  }
  throw err;
});

if (import.meta.main) {
  main()
    .then((code) => process.exit(code))
    .catch((err) => {
      try {
        fs.appendFileSync(
          "/tmp/plastic-dict/crash.log",
          `[${new Date().toISOString()}] ${err?.stack ?? err}\n`,
        );
      } catch {
        /* ignore */
      }
      console.error(err);
      process.exit(1);
    });
}
