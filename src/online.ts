import { e2 } from "./utils";

export const ONLINE_SITES = [
  "http://dict.youdao.com/w/eng/%GDWORD%",
  "https://www.bing.com/dict/search?q=%GDWORD%",
];

export function onlineSiteUrls(text: string): string[] {
  const q = encodeURIComponent((text || "").trim());
  return ONLINE_SITES.filter((t) => t.includes("%GDWORD%"))
    .map((t) => t.replace("%GDWORD%", q));
}

export function onlineSiteNames(): string[] {
  const names: string[] = [];
  for (const t of ONLINE_SITES) {
    let host = "";
    try { host = new URL(t).hostname.toLowerCase(); } catch { host = t.toLowerCase(); }
    let name: string | null = null;
    for (const [k, v] of [
      ["youdao", "有道"], ["baidu", "百度翻译"], ["bing", "必应"],
      ["haici", "海词"], ["cambridge", "剑桥"], ["merriam", "韦氏"],
      ["collins", "柯林斯"], ["oxford", "牛津"],
    ] as const) {
      if (host.includes(k)) { name = v; break; }
    }
    names.push(name ?? host);
  }
  return names;
}

export function onlineStackedHtml(query: string): string {
  const names = onlineSiteNames();
  const urls = onlineSiteUrls(query || "");
  const rows: string[] = [];
  urls.forEach((u, i) => {
    rows.push(
      '<div class="site"><div class="hd">' +
        `<iframe id="f${i}" src="${e2(u, true)}" referrerpolicy="no-referrer"></iframe></div>`,
    );
  });
  return (
    '<div id="stack-nav">' +
    '<button onclick="window.scrollTo(0,0)" title="回顶部">⇈</button>' +
    '<button onclick="window.scrollBy(0,-Math.round(window.innerHeight*0.9))" title="上一站/Ctrl+↑">▲</button>' +
    '<button onclick="window.scrollBy(0,Math.round(window.innerHeight*0.9))" title="下一站/Ctrl+↓">▼</button>' +
    '<button onclick="window.scrollTo(0,document.body.scrollHeight)" title="到底部">⇟</button>' +
    "</div>" +
    "<style>" +
    "#stack-nav{position:fixed;right:10px;top:50%;transform:translateY(-50%);" +
    "display:flex;flex-direction:column;gap:6px;z-index:2147483000;}" +
    "#stack-nav button{width:38px;height:38px;font-size:16px;" +
    "border:1px solid #c5cfdd;border-radius:8px;background:rgba(255,255,255,.95);" +
    "cursor:pointer;box-shadow:0 1px 4px rgba(0,0,0,.18);}" +
    "#stack-nav button:active{background:#e8eef7;}" +
    ".site{margin:0 0 10px 0;}" +
    ".site .hd{display:flex;gap:8px;align-items:center;padding:8px 14px;" +
    "background:#eef3fa;border-top:1px solid #dbe5f0;" +
    "border-bottom:1px solid #dbe5f0;font-size:14px;}" +
    ".site .hd .u{color:#888;font-size:12px;overflow:hidden;" +
    "text-overflow:ellipsis;white-space:nowrap;flex:1;}" +
    ".site .hd button{padding:2px 8px;font-size:12px;cursor:pointer;}" +
    ".site iframe{width:100%;height:82vh;border:0;display:block;background:#fff;}" +
    "</style>" +
    rows.join("")
  );
}
