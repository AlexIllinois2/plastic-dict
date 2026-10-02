import * as fs from "node:fs";
import * as path from "node:path";
import { CFG_PATH } from "./paths";

export interface DicCfg {
  zoom?: number;
  width?: number;
  height?: number;
  maximized?: boolean;
  [k: string]: unknown;
}

export function loadCfg(): DicCfg {
  try {
    const data = JSON.parse(fs.readFileSync(CFG_PATH, "utf8"));
    return data && typeof data === "object" ? data : {};
  } catch {
    return {};
  }
}

export function saveCfgMerged(extra: DicCfg): void {
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

export function cfgInt(
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
