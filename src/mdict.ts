import { MDX, MDD } from "js-mdict";
import * as fs from "node:fs";
import * as path from "node:path";
import * as crypto from "node:crypto";
import { DICT_DIR, AUDIO_DIR, INDEX_CACHE_DIR, logErr } from "./paths";
import { render, stripDup, headwordChn } from "./render";

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
  } catch { return null; }
}

function indexCachePath(file: string): string {
  const h = crypto.createHash("sha1").update(path.resolve(file)).digest("hex");
  return path.join(INDEX_CACHE_DIR, `${path.basename(file)}.${h}.json`);
}

const CACHE_FORMAT = 1;

function readIndexCache(file: string): any | null {
  const stamp = fileStamp(file);
  if (!stamp) return null;
  try {
    const raw = JSON.parse(fs.readFileSync(indexCachePath(file), "utf8"));
    if (
      raw?.v !== CACHE_FORMAT || raw?.size !== stamp.size ||
      raw?.mtimeMs !== stamp.mtimeMs ||
      !Array.isArray(raw?.keywordList) || !Array.isArray(raw?.recordInfoList)
    ) return null;
    return raw;
  } catch { return null; }
}

function saveIndexCache(inst: any, file: string): void {
  const stamp = fileStamp(file);
  if (!stamp) return;
  try {
    fs.mkdirSync(INDEX_CACHE_DIR, { recursive: true });
    const meta = inst.meta ?? {};
    const data = {
      v: CACHE_FORMAT, lib: "js-mdict",
      size: stamp.size, mtimeMs: stamp.mtimeMs,
      meta: {
        ext: meta.ext, encoding: meta.encoding, numWidth: meta.numWidth,
        version: meta.version, encrypt: meta.encrypt ?? -1, numFmt: meta.numFmt ?? null,
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
  try { fs.rmSync(indexCachePath(file), { force: true }); } catch { /* ignore */ }
}

export function purgeAllCache(): number {
  let n = 0;
  try {
    for (const f of fs.readdirSync(INDEX_CACHE_DIR)) {
      try { fs.rmSync(path.join(INDEX_CACHE_DIR, f), { force: true }); n += 1; }
      catch { /* ignore */ }
    }
  } catch { /* ignore */ }
  return n;
}

function makeScanner(file: string) {
  const fd = fs.openSync(file, "r");
  return {
    offset: 0, filepath: file, fd,
    close() { try { fs.closeSync(fd); } catch { /* ignore */ } },
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
      decoder = new TextDecoder(enc.toUpperCase() === "UTF-16" ? "utf-16le" : enc.toLowerCase());
    } catch { decoder = new TextDecoder("utf-8"); }
    inst.meta = {
      fname: file, passcode: undefined,
      ext: c.meta?.ext ?? kind,
      version: c.meta?.version ?? 2,
      numWidth: c.meta?.numWidth ?? 8,
      numFmt: c.meta?.numFmt ?? { begin: 0, end: 0, step: 0 },
      encoding: c.meta?.encoding ?? "UTF-8",
      decoder, encrypt: c.meta?.encrypt ?? -1,
    };
    inst.options = {
      passcode: undefined, debug: false, resort: true,
      isStripKey: true, isCaseSensitive: false, encryptType: -1,
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
    const first = inst.keywordList?.[0]?.keyText;
    if (typeof first === "string" && first) {
      if (kind === "mdd") inst.locate(first);
      else inst.lookup(first);
    } else if ((inst.keywordList?.length ?? 0) !== 0) return null;
    return inst;
  } catch {
    try { inst?.scanner?.close?.(); } catch { /* ignore */ }
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
      purgeIndexCache(mdx);
      logErr(`  [构建] 索引(一次性): ${path.basename(mdx)} (${(e as Error).message})`);
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
    return Buffer.from(x as Uint8Array).toString("utf-8");
  }
  return String(x);
}

function candidates(ref: string): string[] {
  const r = ref.replace(/\\/g, "/").replace(/^\/+/, "");
  return ["/" + r, "\\" + r, r];
}

function listMdxFiles(): string[] {
  const out: string[] = [];
  const walkDir = (dir: string): void => {
    let entries: fs.Dirent[];
    try { entries = fs.readdirSync(dir, { withFileTypes: true }); } catch { return; }
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
    try { entries = fs.readdirSync(d, { withFileTypes: true }); } catch { return; }
    for (const e of entries.sort((a, b) => a.name.localeCompare(b.name))) {
      const p = path.join(d, e.name);
      if (e.isDirectory()) walkDir(p);
      else if (e.isFile() && e.name.toLowerCase().endsWith(".css")) out.push(p);
    }
  };
  walkDir(dir);
  return out.sort();
}

// ---- 多分卷 MDD ----
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
    const names = [base + ".mdd"];
    try {
      const re = new RegExp("^" + escapeRegExp(base) + "\\.\\d+\\.mdd$");
      for (const f of fs.readdirSync(dir).sort()) if (re.test(f)) names.push(f);
    } catch { /* ignore */ }
    const seen = new Set<string>();
    for (const n of names) {
      const p = path.join(dir, n);
      try {
        if (!fs.statSync(p).isFile()) continue;
        const rp = fs.realpathSync(p);
        if (!seen.has(rp)) { seen.add(rp); this.vols.push(p); }
      } catch { /* ignore */ }
    }
  }

  private mount(vol: string): any {
    if (this.mounted === vol) return this.inst;
    this.inst = getMdd(vol);
    this.mounted = vol;
    return this.inst;
  }

  lookup(ref: string, cands: (r: string) => string[]): Uint8Array | null {
    for (const vol of this.vols) {
      const inst = this.mount(vol);
      for (const cand of cands(ref)) {
        try {
          const res = inst.locate(cand);
          if (res && res.definition) {
            const buf = Buffer.from(String(res.definition), "base64");
            if (buf.length) return new Uint8Array(buf);
          }
        } catch { continue; }
      }
    }
    const suffix = "/" + String(ref).replace(/\\/g, "/").replace(/^\/+/, "");
    for (const vol of this.vols) {
      const inst = this.mount(vol);
      let keys: string[] = [];
      try { keys = (inst.keywordList ?? []).map((k: any) => String(k.keyText)); }
      catch { continue; }
      for (const k of keys) {
        if (k.replace(/\\/g, "/").endsWith(suffix)) {
          try {
            const res = inst.locate(k);
            if (res && res.definition) {
              const buf = Buffer.from(String(res.definition), "base64");
              if (buf.length) return new Uint8Array(buf);
            }
          } catch { continue; }
        }
      }
    }
    return null;
  }

  allKeys(): string[] {
    const out: string[] = [];
    for (const vol of this.vols) {
      try {
        const inst = this.mount(vol);
        for (const k of inst.keywordList ?? []) out.push(String(k.keyText));
      } catch { /* ignore */ }
    }
    return out;
  }
}

const _mddPool = new Map<string, MultiMDD>();

function multiMddLookup(mdx: string, ref: string, cands: (r: string) => string[]): Uint8Array | null {
  return getMultiMDD(mdx).lookup(ref, cands);
}

function getMultiMDD(mdx: string): MultiMDD {
  let pool = _mddPool.get(mdx);
  if (!pool) { pool = new MultiMDD(mdx); _mddPool.set(mdx, pool); }
  return pool;
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
        (base.includes("__") && ["gbs", "uss", "brs", "ams"].some((t) => base.includes(t)))
      ) continue;
      seen.add(ref);
      refs.push(ref);
    }
  }
  return refs;
}

