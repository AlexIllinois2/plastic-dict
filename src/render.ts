import {
  attrOf, childElems, dropTag, dropTree, isElem,
  iterAll, parseHtml, serializeDoc, tagOf, textContent,
} from "./dom";
import { Element, Text } from "domhandler";

const NOISE = new Set([
  "script", "style", "link", "head", "symbol", "xsymb", "img", "hkey",
  "topic", "ftindex", "fthzmark", "fthzindex", "xhtml", "sdsymb",
  "audio-wr", "audio", "audio-gbs-liju", "audio-uss-liju",
  "audio-brs-liju", "audio-ams-liju", "un",
]);

const RENDER_NOISE_EXTRA = new Set([
  "xr-g", "xr-gs", "cf-blk", "cf", "syn-g-blk", "syn-g", "syn-gs",
  "lb-g", "lb", "lmb", "symbol", "un", "unx-g",
]);

const SKIP_TAGS = new Set([
  "h", "phon", "pos", "brelabel", "namelabel", "pron-g",
  "vpform", "infl", "v-g",
]);

const RESET: Record<string, boolean> = {
  "pos-g": true, "pv-blk": false, "pv-g-blk": false,
  "idm-blk": false, "idm-gs-blk": false, "subentry-g": false, boxblock: false,
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
  ["past participle", "过去分词"], ["past simple", "过去式"],
  ["-ing form", "现在分词"], ["present simple - he", "三单"],
  ["present simple", "原形"], ["plural", "复数"], ["third person", "三单"],
];

function _normPhon(p: string): string {
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

  // ---- 1. 提取词形变化表 ----
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
    if (NOISE.has(t) || RENDER_NOISE_EXTRA.has(t)) dropTree(el);
    else if (t === "div" && (attrOf(el, "class") || "").includes("cixing_tiaozhuan"))
      dropTree(el);
  }
  for (const tag of ["audio-gb", "audio-us", "pron-g", "audio"]) {
    for (const el of [...iterAll(root)]) if (tagOf(el) === tag) dropTag(el);
  }

  // ---- 3. 词头 + 音标 ----
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
    head.push("音标：" + prons.map(([lab, ph]) => `${lab} ${ph}`).join(" ｜ "));
  if (inflRows.length) {
    head.push("词形变化：");
    for (const [name, w, ph] of inflRows)
      head.push(`  ${name}：${w}${ph ? " " + ph : ""}`);
  }

  // ---- 4. 正文行走 ----
  const outLines: string[] = [];
  const cur = { ind: 4, chunks: [] as string[], chn: [] as string[] };
  const st = { sn: 0, num: true, top: 0 };
  const sections = new Set<string>();

  const noise = new Set([...NOISE, ...RENDER_NOISE_EXTRA]);
  const B = (t: string): string => (bold ? `\x1b[1m${t}\x1b[0m` : t);
  const add = (s: string): void => { cur.chunks.push(s); };
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
    const cls = (attrOf(el, "class") || "").split(/\s+/);
    if (cls.some((c) => c.includes("slash"))) return;
    if (tag === "top-g") {
      st.top += 1;
      if (st.top > 1) { flush(); blank(); }
      for (const c of childElems(el)) walk(c);
      flush();
      return;
    }
    if (SKIP_TAGS.has(tag)) return;
    if (tag === "br") { flush(); return; }
    if (tag in RESET) { st.sn = 0; st.num = RESET[tag]; }
    if (tag === "pos-g") {
      let p = "";
      for (const c of iterAll(el)) {
        if (tagOf(c) === "pos") { p = textContent(c).trim().toLowerCase(); break; }
      }
      flush(); blank();
      if (p) {
        const key = p in POS ? POS[p] : p;
        outLines.push(`${POS_CN[key] ?? key} (${key})：`);
        st.sn = 0; st.num = true; cur.ind = 4;
      }
      return;
    }
    if (tag === "sn-g") {
      flush(); cur.ind = 4;
      if (st.num) { st.sn += 1; add(`${st.sn}. `); }
    } else if (tag === "shcut") {
      const t = textContent(el).trim();
      if (t) { flush(); blank(); outLines.push("  ◆ " + B(t)); cur.ind = 4; }
      return;
    } else if (tag === "x-g-blk") { flush(); cur.ind = 6; }
    else if (tag === "pv-blk" || tag === "pv-g-blk") {
      flush(); blank();
      if (tag === "pv-blk" && !sections.has("pv")) {
        outLines.push("短语动词 (Phrasal Verbs)：");
        sections.add("pv");
      }
      cur.ind = 6;
    } else if (tag === "idm-blk" || tag === "idm-gs-blk") {
      flush(); blank();
      if (!sections.has("idm")) {
        outLines.push("常用习语 (Idioms)：");
        sections.add("idm");
      }
      cur.ind = 6;
    } else if (tag === "pv" || tag === "idm") {
      const t = textContent(el).trim();
      if (t) { flush(); blank(); outLines.push("  · " + B(t)); cur.ind = 6; }
      return;
    } else if (tag === "x-g") {
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
      flush(); cur.ind += 2; add("- ");
      for (const c of childElems(el)) walk(c);
      flush(); cur.ind -= 2;
      return;
    }
    for (const c of el.children ?? []) {
      if (c.type === "text") add((c as Text).data);
      else if (isElem(c)) walk(c);
    }
  };

  for (const c of childElems(root)) walk(c);
  flush();

  // ---- 5. 组装 + 折行 ----
  const parts = [...head, ...(head.length && outLines.length ? [""] : []), ...outLines];
  const out: string[] = [];
  for (const line of parts) {
    const ind = (line.match(/^( *)/) ?? ["", ""])[1];
    if (line.length <= width) { out.push(line); continue; }
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

export function stripDup(html: string): string {
  const root = parseHtml(html);
  if (!root) return html;
  let removed = 0;
  for (const el of [...iterAll(root)]) {
    if (tagOf(el) !== "aunbox") continue;
    const p = el.parent as Element | null;
    if (!p) continue;
    const twin = childElems(p).some(
      (s) => s !== el && tagOf(s) === "unbox" && textContent(s) === textContent(el),
    );
    if (twin) { dropTree(el); removed += 1; }
  }
  return removed ? serializeDoc(root) : html;
}

export function headwordChn(html: string): string {
  const root = parseHtml(html);
  if (!root) return "";
  for (const el of iterAll(root)) {
    if (tagOf(el) !== "chn") continue;
    let p = el.parent as Element | null;
    let inside = false;
    while (p) {
      const t = tagOf(p);
      if (t === "unbox" || t === "aunbox") { inside = true; break; }
      p = p.parent as Element | null;
    }
    if (inside) continue;
    const t = textContent(el).trim();
    if (t) return t;
  }
  return "";
}
