/** 单个单词才走本地词典；短语/句子走在线站点。 */
export function isSingleWord(text: string): boolean {
  text = (text || "").trim();
  if (!text || text.split(/\s+/).length !== 1) return false;
  return !/[.!?,;:?。！？，、；：！？]/.test(text);
}

/** 是否含中文（CJK 统一表意文字，含扩展 A 区与兼容表意文字）。 */
export function isChinese(text: string): boolean {
  return /[\u3400-\u4dbf\u4e00-\u9fff\uf900-\ufaff]/.test(text || "");
}

export function escHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

export function e2(s: string, quote = false): string {
  let out = s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
  if (quote) out = out.replace(/"/g, "&quot;").replace(/'/g, "&#39;");
  return out;
}

export function playAudio(audioPath: string): boolean {
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
      Bun.spawn(args, { stdout: "ignore", stderr: "ignore", stdin: "ignore" });
      return true;
    } catch {
      /* ignore */
    }
  }
  return false;
}