export function lookup(word: string, bold = true): any {
  const mdxFiles = listMdxFiles();
  const r: any = {
    word, found: false, text: null, audio_refs: [] as string[],
    dict: null, _mdx: null, _html: null, css: "",
  };
  if (!mdxFiles.length) return r;
  for (const mdx of mdxFiles) {
    const builder = getBuilder(mdx);
    let def: string | null = null;
    try {
      const res = builder.lookup(word);
      def = res && res.definition ? String(res.definition) : null;
      if (!def) {
        const res2 = builder.lookup(word.toLowerCase());
        def = res2 && res2.definition ? String(res2.definition) : null;
      }
    } catch {
      purgeIndexCache(mdx);
      builders.delete(mdx);
      try {
        const fresh: any = new MDX(mdx);
        saveIndexCache(fresh, mdx);
        builders.set(mdx, fresh);
        const res = fresh.lookup(word);
        def = res && res.definition ? String(res.definition) : null;
      } catch { continue; }
    }
    if (!def) continue;
    const html = toStr(def).replace(/\x00+$/, "").replace(/\s+$/, "");
    r.found = true;
    r.dict = path.basename(mdx);
    r._mdx = mdx;
    r.text = render(stripDup(html), 76, bold);
    const cssParts: string[] = [];
    for (const cp of listCssFiles(path.dirname(mdx))) {
      try { cssParts.push(fs.readFileSync(cp, "utf8")); } catch { /* ignore */ }
    }
    r.css = cssParts.join("\n\n");
    r._html = stripDup(html);
    let hw = headwordChn(html);
    if (hw) {
      hw = hw.replace(/（[^）]*）|\([^)]*\)/g, "")
        .replace(/^[ ；;，,]+/, "").replace(/[ ；;，,]+$/, "");
      if (hw.length > 30) hw = hw.slice(0, 30) + "…";
    }
    if (hw) {
      const lines = String(r.text).split("\n");
      lines[0] = lines[0].replace(/\s+$/, "") + " 【" + hw + "】";
      r.text = lines.join("\n");
    }
    r.audio_refs = parseAudioRefs(html);
    break;
  }
  return r;
}

