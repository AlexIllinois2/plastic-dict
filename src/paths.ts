import * as path from "node:path";
import * as os from "node:os";

export function expandUser(p: string): string {
  if (p === "~") return os.homedir();
  if (p.startsWith("~/")) return path.join(os.homedir(), p.slice(2));
  return p;
}

export const DICT_DIR = expandUser(
  process.env.GOLDENDICT_DIR || "~/.local/share/goldendict/Oxford9",
);
export const AUDIO_DIR = "/tmp/plastic-dict/audio";
export const INDEX_CACHE_DIR = path.join(os.homedir(), ".cache", "plastic-dict", "index");
export const CFG_PATH = path.join(os.homedir(), ".config", "plastic-dict", "config.json");
export const DICT_NAME = "plastic-dict";
export const CRASH_LOG = "/tmp/plastic-dict/crash.log";

export function logErr(...a: unknown[]) {
  console.error("[plastic-dict]", ...a);
}
