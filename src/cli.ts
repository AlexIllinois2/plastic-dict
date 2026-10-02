import { translateText } from "./translate";
import { getPlayable, lookup } from "./mdict";
import { isChinese, isSingleWord } from "./utils";
import { runGuiWeb } from "./gui";

export async function runCli(content: string): Promise<number> {
  content = (content || "").trim();
  if (!content) return 1;

  // [中文] 中文输入 → 直接打开在线词典窗口
  if (isChinese(content)) {
    console.log("🌐 中文输入: 打开在线词典窗口…");
    await runGuiWeb(content);
    return 0;
  }
  // 短语/句子 → 打开在线词典窗口
  if (!isSingleWord(content)) {
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