export function extractAudio(mdx: string, ref: string): string | null {
  ref = String(ref).replace(/\\/g, "/").trim().replace(/^\/+/, "");
  let data = multiMddLookup(mdx, ref, candidates);
  if (!data) {
    const base = path.basename(ref).toLowerCase();
    let keys: string[] = [];
    try { keys = getMultiMDD(mdx).allKeys(); } catch { keys = []; }
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
  if (fs.existsSync(out) && fs.statSync(out).size === data.byteLength) return out;
  try {
    for (const f of fs.readdirSync(AUDIO_DIR)) {
      if (f !== name) { try { fs.rmSync(path.join(AUDIO_DIR, f), { force: true }); } catch { /* ignore */ } }
    }
  } catch { /* ignore */ }
  fs.writeFileSync(out, data);
  return out;
}

export function getPlayable(word: string, variant: "gb" | "us"): string | null {
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
    } catch { continue; }
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

/** 按精确候选 → 全卷 basename 兜底, 在所有 MDD 卷里找资源 */
export function dicresFind(ref: string): Uint8Array | null {
  for (const mdx of listMdxFiles()) {
    const data = multiMddLookup(mdx, ref, candidates);
    if (data) return data;
  }
  const base = path.basename(ref.replace(/\\/g, "/")).toLowerCase();
  for (const mdx of listMdxFiles()) {
    let keys: string[] = [];
    try { keys = getMultiMDD(mdx).allKeys(); } catch { continue; }
    for (const k of keys) {
      if (path.basename(k.replace(/\\/g, "/")).toLowerCase() === base) {
        const data = multiMddLookup(mdx, k, candidates);
        if (data) return data;
      }
    }
  }
  return null;
}

/** 进程退出时释放 scanner。 */
export function closeAllBuilders(): void {
  for (const inst of [...builders.values(), ..._mddCache.values()]) {
    try { inst?.scanner?.close?.(); } catch { /* ignore */ }
  }
}
