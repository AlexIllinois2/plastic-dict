#!/usr/bin/env bun
/**
 * plastic-dict — 单文件整合版（Bun + TypeScript 重写版）模块化后主入口
 * 逻辑与原来一致；新增：中文输入自动走在线词典站。
 *
 * 用法:
 *   ./plastic-dict.ts "<content>"     自动: 英文单词→WebView 本地词典; 中文/短语/句子→WebView 在线词典
 *   ./plastic-dict.ts -t "<content>"  命令行查词/翻译 (中文/短语时弹 WebView 在线词典窗口)
 *   ./plastic-dict.ts -w "<content>"  强制 WebView (与默认一致, 保留参数兼容)
 *   ./plastic-dict.ts --purge-cache   清空词典索引磁盘缓存
 */

import * as fs from "node:fs";
import { runGuiWeb } from "./src/gui";
import { runCli } from "./src/cli";
import { purgeAllCache } from "./src/mdict";
import { CRASH_LOG, INDEX_CACHE_DIR } from "./src/paths";

function printUsage(): number {
  console.log(
    [
      "用法:",
      '  ./plastic-dict.ts "<content>"   英文单词→WebView 本地词典; 中文/短语/句子→WebView 在线词典',
      '  ./plastic-dict.ts -g "<content>" 同上',
      '  ./plastic-dict.ts -w "<content>" 强制 WebView (与默认一致)',
      '  ./plastic-dict.ts -t "<content>" 命令行模式 (中文/短语时弹 WebView 在线词典窗口)',
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
    if (!content) { console.error('用法: ./plastic-dict.ts -g "<content>"'); return 1; }
    await runGuiWeb(content);
    return 0;
  }
  if (args[0] === "-w" || args[0] === "--webview") {
    const content = args.slice(1).join(" ").trim();
    if (!content) { console.error('用法: ./plastic-dict.ts -w "<content>"'); return 1; }
    await runGuiWeb(content);
    return 0;
  }
  if (args[0] === "-t" || args[0] === "--text") {
    const content = args.slice(1).join(" ").trim();
    if (!content) { console.error('用法: ./plastic-dict.ts -t "<content>"'); return 1; }
    return runCli(content);
  }
  if (args[0] === "-h" || args[0] === "--help") return printUsage();

  const content = args.join(" ").trim();
  if (!content) return 1;
  await runGuiWeb(content);
  return 0;
}

process.on("uncaughtException", (err) => {
  try {
    fs.appendFileSync(CRASH_LOG, `[${new Date().toISOString()}] ${err.stack ?? err}\n`);
  } catch { /* ignore */ }
  throw err;
});

if (import.meta.main) {
  main()
    .then((code) => process.exit(code))
    .catch((err) => {
      try {
        fs.appendFileSync(CRASH_LOG, `[${new Date().toISOString()}] ${err?.stack ?? err}\n`);
      } catch { /* ignore */ }
      console.error(err);
      process.exit(1);
    });
}
