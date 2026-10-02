import * as fs from "node:fs";
import * as path from "node:path";
import {
  attrOf, delAttr, iterAll, parseHtml, serializeDoc, setAttr, tagOf,
} from "./dom";
import { DICT_NAME, logErr } from "./paths";
import { type DicCfg, cfgInt, loadCfg, saveCfgMerged } from "./config";
import { escHtml, isChinese, isSingleWord, playAudio } from "./utils";
import { lookup, extractAudio, getPlayable, dicresFind, closeAllBuilders } from "./mdict";
import { translateText } from "./translate";
import { onlineStackedHtml, onlineSiteNames, onlineSiteUrls } from "./online";
import { INIT_SCRIPT, OXFORD9_CSS, webShell } from "./shell";

const RES_SKIP = /^(https?:|data:|file:|about:|\/\/)/i;
const CSS_URL_RE = /url\(\s*(['"]?)([^'")]+)\1\s*\)/g;

function cssLocalize(cssText: string, resBase: string): string {
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

export async function runGuiWeb(
  initialText?: string,
  opts?: { headless?: boolean },
): Promise<void> {
  logErr(`webview pid=${process.pid}`);
  const initial = (initialText || "").trim();

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
    text = (text || "").trim();
    state.last_query = text;
    state.online = false;
    return { online: true, html: onlineStackedHtml(text), css: "" };
  }

  function prepare(htmlText: string, resBase: string): string {
    htmlText = htmlText.replaceAll("<xhtml:", "<").replaceAll("</xhtml:", "</");
    htmlText = htmlText.replace(/<script\b[\s\S]*?<\/script\s*>/gi, "");
    htmlText = htmlText.replace(/<script\b[^>]*\/?>/gi, "");
    htmlText = htmlText.replace(/\s+on[a-z]+\s*=\s*("[^"]*"|'[^']*'|[^\s>]+)/gi, "");
    htmlText = htmlText.replace(/\s+unselectable\s*=\s*['"]?(on|true)['"]?/gi, "");
    htmlText = htmlText.replace(
      /(style\s*=\s*['"][^'"]*?)user-select\s*:\s*none\s*;?/gi, "$1",
    );
    const root = parseHtml(htmlText);
    if (root) {
      const snd = /^sound:\/\/|\.mp3(\?|#|$)/i;
      for (const el of [...iterAll(root)]) {
        const tag = tagOf(el);
        if (tag === "img") {
          const v = (attrOf(el, "src") || "").trim();
          if (v && !RES_SKIP.test(v)) {
            setAttr(el, "src",
              `${resBase}/res/` + v.replace(/\\/g, "/").replace(/^\/+/, ""));
          }
          delAttr(el, "srcset");
        }
        let ref = "";
        for (const attr of Object.keys({ ...el.attribs })) {
          const a = attr.toLowerCase();
          if (!["href", "src", "data-src-mp3", "src-mp3", "data-sound"].includes(a)) continue;
          const v = (attrOf(el, attr) || "").trim();
          if (a === "data-sound") ref = ref || v;
          else if (snd.test(v) || a === "data-src-mp3" || a === "src-mp3") {
            if (!ref) ref = v.replace(/^sound:\/\//i, "");
            delAttr(el, attr);
          }
        }
        if (ref) setAttr(el, "data-sound", ref);
        if (tag === "a") {
          for (const junk of ["href", "target", "onclick"]) delAttr(el, junk);
        }
      }
      for (const el of [...iterAll(root)]) {
        const ref = attrOf(el, "data-sound");
        if (!ref) continue;
        const p = el.parent as any;
        if (!p || attrOf(p, "data-sound")) continue;
        if (!(attrOf(p, "class") || "").toLowerCase().includes("pron")) continue;
        const refs = new Set<string>();
        for (const s of iterAll(p)) {
          const v = attrOf(s, "data-sound");
          if (v) refs.add(v);
        }
        if (refs.size === 1) setAttr(p, "data-sound", ref);
      }
      for (const a of [...iterAll(root)]) {
        if (tagOf(a) !== "a") continue;
        let p = a.parent as any;
        while (p) {
          if (tagOf(p) === "a") { a.name = "span"; break; }
          p = p.parent;
        }
      }
      htmlText = serializeDoc(root);
    }
    htmlText = htmlText.replace(/\s+href\s*=\s*["']?sound:\/\/[^\s>"']*/gi, "");
    htmlText = htmlText.replace(/\s+src\s*=\s*["']?sound:\/\/[^\s>"']*/gi, "");
    return htmlText;
  }

  function dump(htmlText: string, css: string): void {
    if (!process.env.DIC_DUMP) return;
    try {
      fs.writeFileSync("/tmp/plastic-dict/last.html", htmlText);
      fs.writeFileSync("/tmp/plastic-dict/last.css", css);
    } catch { /* ignore */ }
  }

  async function apiLookup(word: string): Promise<any> {
    word = (word || "").trim();
    if (!word) return { html: "", css: "" };
    // [中文] 中文输入 或 非单词 → 直接走在线词典站
    if (isChinese(word) || !isSingleWord(word)) return gotoOnline(word);
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
    return { html: htmlText, css };
  }

  function playLog(...a: unknown[]): void {
    try {
      fs.appendFileSync(
        "/tmp/plastic-dict/play.log",
        new Date().toTimeString().slice(0, 8) + " " + a.map(String).join(" ") + "\n",
      );
    } catch { /* ignore */ }
  }

  async function apiPlay(ref: string): Promise<string> {
    const word = state.word || "";
    logErr("[plastic-dict.play]", ref, "| word:", word);
    playLog("play called:", JSON.stringify(ref), "| word:", JSON.stringify(word));
    if (!state.mdx || !ref) return "⛔ play: 内部状态缺失";
    ref = String(ref).trim().replace(/^[a-z][a-z0-9+.-]+:\/\//i, "");
    let audioPath: string | null = null;
    try { audioPath = extractAudio(state.mdx, ref); }
    catch (e) { playLog("提取异常:", String(e)); }
    if (!audioPath) {
      playLog(`未命中 ${JSON.stringify(ref)}, 词条兜底: ${JSON.stringify(word)}`);
      for (const variant of ["gb", "us"] as const) {
        try { audioPath = word ? getPlayable(word, variant) : null; }
        catch { audioPath = null; }
        if (audioPath) { playLog("兜底命中:", variant); break; }
      }
    }
    if (!audioPath) { playLog("最终无音频:", ref); return `⛔ 找不到音频: ${ref}`; }
    const played = playAudio(audioPath);
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
  try { zoom0 = Math.min(3.0, Math.max(0.5, Number(cfgd.zoom || 1.0))); } catch { zoom0 = 1.0; }
  const w0 = cfgInt(cfgd, "width", 760, 320, 7680);
  const h0 = cfgInt(cfgd, "height", 860, 300, 4320);

  // ---- 本地资源/页面服务 ----
  let serverPort = 0;
  let server: ReturnType<typeof Bun.serve> | null = null;

  async function handleReq(req: Request): Promise<Response> {
    const u = new URL(req.url);
    const p = u.pathname;
    try {
      if (p === "/" || p === "/entry") {
        const initialQ = (u.searchParams.get("q") || "").trim();
        const auto = u.searchParams.get("auto");
        let body = "";
        let word0 = "";
        let css0 = OXFORD9_CSS;
        const resBase = `http://127.0.0.1:${serverPort}`;
        // [中文] 只有“英文单词”才走本地词典，中文/短语/句子走在线
        if (initialQ && isSingleWord(initialQ) && !isChinese(initialQ)) {
          word0 = initialQ;
          const r = lookup(initialQ);
          if (r.found) {
            state.mdx = r._mdx;
            state.word = r.word;
            body = prepare(r._html, resBase);
            css0 = cssLocalize(r.css, resBase) + "\n" + OXFORD9_CSS;
            dump(body, css0);
          } else {
            body = `<p>${escHtml(await translateText(initialQ))}</p>`;
          }
        } else if (initialQ) {
          state.last_query = initialQ;
          state.online = false;
          body = onlineStackedHtml(initialQ);
        }
        if (auto && word0) {
          body += '<script>(function w(){if(window.dicApi){try{doLookup();}catch(e){}}else{setTimeout(w,200);}})();</script>';
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
              n: onlineSiteNames()[i] ?? uu, u: uu,
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
            headers: { "Content-Type": MIME_BY_EXT[ext] ?? "application/octet-stream" },
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

  server = Bun.serve({ hostname: "127.0.0.1", port: 0, fetch: handleReq });
  serverPort = server.port ?? 0;
  const baseUrl = `http://127.0.0.1:${serverPort}`;

  if (opts?.headless) {
    console.log(`[plastic-dict] headless server: ${baseUrl}`);
    await new Promise<never>(() => {});
    return;
  }

  const wv = new Webview(true, { width: w0, height: h0 });
  wv.title = DICT_NAME;

  let running = true;
  let windowGone = false;
  wv.bind("dicApi", (method: string, ...args: any[]) => {
    return (async () => {
      switch (method) {
        case "play": return apiPlay(String(args[0] ?? ""));
        case "onlineOpen": {
          let idx = parseInt(String(args[0] ?? "0"), 10);
          if (Number.isNaN(idx)) idx = 0;
          const urls = onlineSiteUrls(state.last_query);
          if (!urls.length) return "⛔ 无查询内容";
          if (idx < 0 || idx >= urls.length) idx = 0;
          state.online = true;
          try { wv.navigate(urls[idx]); return "🌐 " + onlineSiteNames()[idx]; }
          catch (e) { return `⛔ 打开失败: ${(e as Error).message}`; }
        }
        case "onlineHome": {
          state.online = false;
          const w = state.word || "";
          const auto = w ? "1" : "";
          wv.navigate(`${baseUrl}/entry?q=${encodeURIComponent(w)}${auto ? "&auto=1" : ""}`);
          return true;
        }
        case "close": running = false; return true;
        case "saveCfg":
          try { saveCfgMerged((args[0] ?? {}) as DicCfg); } catch { /* ignore */ }
          return true;
        case "getZoom": return zoom0;
        case "onlineBarData": {
          const sites = state.online
            ? onlineSiteUrls(state.last_query).map((uu, i) => ({
                n: onlineSiteNames()[i] ?? uu, u: uu,
              }))
            : [];
          return { sites, q: state.online ? state.last_query : "" };
        }
        case "devtools": return "请右键页面 → 检查元素 (WebInspector)";
        default: return { error: "unknown method: " + method };
      }
    })();
  });

  wv.init(INIT_SCRIPT);

  const url0 = initial
    ? `${baseUrl}/entry?q=${encodeURIComponent(initial)}`
    : `${baseUrl}/entry?q=`;
  wv.navigate(url0);

  const { dlopen, FFIType } = await import("bun:ffi");
  let glib: any = null;
  let gtk4: any = null;
  try {
    glib = dlopen("libglib-2.0.so.0", {
      g_main_context_iteration: { args: [FFIType.ptr, FFIType.i32], returns: FFIType.i32 },
    });
  } catch (e) {
    console.error("无法加载 libglib-2.0.so.0 (系统 webview 依赖), 请安装 GTK4 运行时:",
      (e as Error).message);
    process.exit(1);
  }
  try {
    gtk4 = dlopen("libgtk-4.so.1", {
      gtk_window_present: { args: [FFIType.ptr], returns: FFIType.void },
      gtk_window_maximize: { args: [FFIType.ptr], returns: FFIType.void },
      gtk_widget_get_visible: { args: [FFIType.ptr], returns: FFIType.i32 },
    });
  } catch { gtk4 = null; }

  const gtkw = wv.window;
  try {
    if (gtkw && gtk4) {
      gtk4.symbols.gtk_window_present(gtkw);
      if (cfgd.maximized) gtk4.symbols.gtk_window_maximize(gtkw);
    }
  } catch (e) { logErr("窗口呈现失败:", (e as Error).message); }

  try {
    while (running) {
      glib.symbols.g_main_context_iteration(null, 0);
      if (gtk4 && gtkw && !windowGone) {
        let visible = 1;
        try { visible = gtk4.symbols.gtk_widget_get_visible(gtkw); } catch { visible = 1; }
        if (!visible) { windowGone = true; break; }
      }
      await Bun.sleep(2);
    }
  } finally {
    try { server?.stop(true); } catch { /* ignore */ }
    closeAllBuilders();
    if (!windowGone) { try { wv.destroy(); } catch { /* ignore */ } }
  }
}
