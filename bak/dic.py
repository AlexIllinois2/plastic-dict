#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
dic — 单文件整合版（GUI + CLI + 本地词典 + Bing 在线翻译 + WebView 在线词典站）

用法:
  ./dic "<content>"     自动: 单词→WebView 本地词典; 短语/句子→WebView 在线词典
  ./dic -t "<content>"  命令行查词/翻译 (短语时弹 WebView 在线词典窗口)
  ./dic -w "<content>"  强制 WebView (auto_switch=False)
  ./dic --native "<content>"  强制原生 Qt 窗口

快捷键: bash -lc '$HOME/.local/script/dic "$(/usr/bin/wl-paste -n -p)"'

依赖安装见文末注释; 非单词输入会在同一 WebView 窗口内打开 ONLINE_SITES
(GoldenDict 风格 %GDWORD% 模板), 页面顶部有站点切换工具条。

# 依赖 (Linux / deb + dnf)
# Fedora/RHEL: sudo dnf install python3-lxml python3-gobject gtk3 webkit2gtk4.1 \
#   python3-requests python3-tkinter google-noto-sans-cjk-ttc-fonts pulseaudio-utils
# Debian/Ubuntu: sudo apt install python3-lxml python3-gi gir1.2-gtk-3.0 \
#   gir1.2-webkit2-4.1 python3-requests python3-tk fonts-noto-cjk pulseaudio-utils
# 注: 老发行版把 webkit2gtk4.1 换成 4.0; CJK 字体不装中文会变豆腐块
# pip: pip3 install --user 'pywebview>=4' PySide6 FreeSimpleGUI FreeSimpleGUIQt mdict-mquery
#   (报 externally-managed-environment 加 --break-system-packages)
# 音频: mpv / ffplay / cvlc / paplay 任一即可
# 离线词典: https://github.com/yanyingwang/goldendict 的百度网盘 dicts/Oxford9 移到 ~/.local/share/golden-dict/Oxford
"""

import sys, os, re, glob, shutil, sqlite3, platform, subprocess
import time
import requests
from pathlib import Path

# ============================================================
# 渲染层（原 render.py）
# ============================================================
from lxml import html as lh

NL, IND = '\x00', '\x02'  # 有意换行/缩进哨兵, 空白清洗后再还原

NOISE = {
    'script', 'style', 'link', 'head', 'symbol', 'xsymb', 'img', 'hkey',
    'topic', 'ftindex', 'fthzmark', 'fthzindex', 'xhtml', 'sdsymb',
    'audio-wr', 'audio', 'audio-gbs-liju', 'audio-uss-liju',
    'audio-brs-liju', 'audio-ams-liju', 'un',
}

# 在 NOISE 基础上追加要丢弃的交叉引用/同义词噪音
RENDER_NOISE_EXTRA = {'xr-g', 'xr-gs', 'cf-blk', 'cf', 'syn-g-blk', 'syn-g'}

BOLD = {'h', 'idm', 'pv', 'shcut', 'cf'}

RESET = {
    'pos-g': True,
    'pv-blk': False,
    'pv-g-blk': False,
    'idm-blk': False,
    'idm-gs-blk': False,
    'subentry-g': False,
    'boxblock': False,
}

POS_CN = {
    'verb': '动词', 'noun': '名词', 'adjective': '形容词', 'adverb': '副词',
    'exclamation': '感叹词', 'preposition': '介词', 'conjunction': '连词',
    'pronoun': '代词', 'determiner': '限定词', 'number': '数词',
    'modal verb': '情态动词', 'auxiliary verb': '助动词',
    'v.': '动词', 'n.': '名词', 'adj.': '形容词', 'adv.': '副词',
    'excl.': '感叹词', 'prep.': '介词', 'conj.': '连词', 'pron.': '代词',
    'det.': '限定词', 'num.': '数词', 'modal v.': '情态动词',
    'aux. v.': '助动词',
}

POS = {}

INFL_CN = [
    ('past participle', '过去分词'),
    ('past simple', '过去式'),
    ('-ing form', '现在分词'),
    ('present simple - he', '三单'),
    ('present simple', '原形'),
    ('plural', '复数'),
    ('third person', '三单'),
]

RENDER_NOISE_EXTRA = {
    'xr-g', 'xr-gs', 'cf-blk', 'cf', 'syn-g-blk', 'syn-g', 'syn-gs',
    'lb-g', 'lb', 'lmb',  # language bank 语料库
    'symbol', 'un', 'unx-g',
}

# 正文行走时直接跳过的标签(内容已单独提取或纯噪音)
_SKIP_TAGS = {'h', 'phon', 'pos', 'brelabel', 'namelabel', 'pron-g',
              'vpform', 'infl', 'v-g'}


def _norm(tag):
    if not isinstance(tag, str):
        return ''
    return tag[7:] if tag.startswith('xhtml:') else tag


def _norm_phon(p):
    """音标补斜杠: 这个包里斜杠是独立节点, phon 取出来是裸 IPA"""
    p = (p or '').strip()
    if p and not p.startswith('/'):
        p = '/' + p.strip('/') + '/'
    return p


def _infl_cn(name):
    low = name.lower()
    for k, v in INFL_CN:
        if k in low:
            return v
    return name


def render(html_text, width=76, bold=True):
    root = lh.fromstring(html_text)

    # ---- 1. 提取词形变化表: 以 vpform 为锚, 父节点即一条变形记录 ----
    infl_rows = []
    for vp in list(root.iter()):
        if _norm(vp.tag) != 'vpform':
            continue
        p = vp.getparent()
        if p is None:
            continue
        form = (vp.text_content() or '').strip()
        hw = ph = ''
        for el in p.iter():
            t = _norm(el.tag)
            if t == 'h' and not hw:
                hw = (el.text_content() or '').strip()
            elif t == 'phon' and not ph:
                ph = (el.text_content() or '').strip()
        if form and (hw or ph):
            infl_rows.append((_infl_cn(form), hw, _norm_phon(ph)))
        try:
            p.drop_tree()
        except Exception:
            pass

    # ---- 2. 清噪音 ----
    noise = set(NOISE) | RENDER_NOISE_EXTRA
    for el in list(root.iter()):
        t = _norm(el.tag)
        if t in noise:
            try:
                el.drop_tree()
            except Exception:
                pass
        elif t == 'div' and 'cixing_tiaozhuan' in (el.get('class') or ''):
            try:
                el.drop_tree()
            except Exception:
                pass

    for tag in ('audio-gb', 'audio-us', 'pron-g', 'audio'):
        for el in list(root.iter(tag)):
            try:
                el.drop_tag()
            except Exception:
                pass

    # ---- 3. 词头 + 音标 (只取第一个 top-g) ----
    word, prons = '', []
    for topg in root.iter():
        if _norm(topg.tag) != 'top-g':
            continue
        label = None
        for el in topg.iter():
            t = _norm(el.tag)
            cls = (el.get('class') or '')
            if t == 'h' and not word:
                word = (el.text_content() or '').strip()
            elif t == 'brelabel' or 'bre' in cls.split():
                label = 'BrE'
            elif t == 'namelabel' or 'name' in cls.split():
                label = 'NAmE'
            elif t == 'phon' and label:
                ph = _norm_phon((el.text_content() or '').strip())
                if ph and (label, ph) not in prons:
                    prons.append((label, ph))
        break

    head = []
    if word:
        head.append('单词：%s' % word)
    if prons:
        head.append('音标：' + ' ｜ '.join(
            '%s %s' % (lab, ph) for lab, ph in prons))
    if infl_rows:
        head.append('词形变化：')
        for name, w, ph in infl_rows:
            head.append('  %s：%s%s' % (name, w, (' ' + ph) if ph else ''))

    # ---- 4. 正文行走 ----
    out_lines = []
    cur = {'ind': 4, 'chunks': [], 'chn': []}
    st = {'sn': 0, 'num': True, 'top': 0}
    sections = set()  # 已打印过的栏目标题, 防重复

    def B(t):
        return '\033[1m' + t + '\033[0m' if bold else t

    def add(s):
        cur['chunks'].append(s)

    def flush():
        text = re.sub(r'\s+', ' ', ''.join(cur['chunks'])).strip()
        if cur['chn']:
            cn = '；'.join(cur['chn'])
            text = cn + (' ' + text if text else '')
        if text:
            out_lines.append(' ' * cur['ind'] + text)
        cur['chunks'], cur['chn'] = [], []

    def blank():
        if out_lines and out_lines[-1] != '':
            out_lines.append('')

    def walk(el):
        tag = _norm(el.tag)
        if tag in noise:
            return
        # 斜杠等纯符号节点(独立 span), 跳过
        cls = (el.get('class') or '').split()
        if any('slash' in c for c in cls):
            return
        if tag == 'top-g':
            st['top'] += 1
            if st['top'] > 1:
                flush()
                blank()
            for c in el:
                walk(c)
            flush()
            return
        if tag in _SKIP_TAGS:
            return
        if tag == 'br':
            flush()
            return
        if tag in RESET:
            st['sn'], st['num'] = 0, RESET[tag]
        if tag == 'pos-g':
            p = ''
            for c in el.iter():
                if _norm(c.tag) == 'pos':
                    p = (c.text_content() or '').strip().lower()
                    break
            flush()
            blank()
            if p:
                key = POS.get(p, p)
                out_lines.append('%s (%s)：' % (POS_CN.get(key, key), key))
                st['sn'], st['num'], cur['ind'] = 0, True, 4
            return
        if tag == 'sn-g':
            flush()
            cur['ind'] = 4
            if st['num']:
                st['sn'] += 1
                add('%d. ' % st['sn'])
            # 注意: 不 return, 继续走子节点取释义/例句
        elif tag == 'shcut':  # 主题小标题
            t = (el.text_content() or '').strip()
            if t:
                flush()
                blank()
                out_lines.append('  ◆ ' + B(t))
                cur['ind'] = 4
            return
        elif tag == 'x-g-blk':
            flush()
            cur['ind'] = 6
        elif tag in ('pv-blk', 'pv-g-blk'):
            flush()
            blank()
            if tag == 'pv-blk' and 'pv' not in sections:
                out_lines.append('短语动词 (Phrasal Verbs)：')
                sections.add('pv')
            cur['ind'] = 6
            # 不 return, 继续走子节点
        elif tag in ('idm-blk', 'idm-gs-blk'):
            flush()
            blank()
            if 'idm' not in sections:
                out_lines.append('常用习语 (Idioms)：')
                sections.add('idm')
            cur['ind'] = 6
            # 不 return
        elif tag in ('pv', 'idm'):  # 短语/习语词条头
            t = (el.text_content() or '').strip()
            if t:
                flush()
                blank()
                out_lines.append('  · ' + B(t))
                cur['ind'] = 6
            return  # 头已用 text_content 提取, 跳过自身子树
        elif tag == 'x-g':  # 例句: 英文在前, 全译括号在后
            flush()
            xs, chns = [], []
            for sub in el.iter():
                t2 = _norm(sub.tag)
                if t2 == 'x':
                    xs.append((sub.text_content() or '').strip())
                elif t2 == 'chn':
                    chns.append((sub.text_content() or '').strip())
            eng = ' '.join(x for x in xs if x) or (el.text_content() or '').strip()
            cn = '；'.join(c for c in chns if c)
            cur['ind'] = 6
            add('· ' + eng + ((' （%s）' % cn) if cn else ''))
            flush()
            return
        elif tag == 'chn':  # 释义中文: 缓存, flush 时前置
            t = (el.text_content() or '').strip()
            if t:
                cur['chn'].append(t)
            return
        elif tag == 'gram':
            t = (el.text_content() or '').strip()
            if t:
                add('(%s) ' % t)
            return
        elif tag == 'gram-blk':
            for c in el:
                walk(c)
            return
        elif tag == 'li':
            flush()
            cur['ind'] += 2
            add('- ')
            for c in el:
                walk(c)
            flush()
            cur['ind'] -= 2
            return

        if el.text:
            add(el.text)
        for c in el:
            walk(c)
            if c.tail:
                add(c.tail)

    for c in root:
        walk(c)
    flush()

    # ---- 5. 组装 + 折行(保留缩进) ----
    parts = head + ([''] if head and out_lines else []) + out_lines
    out = []
    for line in parts:
        ind = re.match(r'^( *)', line).group(1)
        if len(line) <= width:
            out.append(line)
            continue
        words, buf = line.split(' '), ''
        for w in words:
            if buf and len(buf) + 1 + len(w) > width:
                out.append(buf)
                buf = ind + '    ' + w
            else:
                buf = (buf + ' ' + w) if buf else (ind + w)
        if buf:
            out.append(buf)
    return '\n'.join(out)


# ============================================================
# 多分卷 MDD 查询层（原 multimdd.py）
# ============================================================
class MultiMDD:
    def __init__(self, builder):
        self.builder = builder
        stem = os.path.splitext(builder._mdx_file)[0]
        seen, vols = set(), []
        # <词干>.mdd 与 <词干>.N.mdd, 按 realpath 去重(兼容符号链接)
        for p in [stem + ".mdd"] + sorted(glob.glob(glob.escape(stem) + ".*.mdd")):
            if os.path.isfile(p):
                rp = os.path.realpath(p)
                if rp not in seen:
                    seen.add(rp)
                    vols.append((p, p + ".db"))
        self.vols = vols
        self._mounted = None

    def _mount(self, vol):
        if self._mounted is vol:
            return
        mdd, db = vol
        b = self.builder
        b._mdd_file = mdd
        b._mdd_db = db
        if not os.path.isfile(db):
            print(f"  [构建] MDD 索引(一次性): {os.path.basename(db)}")
            b._make_mdd_index(db)
        self._mounted = vol

    def lookup(self, ref, candidates):
        # 第一轮: 精确候选, 逐卷尝试
        for vol in self.vols:
            self._mount(vol)
            for cand in candidates(ref):
                try:
                    res = self.builder.mdd_lookup(cand)
                except Exception:
                    continue
                if res:
                    v = res[0]
                    return v.encode() if isinstance(v, str) else bytes(v)
        # 第二轮: 后缀兜底, 每卷扫 get_mdd_keys()
        suffix = "/" + str(ref).replace("\\", "/").lstrip("/")
        for vol in self.vols:
            self._mount(vol)
            try:
                keys = self.builder.get_mdd_keys()
            except Exception:
                continue
            for k in keys:
                if isinstance(k, (tuple, list)) and k:
                    k = k[0]
                if isinstance(k, bytes):
                    k = k.decode("utf-8", "ignore")
                else:
                    k = str(k)
                if k.replace("\\", "/").endswith(suffix):
                    try:
                        res = self.builder.mdd_lookup(k)
                    except Exception:
                        continue
                    if res:
                        v = res[0]
                        return v.encode() if isinstance(v, str) else bytes(v)
        return None


_POOL = {}


def multi_mdd_lookup(builder, ref, candidates):
    key = id(builder)
    if key not in _POOL:
        _POOL[key] = MultiMDD(builder)
    return _POOL[key].lookup(ref, candidates)


# ============================================================
# 词典门面（原 dictapi.py）
# ============================================================
import mdict_mquery

if not hasattr(mdict_mquery, "version"):
    # 包内 bug: NameError: version
    mdict_mquery.version = getattr(mdict_mquery, "__version__", "unknown")

from mdict_mquery import IndexBuilder

DICT_DIR = os.path.expanduser(
    os.environ.get("GOLDENDICT_DIR", "~/.local/share/golden-dict/Oxford"))
AUDIO_DIR = "/tmp/dic/audio"
AUDIO_PATTERNS = [
    re.compile(r'sound://([^"\'\s>\\]+)'),
    re.compile(r'data-src-mp3=["\']([^"\']+)'),
    re.compile(r'href=["\']([^"\']*?\.mp3)'),
]

_BUILDERS = {}


def _get_builder(mdx):
    if mdx not in _BUILDERS:
        try:
            _BUILDERS[mdx] = IndexBuilder(mdx)
        except sqlite3.OperationalError:
            # 缓存损坏 → 清理重建
            _purge_generated(mdx)
            _BUILDERS[mdx] = IndexBuilder(mdx)
    return _BUILDERS[mdx]


def _purge_generated(m):
    """按后缀白名单清理包生成的索引/缓存, 绝不动 .mdx/.mdd 源文件"""
    stem = os.path.splitext(m)[0]
    whitelist = {"index", "db", "idx", "cache", "pickle", "pkl",
                 "keys", "sqlite", "sqlite3", "mquery"}
    for base in (m, stem):
        for p in sorted(glob.glob(base + ".*")):
            if p in (m, stem + ".mdx", stem + ".mdd"):
                continue
            if os.path.splitext(p)[1].lstrip(".").lower() in whitelist:
                try:
                    os.remove(p)
                except OSError:
                    pass


def _to_str(x):
    if isinstance(x, str):
        return x
    if isinstance(x, (bytes, bytearray)):
        for e in ("utf-8", "utf-16-le", "gb18030"):
            try:
                return bytes(x).decode(e)
            except UnicodeDecodeError:
                pass
    return str(x)


def _strip_dup(html):
    """OALD9 的 aunbox 是 unbox 的锚点副本(GoldenDict 靠 CSS 隐藏)。
    只删除确有相同文本 unbox 孪生的 aunbox, 孤立 aunbox 保留。"""
    try:
        root = lh.fromstring(html)
    except Exception:
        return html
    tag = lambda el: el.tag.split(":")[-1].lower()
    text = lambda el: "".join(el.itertext())
    removed = 0
    for el in list(root.iter()):
        if tag(el) != "aunbox":
            continue
        p = el.getparent()
        if p is None:
            continue
        twin = any(tag(s) == "unbox" and text(s) == text(el)
                   for s in p if s is not el)
        if twin:
            p.remove(el)
            removed += 1
    return lh.tostring(root, encoding="unicode") if removed else html


def _headword_chn(html):
    """取第一个不在 unbox/aunbox 内的 chn 文本, 作为词头中文摘要"""
    try:
        root = lh.fromstring(html)
    except Exception:
        return ""
    tag = lambda el: el.tag.split(":")[-1].lower()
    for el in root.iter():
        if tag(el) != "chn":
            continue
        p, inside = el.getparent(), False
        while p is not None and getattr(p, "tag", None):
            if tag(p) in ("unbox", "aunbox"):
                inside = True
                break
            p = p.getparent()
        if inside:
            continue
        t = " ".join(el.itertext()).strip()
        if t:
            return t
    return ""


def _candidates(ref):
    r = ref.replace("\\", "/").lstrip("/")
    return ["/" + r, "\\" + r, r]


def _save_audio(data, ref):
    os.makedirs(AUDIO_DIR, exist_ok=True)
    out = os.path.join(AUDIO_DIR, os.path.basename(ref.replace("\\", "/")))
    if not (os.path.exists(out) and os.path.getsize(out) == len(data)):
        with open(out, "wb") as f:
            f.write(data)
    return out


def lookup(word, bold=True):
    mdx_files = sorted(glob.glob(
        os.path.join(DICT_DIR, "**", "*.mdx"), recursive=True))
    r = {"word": word, "found": False, "text": None, "audio_refs": [],
         "dict": None}
    if not mdx_files:
        return r
    for mdx in mdx_files:
        builder = _get_builder(mdx)
        results = builder.mdx_lookup(word)
        if not results:
            results = builder.mdx_lookup(word.lower())  # 大小写兜底
        if not results:
            continue
        html = _to_str(results[0])
        r.update(found=True, dict=os.path.basename(mdx), _mdx=mdx)
        r["text"] = render(_strip_dup(html), bold=bold)
        import glob as _glob
        _css_parts = []
        for _cp in sorted(_glob.glob(os.path.join(
                os.path.dirname(mdx), "**", "*.css"), recursive=True)):
            try:
                with open(_cp, encoding="utf-8", errors="ignore") as _f:
                    _css_parts.append(_f.read())
            except OSError:
                pass
        r["css"] = "\n\n".join(_css_parts)
        r["_html"] = _strip_dup(html)
        _hw = _headword_chn(html)
        if _hw:
            _hw = re.sub(r"（[^）]*）|\([^)]*\)", "", _hw).strip(" ；;，,")
            if len(_hw) > 30:
                _hw = _hw[:30] + "…"
        if _hw:
            _lines = r["text"].split("\n")
            _lines[0] = _lines[0].rstrip() + " 【" + _hw + "】"
            r["text"] = "\n".join(_lines)
        r["audio_refs"] = _parse_audio_refs(html)  # 只记引用, 不解压
        break  # 命中第一个词典即止
    return r


def _parse_audio_refs(html):
    seen, refs = set(), []
    for pat in AUDIO_PATTERNS:
        for m in pat.finditer(html):
            ref = m.group(1).strip()
            if ref.startswith("sound://"):
                ref = ref[len("sound://"):]
            if not ref or ref in seen:
                continue
            base = os.path.basename(ref)
            if base.startswith("_") or ("__" in base and any(
                    t in base for t in ("gbs", "uss", "brs", "ams"))):
                continue  # 例句音包未下载, 跳过
            seen.add(ref)
            refs.append(ref)
    return refs


def extract_audio(mdx, ref):
    """点播放才解压单个 ref; 目录里只保留当前这一个。
    精确路径失败时, 按 basename 在所有 MDD 卷里兜底匹配。"""
    ref = str(ref).replace("\\", "/").strip().lstrip("/")
    data = multi_mdd_lookup(_get_builder(mdx), ref, _candidates)
    if not data:
        base = os.path.basename(ref).lower()
        try:
            keys = _get_builder(mdx).get_mdd_keys()
        except Exception:
            keys = []
        for k in keys:
            if isinstance(k, (tuple, list)):
                k = k[0] if k else b""
            if isinstance(k, bytes):
                k = k.decode("utf-8", "ignore")
            k = str(k)
            if os.path.basename(k.replace("\\", "/")).lower() == base:
                data = multi_mdd_lookup(_get_builder(mdx), k, _candidates)
                if data:
                    break
    if not data:
        return None
    os.makedirs(AUDIO_DIR, exist_ok=True)
    name = os.path.basename(ref.replace("\\", "/"))
    out = os.path.join(AUDIO_DIR, name)
    if os.path.exists(out) and os.path.getsize(out) == len(data):
        return out
    for f in os.listdir(AUDIO_DIR):
        if f != name:
            try:
                os.remove(os.path.join(AUDIO_DIR, f))
            except OSError:
                pass
    with open(out, "wb") as fp:
        fp.write(data)
    return out


def get_playable(word, variant="gb"):
    """查词并解压指定发音变体("gb"英音/"us"美音), 返回 mp3 路径或 None
    不走 render(), 纯查索引, 开销极小"""
    mdx_files = sorted(glob.glob(
        os.path.join(DICT_DIR, "**", "*.mdx"), recursive=True))
    for mdx in mdx_files:
        b = _get_builder(mdx)
        results = b.mdx_lookup(word) or b.mdx_lookup(word.lower())
        if not results:
            continue
        html = _to_str(results[0])
        for ref in _parse_audio_refs(html):
            if ("_%s_" % variant) in os.path.basename(ref).lower():
                return extract_audio(mdx, ref)
    return None


# ============================================================
# Bing 在线翻译（免 token 端点）
# ============================================================
BING_URL = "https://edge.microsoft.com/translate/translatetext"
EDGE_UA = ("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
           "(KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 Edg/131.0.0.0")


def translate_text(text, target_lang="zh-Hans"):
    if not text or not text.strip():
        return ""
    url = f"{BING_URL}?isEnterpriseClient=false&to={target_lang}"
    headers = {"Content-Type": "application/json", "User-Agent": EDGE_UA}
    try:
        # body 是纯字符串数组 [text]，非对象；source 不传，服务端自动检测
        resp = requests.post(url, json=[text], headers=headers, timeout=15)
        resp.raise_for_status()
        result = resp.json()
        if (isinstance(result, list) and result
                and "translations" in result[0]):
            return result[0]["translations"][0]["text"].strip()
        return "解析响应失败"
    except requests.exceptions.RequestException as e:
        return f"网络请求失败: {e}"


# ============================================================
# GoldenDict 风格在线词典/翻译站
# 非单词输入时在同一 WebView 窗口内打开(像本地查词一样渲染);
# 模板里的 %GDWORD% 会被替换成 URL 编码后的查询文本, 按需增删即可
# ============================================================
ONLINE_SITES = [
    "http://dict.youdao.com/w/eng/%GDWORD%",
    "https://www.bing.com/dict/search?q=%GDWORD%",
]


def online_site_urls(text):
    """按模板生成完整 URL 列表 (%GDWORD% -> urlencode(text))"""
    import urllib.parse
    q = urllib.parse.quote((text or "").strip(), safe="")
    return [t.replace("%GDWORD%", q) for t in ONLINE_SITES if "%GDWORD%" in t]


def open_online_sites(text):
    """(备用) 用系统默认浏览器依次打开 ONLINE_SITES, 返回成功个数"""
    import webbrowser
    n = 0
    for url in online_site_urls(text):
        try:
            if webbrowser.open(url):
                n += 1
        except Exception:
            pass
    return n


def _idle_call(fn, *args):
    """把 UI 调用投递到 GTK 主线程执行。
    WebKitGTK 禁止跨线程操作 WebView; pywebview 的 JS-API 回调在
    工作线程里, load_url/load_html 必须经过 GLib.idle_add。"""
    done = {"v": False}

    def _wrap():
        done["v"] = True
        try:
            fn(*args)
        except Exception:
            pass
        return False

    try:
        from gi.repository import GLib
        GLib.idle_add(_wrap)
    except Exception:
        _wrap()

    def _fallback():
        if not done["v"]:  # 非 GTK 后端时 idle 不会跑, 线程内兜底
            _wrap()
        return False

    try:
        import threading
        t = threading.Timer(1.2, _fallback)
        t.daemon = True
        t.start()
    except Exception:
        pass
    return True


def _online_navigate(window, url):
    """线程安全地在本 WebView 窗口内打开 URL"""
    def _go():
        try:
            window.load_url(url)
        except Exception:
            try:
                import json as _j
                window.evaluate_js("location.replace(%s)" % _j.dumps(url))
            except Exception:
                pass
    return _idle_call(_go)


def _online_site_names():
    """ONLINE_SITES 的按钮短名, 未识别域名显示 host"""
    import urllib.parse
    names = []
    for t in ONLINE_SITES:
        host = urllib.parse.urlsplit(t).netloc.lower()
        for k, v in (("youdao", "有道"), ("baidu", "百度翻译"),
                     ("bing", "必应"), ("haici", "海词"),
                     ("cambridge", "剑桥"), ("merriam", "韦氏"),
                     ("collins", "柯林斯"), ("oxford", "牛津")):
            if k in host:
                names.append(v)
                break
        else:
            names.append(host)
    return names


def _online_inject_js(query=""):
    """远程词典页顶部注入站点切换工具条的 JS"""
    import json
    names = _online_site_names()
    urls = online_site_urls(query or "")
    data = [{"n": names[i], "u": urls[i]}
            for i in range(min(len(names), len(urls)))]
    return (
        "(function(){var d=%s,q=%s;"
        "var o=document.getElementById('dic-online-bar');"
        "if(o)o.remove();"
        "var b=document.createElement('div');b.id='dic-online-bar';"
        "b.style.cssText='position:fixed;top:0;left:0;right:0;"
        "z-index:2147483647;background:rgba(245,245,245,.97);"
        "border-bottom:1px solid #ddd;padding:6px 8px;"
        "display:flex;gap:6px;align-items:center;"
        "font:14px/1.4 sans-serif;';"
        "var t=document.createElement('span');t.textContent=q;"
        "t.style.cssText='flex:1;overflow:hidden;text-overflow:ellipsis;"
        "white-space:nowrap;color:#555;';b.appendChild(t);"
        "function mk(s,fn){var x=document.createElement('button');"
        "x.textContent=s;x.style.cssText='padding:4px 10px;"
        "border:1px solid #ccc;border-radius:6px;background:#fff;"
        "cursor:pointer;';x.onclick=function(){try{fn()}catch(e){}};"
        "return x;}"
        "d.forEach(function(it,i){b.appendChild(mk(it.n,function(){"
        "pywebview.api.online_open(i);}));});"
        "b.appendChild(mk('⌂',function(){pywebview.api.online_home();}));"
        "b.appendChild(mk('✕',function(){pywebview.api.close();}));"
        "document.body.appendChild(b);"
        "document.body.style.paddingTop='44px';"
        "document.addEventListener('keydown',function(e){"
        "if(e.ctrlKey&&e.shiftKey&&(e.key==='I'||e.key==='i')){"
        "e.preventDefault();try{pywebview.api.devtools();}catch(err){}return;}"
        "if(e.key==='Escape'){"
        "try{pywebview.api.close();}catch(err){}}},true);})();"
    ) % (json.dumps(data, ensure_ascii=False),
         json.dumps(query or "", ensure_ascii=False))


def _online_stacked_html(query):
    """GoldenDict 式拼接页: 所有在线站点以 iframe 上下排列。
    (dic scroll patch) 跨域 iframe 滚到边不会接力滚动外层,
    提供右侧悬浮 ▲▼⇈⇟ / Ctrl+↓↑ 等滚动外层的方式。"""
    from html import escape as _e2
    names = _online_site_names()
    urls = online_site_urls(query or "")
    rows = []
    for i, u in enumerate(urls):
        nm = names[i] if i < len(names) else u
        rows.append(
            '<div class="site"><div class="hd"><b>'
            + str(i + 1) + ". " + _e2(nm) + "</b>"
            + '<button onclick="var f=document.getElementById(\'f' + str(i)
            + '\');if(f){var s=f.src;f.src=\'about:blank\';'
              "setTimeout(function(){f.src=s;},60);}\">⟳</button>"
            + '<button onclick="try{pywebview.api.open_external(' + str(i)
            + ')}catch(e){}">↗ 独立打开</button>'
            + '<span class="u">' + _e2(u) + "</span></div>"
            + '<iframe id="f' + str(i) + '" src="' + _e2(u, quote=True)
            + '" referrerpolicy="no-referrer"></iframe></div>'
        )
    return (
        '<div id="stack-tip">🌐 已同时打开 ' + str(len(urls))
        + " 个站点（上下拼接）。滚轮在站点内部滚动；跨站点用右侧 ▲▼ 按钮或 "
          "Ctrl+↓/↑/PgDn/PgUp（焦点在站点里也有效），也可把鼠标移到站点间的"
          "蓝色标题条上滚外页。站点空白=该站禁止内嵌，点其 ↗。</div>"
        '<div id="stack-nav">'
        '<button onclick="window.scrollTo(0,0)" title="回顶部">⇈</button>'
        '<button onclick="window.scrollBy(0,-Math.round('
        'window.innerHeight*0.9))" title="上一站/Ctrl+↑">▲</button>'
        '<button onclick="window.scrollBy(0,Math.round('
        'window.innerHeight*0.9))" title="下一站/Ctrl+↓">▼</button>'
        '<button onclick="window.scrollTo(0,document.body.scrollHeight)"'
        ' title="到底部">⇟</button>'
        "</div>"
        "<style>"
        "#stack-tip{padding:8px 14px;color:#555;font-size:13px;"
        "background:#f7f7f7;border-bottom:1px solid #eee;}"
        "#stack-nav{position:fixed;right:10px;top:50%;"
        "transform:translateY(-50%);display:flex;flex-direction:column;"
        "gap:6px;z-index:2147483000;}"
        "#stack-nav button{width:38px;height:38px;font-size:16px;"
        "border:1px solid #c5cfdd;border-radius:8px;"
        "background:rgba(255,255,255,.95);cursor:pointer;"
        "box-shadow:0 1px 4px rgba(0,0,0,.18);}"
        "#stack-nav button:active{background:#e8eef7;}"
        ".site{margin:0 0 10px 0;}"
        ".site .hd{display:flex;gap:8px;align-items:center;padding:8px 14px;"
        "background:#eef3fa;border-top:1px solid #dbe5f0;"
        "border-bottom:1px solid #dbe5f0;font-size:14px;}"
        ".site .hd .u{color:#888;font-size:12px;overflow:hidden;"
        "text-overflow:ellipsis;white-space:nowrap;flex:1;}"
        ".site .hd button{padding:2px 8px;font-size:12px;cursor:pointer;}"
        ".site iframe{width:100%;height:82vh;border:0;display:block;"
        "background:#fff;}"
        "</style>" + "".join(rows)
    )

# ============================================================
# GUI / CLI
# ============================================================
# ============================================================
# (dic img patch) dicres:// 协议: 本地词典图片/CSS 资源从 MDD 直读
# ============================================================
_RES_SKIP = re.compile(r"(?i)^(https?:|data:|file:|dicres:|about:|//)")
_CSS_URL = re.compile(r"url\(\s*(['\"]?)([^'\")]+)\1\s*\)")


def _css_localize(css_text):
    """词典 CSS 里的 url(/x.png) → url(dicres://x.png)"""
    def _repl(m):
        u = (m.group(2) or "").strip()
        if not u or _RES_SKIP.match(u):
            return m.group(0)
        return "url(dicres://%s)" % u.replace("\\", "/").lstrip("/")
    return _CSS_URL.sub(_repl, css_text or "")


def _dicres_find(ref):
    """按精确候选 → 全卷 basename 兜底, 在所有 MDD 卷里找资源"""
    import glob as _g
    mdx_list = sorted(_g.glob(os.path.join(DICT_DIR, "**", "*.mdx"),
                              recursive=True))
    data = None
    for mdx in mdx_list:
        try:
            data = multi_mdd_lookup(_get_builder(mdx), ref, _candidates)
        except Exception:
            data = None
        if data:
            return data
    base = os.path.basename(ref.replace("\\", "/")).lower()
    if not base:
        return None
    for mdx in mdx_list:
        try:
            keys = _get_builder(mdx).get_mdd_keys()
        except Exception:
            continue
        hit = None
        for k in keys:
            if isinstance(k, (tuple, list)):
                k = k[0] if k else b""
            if isinstance(k, bytes):
                k = k.decode("utf-8", "ignore")
            if os.path.basename(str(k).replace("\\", "/")).lower() == base:
                hit = str(k)
                break
        if hit:
            try:
                data = multi_mdd_lookup(_get_builder(mdx), hit, _candidates)
            except Exception:
                data = None
            if data:
                return data
    return None


def _dicres_serve(request, *args):
    """dicres:// 协议回调: 从 MDD 资源包提取图片等资源"""
    import mimetypes
    import urllib.parse as _up
    from gi.repository import Gio as _Gio, GLib as _GLib
    data = None
    raw = ""
    try:
        uri = request.get_uri() or ""
        raw = re.sub(r"(?i)^dicres://", "", uri).lstrip("/")
        for cand in (_up.unquote(raw), raw):
            if not cand:
                continue
            data = _dicres_find(cand)
            if data:
                break
    except Exception:
        data = None
    try:
        if data:
            mime = mimetypes.guess_type(raw)[0] or "application/octet-stream"
            stream = _Gio.MemoryInputStream.new_from_bytes(
                _GLib.Bytes.new(bytes(data)))
            request.finish(stream, len(data), mime)
        else:
            stream = _Gio.MemoryInputStream.new_from_bytes(
                _GLib.Bytes.new(b""))
            request.finish(stream, 0, "text/plain")
    except Exception:
        pass


def _register_dicres_scheme():
    """在默认 WebContext 上注册 dicres:// (必须在 webview 创建前调用)"""
    try:
        import gi as _gi
    except Exception:
        return False
    ctx = None
    for _ver in ("4.1", "4.0"):
        try:
            _gi.require_version("WebKit2", _ver)
            from gi.repository import WebKit2 as _WK2
            ctx = _WK2.WebContext.get_default()
            break
        except Exception:
            ctx = None
    if ctx is None:
        return False
    try:
        ctx.register_uri_scheme("dicres", _dicres_serve)
        return True
    except Exception:
        return False


def _is_single_word(text):
    """单个单词才使用 WebView 词典；短语/句子使用在线词典站。"""
    text = (text or "").strip()
    if not text or len(text.split()) != 1:
        return False
    return re.search(r"[.!?,;:?。！？，、；：！？]", text) is None


def _switch_ui(text, mode):
    """启动另一种 GUI；mode 为 web/native。"""
    text = (text or "").strip()
    if not text:
        return
    flag = "--web" if mode == "web" else "--native"
    subprocess.Popen(
        [sys.executable, os.path.abspath(__file__), flag, text],
        start_new_session=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )


def _play_audio(path):
    """使用系统播放器播放本地音频。"""
    players = (
        ("mpv", ["mpv", "--no-video", "--really-quiet", path]),
        ("paplay", ["paplay", path]),
        ("aplay", ["aplay", path]),
        ("ffplay", ["ffplay", "-nodisp", "-autoexit", "-loglevel", "quiet", path]),
        ("cvlc", ["cvlc", "--play-and-exit", "--intf", "dummy", path]),
    )
    for _, args in players:
        if not shutil.which(args[0]):
            continue
        try:
            subprocess.Popen(
                args,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            return True
        except OSError:
            pass
    return False


# ============================================================
# 原生 Qt 翻译窗口
# ============================================================
def run_gui(initial_text=None, auto_switch=True):
    try:
        import FreeSimpleGUIQt as sg
        using_qt = True
    except ImportError:
        import FreeSimpleGUI as sg
        using_qt = False

    current = {"word": None}

    def do_translate(text):
        text = (text or "").strip()
        if not text:
            return
        if auto_switch:
            # dic online patch: 单词/短语一律交给 WebView 窗口
            # (单词→本地词典, 短语/句子→在线词典)
            _switch_ui(text, "web")
            try:
                window.close()
            except Exception:
                pass
            return
        current["word"] = None
        if _is_single_word(text):
            r = lookup(text, bold=False)
            if r["found"]:
                window["-OUTPUT-"].update(r["text"] or "(渲染为空)")
                window["-STATUS-"].update("📖 本地词典 | " + r["dict"])
                current["word"] = r["word"]
                return
        window["-STATUS-"].update("本地未收录, 在线翻译中…")
        window["-OUTPUT-"].update(translate_text(text))
        window["-STATUS-"].update("🌐 在线翻译")

    def safe_copy(text):
        if not text or not text.strip():
            return False
        if hasattr(sg, "clipboard_set"):
            try:
                sg.clipboard_set(text)
                return True
            except Exception:
                pass
        if using_qt:
            try:
                from PySide6.QtWidgets import QApplication
                QApplication.clipboard().setText(text)
                return True
            except Exception:
                pass
        return False

    layout = [
        [sg.Text("输入内容后回车翻译", font=("微软雅黑", 12), key="-STATUS-")],
        [sg.Multiline(key="-INPUT-", size=(60, 6), font=("微软雅黑", 11), focus=True)],
        [sg.Text("🌐 翻译结果", font=("微软雅黑", 12))],
        [sg.Multiline(key="-OUTPUT-", size=(60, 18), font=("微软雅黑", 11))],
        [
            sg.Button("📋 复制", key="-COPY-", size=(10, 1)),
            sg.Button("🗑️ 清空", key="-CLEAR-", size=(10, 1)),
            sg.Button("🚪 退出", key="-EXIT-", size=(10, 1)),
        ],
    ]

    window = sg.Window(
        "dic",
        layout,
        resizable=True,
        finalize=True,
        return_keyboard_events=True,
    )

    if using_qt:
        try:
            from PySide6.QtWidgets import QMenu
            from PySide6.QtGui import QAction
            from PySide6.QtCore import Qt as _Qt
            out_widget = window["-OUTPUT-"].Widget

            def _show_menu(pos):
                menu = QMenu(out_widget)
                a1 = QAction("📋 复制选中", menu)
                a1.triggered.connect(
                    lambda: safe_copy(
                        out_widget.textCursor().selectedText().replace("\u2029", "\n")
                    )
                )
                a2 = QAction("全选", menu)
                a2.triggered.connect(out_widget.selectAll)
                menu.addAction(a1)
                menu.addAction(a2)
                menu.exec(out_widget.mapToGlobal(pos))

            out_widget.setContextMenuPolicy(_Qt.CustomContextMenu)
            out_widget.customContextMenuRequested.connect(_show_menu)
        except Exception as exc:
            print("右键菜单挂载失败(不影响主功能):", exc)

    if initial_text:
        initial_text = initial_text.strip()
        if initial_text:
            window["-INPUT-"].update(initial_text)
            do_translate(initial_text)

    ENTER_KEYS = {"special 16777220", "special 16777221", "\r", "\n"}
    ESC_KEYS = {"special 16777216", "Escape:27", "Escape"}

    try:
        while True:
            event, values = window.read()
            if (
                event in (sg.WIN_CLOSED, None, "")
                or event == "-EXIT-"
                or event in ESC_KEYS
            ):
                break
            if event in ENTER_KEYS:
                text = (values.get("-INPUT-", "") or "").strip()
                if text:
                    do_translate(text)
                continue
            if event == "-COPY-":
                text = (values.get("-OUTPUT-", "") or "").strip()
                safe_copy(text)
                continue
            if event in ("-PLAY-GB-", "-PLAY-US-"):
                word = current.get("word")
                if not word:
                    window["-STATUS-"].update("当前结果无本地发音")
                    continue
                variant = "gb" if event == "-PLAY-GB-" else "us"
                path = get_playable(word, variant)
                if path and _play_audio(path):
                    window["-STATUS-"].update("🔊 " + os.path.basename(path))
                else:
                    window["-STATUS-"].update(
                        "⛔ 无音频或缺少播放器(需 mpv/ffplay/vlc)"
                    )
                continue
            if event == "复制选中":
                try:
                    text = window["-OUTPUT-"].Widget.textCursor().selectedText()
                    text = text.replace("\u2029", "\n")
                except Exception:
                    text = ""
                safe_copy(text)
                continue
            if event == "全选":
                try:
                    window["-OUTPUT-"].Widget.selectAll()
                except Exception:
                    pass
                continue
    finally:
        try:
            window.close()
        except Exception:
            pass


# ============================================================
# WebView 词典窗口
# ============================================================
WEB_SHELL = r'''<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<style>
body { font-family: 'Noto Sans CJK SC', '微软雅黑', sans-serif; font-size: 17px; margin: 0; line-height: 1.55; }
#bar { position: sticky; top: 0; background: #f5f5f5; border-bottom: 1px solid #ddd; padding: 8px; display: flex; gap: 6px; z-index: 99999; }
#q { flex: 1; font-size: 15px; padding: 6px 10px; border: 1px solid #ccc; border-radius: 6px; }
button { padding: 6px 12px; border: 1px solid #ccc; border-radius: 6px; background: #fff; cursor: pointer; }
button:hover { background: #eee; }
#content { padding: 14px 18px; }
#content img { max-width: 100%; }
[data-sound] { cursor: pointer; }
</style>
<style id="dict-css"></style>
</head>
<body>
<div id="bar">
<input id="q" value="__WORD__" placeholder="输入单词后回车…" autocomplete="off" autofocus>
<button id="lookup-button">查词</button>
</div>
<div id="content">__CONTENT__</div>
<script>
var MEDIA_EXT = /\.(mp3|wav|ogg|oga|aac|m4a|spx|opus|wma)(\?|#|$)/i;
var DENY_SCHEME = /^(https?|mailto|javascript|about|file|entry|bword|gdlookup|dic):/i;
function flash(msg) {
  var t = document.getElementById('dic-toast');
  if (!t) {
    t = document.createElement('div');
    t.id = 'dic-toast';
    t.style.cssText = 'position:fixed;right:12px;top:52px;background:#333;color:#fff;padding:6px 10px;border-radius:6px;font-size:12px;z-index:999999;max-width:70%;word-break:break-all;display:none;white-space:pre-wrap;';
    document.body.appendChild(t);
  }
  t.textContent = msg;
  t.style.display = 'block';
  clearTimeout(t._tm);
  t._tm = setTimeout(function(){ t.style.display = 'none'; }, 3500);
}
function stripScheme(s) {
  return String(s).replace(/^[a-z][a-z0-9+.\-]*:\/\//i, '');
}
function soundRef(el) {
  if (!el || el.nodeType !== 1) return '';
  var attrs = ['data-sound', 'data-src-mp3', 'src-mp3'];
  for (var i = 0; i < attrs.length; i++) {
    var v = el.getAttribute(attrs[i]);
    if (v) return stripScheme(v);
  }
  if ((el.tagName || '').toLowerCase() === 'a') {
    var h = String(el.getAttribute('href') || '').trim();
    if (!h) return '';
    var low = h.toLowerCase();
    if (low.indexOf('sound://') === 0 || MEDIA_EXT.test(low)) return stripScheme(h);
  }
  return '';
}
function callPlay(ref) {
  function go() {
    Promise.resolve(pywebview.api.play(ref)).then(function(r) {
      if (r) flash(String(r));
    }).catch(function(err) { flash('⛔ play: ' + err); });
  }
  if (window.pywebview && window.pywebview.api && window.pywebview.api.play) { go(); return; }
  var tries = 0;
  var t = setInterval(function() {
    tries++;
    if (window.pywebview && window.pywebview.api && window.pywebview.api.play) { clearInterval(t); go(); }
    else if (tries > 50) { clearInterval(t); flash('⛔ pywebview 未就绪'); }
  }, 200);
}
function handler(e) {
  if (e.__dicHandled) return;
  var _sel = window.getSelection && window.getSelection();
  if (_sel && _sel.type === 'Range' && String(_sel).length) { return; }
  e.__dicHandled = true;
  var node = e.target;
  if (!node || node.nodeType !== 1) return;
  while (node && node !== document) {
    if (node.nodeType === 1) {
      var ref = soundRef(node);
      if (ref) {
        e.preventDefault(); e.stopPropagation();
        if (e.stopImmediatePropagation) e.stopImmediatePropagation();
        flash('🔊 ' + ref);
        callPlay(ref);
        return;
      }
      if ((node.tagName || '').toLowerCase() === 'a') {
        var h = String(node.getAttribute('href') || '').trim();
        if (h && !/^(https?:|#|javascript:|mailto:|about:)/i.test(h)) {
          e.preventDefault(); e.stopPropagation();
          if (e.stopImmediatePropagation) e.stopImmediatePropagation();
          if (!DENY_SCHEME.test(h)) {
            var clean = stripScheme(h);
            flash('🔊(兜底) ' + clean);
            callPlay(clean);
          } else {
            flash('⛔ 非音频链接: ' + h + '\n' + String(node.outerHTML || '').substring(0, 120));
          }
          return;
        }
        if (h) return; // 普通网页链接放行
        // 无 href 的 <a>: 继续向上找
      }
    }
    node = node.parentNode;
  }
}
['click', 'auxclick'].forEach(function(ev) {
  window.addEventListener(ev, handler, true);
  document.addEventListener(ev, handler, true);
});
['mousedown', 'pointerdown'].forEach(function(ev) {
  window.addEventListener(ev, function(e) {
    var node = e.target;
    while (node && node !== document) {
      if (node.nodeType === 1 && soundRef(node) && node === e.target) {
        e.preventDefault();
        return;
      }
      node = node.parentNode;
    }
  }, true);
});
function normalizeSounds(root) {
  if (!root || !root.querySelectorAll) return;
  var list = root.querySelectorAll('a[href]');
  for (var i = 0; i < list.length; i++) {
    var el = list[i];
    var h = String(el.getAttribute('href') || '').trim();
    var low = h.toLowerCase();
    if (low.indexOf('sound://') === 0 || MEDIA_EXT.test(low)) {
      el.setAttribute('data-sound', stripScheme(h));
      el.removeAttribute('href');
      el.removeAttribute('target');
      el.removeAttribute('onclick');
    }
  }
  var es = root.querySelectorAll('[data-src-mp3],[src-mp3]');
  for (var j = 0; j < es.length; j++) {
    var e2 = es[j];
    var ref = e2.getAttribute('data-src-mp3') || e2.getAttribute('src-mp3') || '';
    if (ref && !e2.getAttribute('data-sound')) {
      e2.setAttribute('data-sound', stripScheme(ref));
    }
  }
}
function fixSound(root) { /* dic sound v10 */
  if (!root || !root.querySelectorAll) return;
  var els = root.querySelectorAll('[data-sound]');
  for (var i = 0; i < els.length; i++) {
    var a = els[i];
    if (a.__dicDone) continue;
    a.__dicDone = true;
    var ref = a.getAttribute('data-sound') || '';
    if (!ref) continue;
    if (!String(a.textContent).trim() && !a.querySelector('img')) {
      // 罕见空锚点兜底: 绑到词典 CSS 画的图标元素(audio[name=xxx])上
      var base = ref.replace(/\.mp3$/i, ''), au = null;
      try { au = root.querySelector('audio[name="' + base + '"]'); } catch (err) {}
      if (au && au !== a) {
        au.setAttribute('data-sound', ref);
        au.__dicDone = true;
        au.style.cursor = 'pointer';
        if (!au.style.display) au.style.display = 'inline-block';
        continue;
      }
      var sib = a.nextElementSibling;
      if (sib && !sib.getAttribute('data-sound') && String(sib.textContent).trim()) {
        sib.setAttribute('data-sound', ref);
        sib.__dicDone = true;
      }
      continue;
    }
    // 整行提升: pron 容器优先, 否则短音标行提升父容器
    var done = false, p = a.parentElement;
    for (var up = 0; p && p !== root && up < 2; up++, p = p.parentElement) {
      var tn = String(p.tagName || '') + ' ' + String(p.getAttribute('class') || '');
      if (tn.toLowerCase().indexOf('pron') >= 0) {
        var kids = p.querySelectorAll('[data-sound]');
        var all = kids.length > 0;
        for (var k2 = 0; k2 < kids.length; k2++) {
          if (String(kids[k2].getAttribute('data-sound')) !== ref) { all = false; break; }
        }
        if (all && !p.getAttribute('data-sound')) {
          p.setAttribute('data-sound', ref);
          done = true;
        }
        break;
      }
    }
    if (!done) {
      var par = a.parentElement;
      if (par && par !== root && !par.getAttribute('data-sound')) {
        var ks = par.querySelectorAll('[data-sound]');
        if (ks.length === 1 && String(par.textContent).replace(/\s+/g, '').length <= 40) {
          par.setAttribute('data-sound', ref);
        }
      }
    }
  }
}
function normalizeBoth(root) {
  normalizeSounds(root);
  try { fixSound(root); } catch (err) {}
}
var observer = new MutationObserver(function() {
  normalizeBoth(document.getElementById('content'));
});
observer.observe(document.documentElement, {childList: true, subtree: true});
function applyCss(css) {
  var st = document.getElementById('dict-css');
  if (st) st.textContent = css || '';
}
function doLookup() {
  var word = document.getElementById('q').value.trim();
  if (!word) return;
  if (!(window.pywebview && window.pywebview.api)) { flash('⏳ pywebview 未就绪'); return; }
  pywebview.api.lookup(word).then(function(r) {
    if (!r || r.switch) return;
    if (r && r.online) { document.getElementById('content').innerHTML = r.html || ''; applyCss(r.css || ''); normalizeBoth(document.getElementById('content')); return; }
    document.getElementById('content').innerHTML = r.html || '';
    applyCss(r.css || '');
    normalizeBoth(document.getElementById('content'));
  }).catch(function(err) { flash('⛔ ' + err); });
}
document.getElementById('lookup-button').addEventListener('click', doLookup);
document.getElementById('q').addEventListener('keydown', function(e) {
  if (e.key === 'Enter') { e.preventDefault(); doLookup(); return; }
  if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); try { pywebview.api.close(); } catch (err) {} }
}, true);
document.addEventListener('keydown', function(e) {
  if (e.ctrlKey && e.shiftKey && (e.key === 'I' || e.key === 'i')) { e.preventDefault(); e.stopPropagation(); try { pywebview.api.devtools(); } catch (err) {} return; }
  if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); try { pywebview.api.close(); } catch (err) {} }
}, true);
window.addEventListener('pywebviewready', function() {
  normalizeBoth(document.getElementById('content'));
});
normalizeBoth(document.getElementById('content'));
</script>
<script>
/* dic zoom patch (native webkit zoom) */
var DIC_ZOOM = __ZOOM__;
function dicApplyZoom() {
  var tries = 0;
  (function go() {
    if (window.pywebview && window.pywebview.api && window.pywebview.api.set_zoom) {
      pywebview.api.set_zoom(DIC_ZOOM);
      return;
    }
    if (++tries < 50) { setTimeout(go, 200); }
  })();
}
document.addEventListener('keydown', function(e) {
  if (!(e.ctrlKey && !e.altKey && !e.metaKey)) return;
  var k = e.key;
  if (k === '=' || k === '+') { DIC_ZOOM = Math.min(3, +(DIC_ZOOM + 0.1).toFixed(2)); }
  else if (k === '-') { DIC_ZOOM = Math.max(0.5, +(DIC_ZOOM - 0.1).toFixed(2)); }
  else if (k === '0') { DIC_ZOOM = 1.0; }
  else { return; }
  e.preventDefault(); e.stopPropagation();
  dicApplyZoom();
  if (typeof flash === 'function') { flash('🔍 缩放 ' + Math.round(DIC_ZOOM * 100) + '%'); }
}, true);
</script>
<script>
/* dic copy patch */
document.addEventListener('keydown', function(e) {
  var _ae = document.activeElement;
  if (_ae && (_ae.tagName === 'INPUT' || _ae.tagName === 'TEXTAREA')) return;
  if (!(e.ctrlKey && !e.altKey && !e.metaKey)) return;
  var k = (e.key || '').toLowerCase();
  if (k === 'c' || k === 'insert') {
    var sel = window.getSelection();
    if (sel && String(sel).length) {
      try { document.execCommand('copy'); flash('📋 已复制 ' + String(sel).length + ' 字符'); } catch (err) {}
      e.preventDefault(); e.stopPropagation();
    }
  } else if (k === 'a') {
    var r = document.createRange();
    r.selectNodeContents(document.getElementById('content'));
    var s = window.getSelection();
    s.removeAllRanges();
    s.addRange(r);
    e.preventDefault(); e.stopPropagation();
    flash('已全选, Ctrl+C 复制');
  }
}, true);
</script>
</body>
</html>'''

OXFORD9_CSS = r'''
pron-g-blk br {
    display: none !important;
}

[data-sound] {
    cursor: pointer;
    text-decoration: none;
}

#content {
    font-size: 17px !important;
    line-height: 1.65 !important;
}

/* dic select patch */
html, body, #content, #content * {
    -webkit-user-select: text !important;
    user-select: text !important;
}

[data-sound], [data-sound] * {
    cursor: pointer;
}
'''


# ============================================================
# dic 配置持久化(窗口尺寸/缩放) + GTK 原生钩子 (dic cfg patch)
# ============================================================
def _dic_cfg_path():
    d = os.path.join(os.path.expanduser("~"), ".config", "dic")
    try:
        os.makedirs(d, exist_ok=True)
    except OSError:
        pass
    return os.path.join(d, "config.json")


def _load_cfg():
    try:
        import json
        with open(_dic_cfg_path(), encoding="utf-8") as f:
            data = json.load(f)
        return data if isinstance(data, dict) else {}
    except Exception:
        return {}


def _save_cfg_merged(extra):
    import json
    cfg = _load_cfg()
    cfg.update({k: v for k, v in extra.items() if v is not None})
    try:
        with open(_dic_cfg_path(), "w", encoding="utf-8") as f:
            json.dump(cfg, f, ensure_ascii=False, indent=1)
    except OSError:
        pass


def _find_webkit(widget):
    """GTK3 下递归查找 WebKit2.WebView"""
    if hasattr(widget, "set_zoom_level") and hasattr(widget, "get_zoom_level"):
        return widget
    try:
        children = widget.get_children()
    except Exception:
        return None
    for ch in children:
        r = _find_webkit(ch)
        if r is not None:
            return r
    return None


def _native_hook(_win, state):
    """GUI 循环启动后: 定位原生 GTK 窗口, 恢复缩放/最大化,
    挂 configure-event 防抖保存窗口尺寸(回调均在主线程)。"""
    try:
        import gi
        gi.require_version("Gtk", "3.0")
        from gi.repository import Gtk, GLib
    except Exception:
        return
    tries = {"n": 0}

    def probe():
        tries["n"] += 1
        gtkw = None
        for tw in Gtk.Window.list_toplevels():
            if (tw.get_title() or "") == "dic":
                gtkw = tw
                break
        if gtkw is None:
            return tries["n"] < 100
        wk = _find_webkit(gtkw)
        if wk is None and tries["n"] < 40:
            return True  # webview 子控件尚未挂好, 稍后再试
        state["webkit"] = wk
        try:
            if wk is not None:  # (dic devtools patch) 允许运行时打开检查器
                wk.get_settings().props.enable_developer_extras = True
        except Exception:
            pass

        # (dic esc patch) 窗口级按键拦截: key 事件先到 GTK 顶层窗口,
        # 因此即使焦点在 iframe 里, Esc / Ctrl+Shift+I 也有效
        def _toggle_insp():
            if wk is None:
                return
            try:
                wk.get_settings().props.enable_developer_extras = True
            except Exception:
                pass
            try:
                insp = wk.get_inspector()
                if state.get("devtools"):
                    insp.close()
                    state["devtools"] = False
                else:
                    insp.show()
                    state["devtools"] = True
            except Exception:
                pass

        def _on_key(_w, ev):
            name = ""
            try:
                name = ev.keyval_name or ""
            except Exception:
                pass
            if name == "Escape":
                try:
                    _win.destroy()
                except Exception:
                    try:
                        import webview as _wv
                        if _wv.windows:
                            _wv.windows[0].destroy()
                    except Exception:
                        pass
                return True
            try:
                st = int(ev.state)
                if (st & 5) == 5 and name in ("I", "i"):  # Ctrl|Shift
                    _toggle_insp()
                    return True
                # (dic scroll patch) 仅 Ctrl: 滚动外层拼接页
                if (st & 4) and not (st & 1):
                    if name in ("Down", "Page_Down"):
                        try:
                            _win.evaluate_js("window.scrollBy(0,Math.round("
                                             "window.innerHeight*0.9));")
                        except Exception:
                            pass
                        return True
                    if name in ("Up", "Page_Up"):
                        try:
                            _win.evaluate_js("window.scrollBy(0,-Math.round("
                                             "window.innerHeight*0.9));")
                        except Exception:
                            pass
                        return True
                    if name == "Home":
                        try:
                            _win.evaluate_js("window.scrollTo(0,0);")
                        except Exception:
                            pass
                        return True
                    if name == "End":
                        try:
                            _win.evaluate_js(
                                "window.scrollTo(0,document.body.scrollHeight);")
                        except Exception:
                            pass
                        return True
            except Exception:
                pass
            return False

        try:
            gtkw.connect("key-press-event", _on_key)
        except Exception:
            pass
        try:
            if wk is not None:  # (dic img patch) 兜底再注册一次资源协议
                wk.get_context().register_uri_scheme("dicres", _dicres_serve)
        except Exception:
            pass
        cfg = _load_cfg()
        try:
            zoom0 = min(3.0, max(0.5, float(cfg.get("zoom") or 1.0)))
        except Exception:
            zoom0 = 1.0
        try:
            if wk is not None and abs(zoom0 - 1.0) > 1e-9:
                wk.set_zoom_level(zoom0)
            if cfg.get("maximized"):
                gtkw.maximize()
        except Exception:
            pass
        pend = {"id": None}

        def _do_save():
            pend["id"] = None
            try:
                if gtkw.is_maximized():
                    _save_cfg_merged({"maximized": True})
                else:
                    cw, ch = gtkw.get_size()
                    _save_cfg_merged({"maximized": False, "width": int(cw), "height": int(ch)})
            except Exception:
                pass
            return False

        def _on_cfg(widget, event):
            if pend["id"]:
                try:
                    GLib.source_remove(pend["id"])
                except Exception:
                    pass
            pend["id"] = GLib.timeout_add(400, _do_save)
            return False

        try:
            gtkw.connect("configure-event", _on_cfg)
        except Exception:
            pass
        return False

    GLib.timeout_add(300, probe)


def run_gui_web(initial_text=None, auto_switch=True):
    print("[dic] audio-click v10 pid=%d" % os.getpid(), file=sys.stderr)
    try:
        import webview
    except ImportError:
        print(
            "缺少 pywebview，请先: pip install pywebview",
            file=sys.stderr,
        )
        # 兜底进 Qt 时关掉自动切换, 避免 web↔qt 无限互启
        run_gui(initial_text=initial_text, auto_switch=False)
        return
    from html import escape as _esc
    state = {"mdx": None, "word": None, "online": False,
             "last_query": "", "webkit": None}

    def _goto_online(text):
        """(dic stack patch) GoldenDict 式: 所有在线站点上下拼接在同一页面"""
        text = (text or "").strip()
        state["last_query"] = text
        state["online"] = False  # 本地拼接页, 不注入远程工具条
        return {"online": True, "html": _online_stacked_html(text), "css": ""}

    def _prepare(html_text):
        html_text = (
            html_text
            .replace("<xhtml:", "<")
            .replace("</xhtml:", "</")
        )
        # 词典自带 JS 会重新接管发音链接, 一律剥离; 内联 on* 事件一并清除
        html_text = re.sub(r"(?is)<script\b.*?</script\s*>", "", html_text)
        html_text = re.sub(r"(?is)<script\b[^>]*/?>", "", html_text)
        html_text = re.sub(
            r"(?i)\s+on[a-z]+\s*=\s*(\"[^\"]*\"|'[^']*'|[^\s>]+)",
            "", html_text
        )
        # dic select patch: 清理禁选属性/内联样式, 允许选中复制
        html_text = re.sub(r'''(?i)\s+unselectable\s*=\s*['"]?(on|true)['"]?''',
                           "", html_text)
        html_text = re.sub(r'''(?i)(style\s*=\s*['"][^'"]*?)user-select\s*:\s*none\s*;?''',
                           r"\1", html_text)
        try:
            root = lh.fromstring(html_text)
        except Exception:
            root = None
        if root is not None:
            snd = re.compile(r"(?i)^sound://|\.mp3(\?|#|$)")
            for el in list(root.iter()):
                if not isinstance(el.tag, str):
                    continue
                tag = el.tag.split(":")[-1].lower()
                if tag == "img":
                    # (dic img patch) 词典图片 → dicres:// 从 MDD 直读
                    v = (el.get("src") or "").strip()
                    if v and not _RES_SKIP.match(v):
                        el.set("src", "dicres://"
                               + v.replace("\\", "/").lstrip("/"))
                    el.attrib.pop("srcset", None)
                ref = ""
                for attr, val in list(el.attrib.items()):
                    a = attr.lower()
                    if a not in ("href", "src", "data-src-mp3",
                                 "src-mp3", "data-sound"):
                        continue
                    v = (val or "").strip()
                    if a == "data-sound":
                        ref = ref or v
                    elif snd.search(v) or a in ("data-src-mp3", "src-mp3"):
                        if not ref:
                            ref = re.sub(r"(?i)^sound://", "", v)
                        el.attrib.pop(attr, None)
                if ref:
                    el.set("data-sound", ref)
                if tag == "a":
                    for junk in ("href", "target", "onclick"):
                        el.attrib.pop(junk, None)
            # dic sound-propagate patch: 把发音引用提升到 pron 容器,
            # 点击音标文字/整行也能发音 (GoldenDict 风格)
            for _el in list(root.iter()):
                _ref = _el.get("data-sound")
                if not _ref:
                    continue
                _p = _el.getparent()
                if _p is None or _p.get("data-sound"):
                    continue
                if "pron" not in (_p.get("class") or "").lower():
                    continue
                _refs = {s.get("data-sound") for s in _p.iter()
                         if s.get("data-sound")}
                if len(_refs) == 1:
                    _p.set("data-sound", _ref)
            # dic nested-a patch: HTML 不允许 <a> 嵌套, WebKit 解析时会截断
            # 外层 data-sound 锚点(音标/图标失去可点性); 把内层 <a> 改名 span
            for _a in list(root.iter('a')):
                _p = _a.getparent()
                while _p is not None:
                    if isinstance(_p.tag, str) and _p.tag.lower() == 'a':
                        _a.tag = 'span'
                        break
                    _p = _p.getparent()
            html_text = lh.tostring(root, encoding="unicode")
        # 兜底: 清掉一切残留的可导航 sound:// 引用(含未加引号写法)
        html_text = re.sub(
            r"(?i)\s+href\s*=\s*[\"']?sound://[^\s>\"']*",
            "", html_text
        )
        html_text = re.sub(
            r"(?i)\s+src\s*=\s*[\"']?sound://[^\s>\"']*",
            "", html_text
        )
        return html_text

    def _dump(html_text, css):
        if not os.environ.get("DIC_DUMP"):
            return
        try:
            Path("/tmp/dic_last.html").write_text(html_text, encoding="utf-8")
            Path("/tmp/dic_last.css").write_text(css, encoding="utf-8")
        except OSError:
            pass

    class Api:
        def lookup(self, word):
            word = (word or "").strip()
            if not word:
                return {"html": "", "css": ""}
            # dic online patch: 短语/句子 -> 本窗口打开在线词典(不再切 Qt)
            if not _is_single_word(word):
                return _goto_online(word)
            r = lookup(word)
            if not r["found"]:
                t = translate_text(word)
                return {
                    "html": (
                        "<p><b>%s</b>（本地未收录，在线翻译）</p><p>%s</p>"
                        % (_esc(word), _esc(t))
                    ),
                    "css": "",
                }
            state["mdx"] = r.get("_mdx")
            state["word"] = r.get("word")
            html_text = _prepare(r["_html"])
            css = _css_localize(r.get("css", "")) + "\n" + OXFORD9_CSS
            _dump(html_text, css)
            # 不加载词典自带 JS，避免其重新接管发音链接。
            return {
                "html": html_text,
                "css": css,
            }

        def play(self, ref):
            import traceback as _tb
            mdx = state.get("mdx")
            word = state.get("word") or ""

            def L(*a):
                try:
                    with open("/tmp/dic_play.log", "a", encoding="utf-8") as f:
                        f.write(time.strftime("[%H:%M:%S] ") + " ".join(
                            str(x) for x in a) + "\n")
                except OSError:
                    pass

            print("[dic.play]", ref, "| word:", word, file=sys.stderr)
            L("play called:", repr(ref), "| word:", repr(word))
            if not mdx or not ref:
                return "⛔ play: 内部状态缺失"
            ref = str(ref).strip()
            ref = re.sub(r"(?i)^[a-z][a-z0-9+.-]+://", "", ref)
            audio_path = None
            try:
                audio_path = extract_audio(mdx, ref)
            except Exception:
                L("提取异常:", _tb.format_exc())
            if not audio_path:
                L("未命中 %r, 词条兜底: %r" % (ref, word))
                for variant in ("gb", "us"):
                    try:
                        audio_path = get_playable(word, variant) if word else None
                    except Exception:
                        audio_path = None
                    if audio_path:
                        L("兜底命中:", variant)
                        break
            if not audio_path:
                L("最终无音频:", ref)
                return "⛔ 找不到音频: %s" % ref
            played = _play_audio(audio_path)
            L("播放:", audio_path, "成功" if played else "失败")
            return ("🔊 " + os.path.basename(audio_path)) if played else "⛔ 播放失败"

        def online_open(self, idx):
            """dic online patch: 在线词典窗口内切换站点"""
            try:
                idx = int(idx)
            except Exception:
                idx = 0
            urls = online_site_urls(state.get("last_query") or "")
            if not urls:
                return "⛔ 无查询内容"
            if idx < 0 or idx >= len(urls):
                idx = 0
            state["online"] = True
            try:
                _online_navigate(window, urls[idx])
                return "🌐 " + _online_site_names()[idx]
            except Exception as exc:
                return "⛔ 打开失败: %s" % exc

        def online_home(self):
            """dic online patch: 返回本地词典查询页"""
            state["online"] = False
            w = state.get("word") or ""

            def _go():
                try:
                    window.load_html(_entry_page(
                        w,
                        '<script>(function w(){'
                        'if(window.pywebview&&pywebview.api){'
                        'try{doLookup();}catch(e){}}'
                        'else{setTimeout(w,200);}})();</script>',
                        OXFORD9_CSS,
                        float(_load_cfg().get("zoom") or 1.0)))
                except Exception:
                    pass
            _idle_call(_go)
            return True

        def open_external(self, idx):
            """(dic stack patch) 用系统浏览器打开第 idx 个在线站点"""
            try:
                idx = int(idx)
            except Exception:
                idx = 0
            urls = online_site_urls(state.get("last_query") or "")
            if 0 <= idx < len(urls):
                import webbrowser, threading
                threading.Thread(
                    target=lambda: webbrowser.open(urls[idx]),
                    daemon=True).start()
            return True

        def devtools(self):
            """(dic devtools patch) Ctrl+Shift+I: 显示/隐藏 WebKit 检查器"""
            def _go():
                wk = state.get("webkit")
                if wk is None:
                    return
                try:
                    wk.get_settings().props.enable_developer_extras = True
                except Exception:
                    pass
                try:
                    insp = wk.get_inspector()
                    if state.get("devtools"):
                        insp.close()
                        state["devtools"] = False
                    else:
                        insp.show()
                        state["devtools"] = True
                except Exception:
                    pass
            _idle_call(_go)
            return True

        def close(self):
            try:
                webview.windows[0].destroy()
            except Exception:
                pass
            return True

        def set_zoom(self, level):
            """原生 WebKit 缩放 + 持久化 (dic zoom save patch)"""
            try:
                level = min(3.0, max(0.5, float(level)))
            except Exception:
                return False
            wk = state.get("webkit")
            if wk is not None:
                try:
                    from gi.repository import GLib
                    GLib.idle_add(wk.set_zoom_level, float(level))
                except Exception:
                    try:
                        wk.set_zoom_level(float(level))
                    except Exception:
                        pass
                _save_cfg_merged({"zoom": level})
            return True

    def _entry_page(word="", body="", css="", zoom=1.0):
        return (
            WEB_SHELL
            .replace("__WORD__", _esc(word))
            .replace("__CONTENT__", body)
            .replace("__ZOOM__", repr(float(zoom)))
            .replace(
                '<style id="dict-css"></style>',
                '<style id="dict-css">' + css + '</style>',
            )
        )

    initial = (initial_text or "").strip()
    body = ""
    word0 = ""
    css0 = OXFORD9_CSS
    if initial and _is_single_word(initial):
        word0 = initial
        r = lookup(initial)
        if r["found"]:
            state["mdx"] = r.get("_mdx")
            state["word"] = r.get("word")
            body = _prepare(r["_html"])
            css0 = _css_localize(r.get("css", "")) + "\n" + OXFORD9_CSS
            _dump(body, css0)
        else:
            body = "<p>%s</p>" % _esc(translate_text(initial))
    elif initial:
        # (dic stack patch) 非单词 -> 直接把拼接式在线词典作为初始内容
        state["last_query"] = initial
        state["online"] = False
        body = _online_stacked_html(initial)

    # dic 恢复上次窗口尺寸/缩放 (dic cfg patch)
    _cfgd = _load_cfg()
    zoom0 = 1.0
    try:
        zoom0 = min(3.0, max(0.5, float(_cfgd.get("zoom") or 1.0)))
    except Exception:
        pass

    def _cfg_int(key, dft, lo, hi):
        try:
            return max(lo, min(hi, int(float(_cfgd.get(key) or dft))))
        except Exception:
            return dft

    w0, h0 = _cfg_int("width", 760, 320, 7680), _cfg_int("height", 860, 300, 4320)
    window = webview.create_window(
        "dic",
        html=_entry_page(word0, body, css0, zoom0),
        js_api=Api(),
        width=w0,
        height=h0,
    )

    # dic online patch: 远程词典页加载完后注入站点切换工具条
    def _on_loaded():
        if state.get("online"):
            try:
                window.evaluate_js(_online_inject_js(
                    state.get("last_query") or ""))
            except Exception:
                pass
    try:
        window.events.loaded += _on_loaded
    except Exception:
        pass

    # pywebview 有原生窗口的情况下，直接挂 Qt Esc 兜底；
    # JS 同时也会处理 Esc，因此不同 backend 都能关闭。
    try:
        def _on_closed():
            pass
        window.events.closed += _on_closed
    except Exception:
        pass

    _register_dicres_scheme()
    webview.start(_native_hook, args=(window, state))


def run_cli(content):
    content = (content or "").strip()
    if not content:
        return 1
    if not _is_single_word(content):
        # dic online patch: 短语/句子 -> 弹出 WebView 在线词典窗口
        print("🌐 非单词输入: 打开在线词典窗口…")
        run_gui_web(initial_text=content, auto_switch=False)
        return 0
    r = lookup(content)
    if not r["found"]:
        out = translate_text(content)
        print(out)
        return 0 if out else 1
    print(f"--- {r['dict']} | {r['word']}\n")
    print(r["text"])
    for variant in ("gb", "us"):
        path = get_playable(r["word"], variant)
        if path:
            print(f"  🎧 {path}")
    return 0


def main():
    args = sys.argv[1:]
    if not args:
        print('用法: ./dic [-g|-w|-t] "<content>"', file=sys.stderr)
        return 1
    if args[0] == "--native":
        content = " ".join(args[1:]).strip()
        if content:
            run_gui(initial_text=content, auto_switch=True)
        return 0
    if args[0] == "--web":
        content = " ".join(args[1:]).strip()
        if content:
            run_gui_web(initial_text=content, auto_switch=True)
        return 0
    if args[0] in ("-g", "--gui"):
        content = " ".join(args[1:]).strip()
        if not content:
            print('用法: ./dic -g "<content>"', file=sys.stderr)
            return 1
        # 单词→本地词典, 短语/句子→在线词典, 均在 WebView 内
        run_gui_web(initial_text=content, auto_switch=True)
        return 0
    if args[0] in ("-w", "--webview"):
        content = " ".join(args[1:]).strip()
        if not content:
            print('用法: ./dic -w "<content>"', file=sys.stderr)
            return 1
        run_gui_web(initial_text=content, auto_switch=False)
        return 0
    if args[0] in ("-t", "--text"):
        content = " ".join(args[1:]).strip()
        if not content:
            print('用法: ./dic -t "<content>"', file=sys.stderr)
            return 1
        return run_cli(content)
    if args[0] in ("-h", "--help"):
        print(
            '用法:\n'
            '  ./dic "<content>"   单词→WebView 本地词典; 短语/句子→WebView 在线词典\n'
            '  ./dic -g "<content>" 同上\n'
            '  ./dic -w "<content>" 强制 WebView (不做自动切换)\n'
            '  ./dic -t "<content>" 命令行模式 (短语时弹 WebView 在线词典窗口)\n'
            '  ./dic --native "<content>" 强制原生 Qt 窗口'
        )
        return 0
    content = " ".join(args).strip()
    if not content:
        return 1
    # 单词→本地词典, 短语/句子→在线词典, 均在 WebView 内
    run_gui_web(initial_text=content, auto_switch=True)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except SystemExit:
        raise
    except BaseException:
        import traceback as _tb
        try:
            with open("/tmp/dic-crash.log", "a", encoding="utf-8") as _f:
                _f.write(time.strftime("[%F %T] ") + _tb.format_exc() + "\n")
        except Exception:
            pass
        raise
