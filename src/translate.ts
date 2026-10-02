const BING_URL = "https://edge.microsoft.com/translate/translatetext";
const EDGE_UA =
  "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 " +
  "(KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 Edg/131.0.0.0";

export async function translateText(text: string, targetLang = "zh-Hans"): Promise<string> {
  if (!text || !text.trim()) return "";
  const url = `${BING_URL}?isEnterpriseClient=false&to=${targetLang}`;
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
    "User-Agent": EDGE_UA,
  };
  try {
    const resp = await fetch(url, {
      method: "POST", headers,
      body: JSON.stringify([text]),
      signal: AbortSignal.timeout(15000),
    });
    if (!resp.ok) return `网络请求失败: HTTP ${resp.status}`;
    const result = await resp.json();
    if (Array.isArray(result) && result.length && result[0] && "translations" in result[0]) {
      return String(result[0].translations[0].text ?? "").trim();
    }
    return "解析响应失败";
  } catch (e) {
    return `网络请求失败: ${(e as Error).message}`;
  }
}
