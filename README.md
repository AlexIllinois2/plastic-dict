# dic (Bun + TypeScript 版)

原 Python 单文件词典脚本的 Bun/TS 重写版。**去掉了 Qt/FreeSimpleGUI 原生窗口**，
只用系统 WebView（webview-bun 的预编译 `libwebview` 直接链接系统
**libwebkitgtk-6.0 / GTK4**，不捆绑任何浏览器内核），配合 CLI 模式。

功能与原版一致：

- 单词 → 本地 MDX 词典（Oxford9 / OALD9 等，`js-mdict` 解析）
- 短语/句子 → 同一 WebView 窗口内打开在线词典站（GoldenDict 风格
  `%GDWORD%` 模板，默认有道 + 必应，上下拼接）
- 本地未收录 → Bing 免 token 在线翻译（`edge.microsoft.com` 端点）
- 发音：点击音标/喇叭图标从 MDD 提取 mp3 并调用系统播放器
  （mpv / ffplay / cvlc / paplay / aplay 任一）
- 词典 CSS/图片经内置本地 HTTP 服务注入（替代原 `dicres://` 协议）
- 缩放 Ctrl+=/-/0（持久化）；Esc 关窗；Ctrl+Shift+I 提示用右键"检查元素"
- **新增**：词典索引磁盘缓存（`~/.cache/dic/index/`），CLI 冷启动无需
  每次重新解析全量词条

## 安装

```bash
# 1) 系统依赖 (GTK4 版 WebKitGTK 运行时)
sudo ./install-deps.sh
# 或手动:
#   Debian 13 / Ubuntu 24.04+: sudo apt install libwebkitgtk-6.0-4 fonts-noto-cjk mpv
#   Fedora 39+:                sudo dnf install webkitgtk6.0 google-noto-sans-cjk-ttc-fonts mpv

# 2) npm 依赖
bun install
```

> ⚠️ 预编译 webview 库链接的是 **GTK4** 的 `libwebkitgtk-6.0.so.4`。
> Ubuntu 22.04 / Debian 12 没有 GTK4 版 WebKitGTK。若必须用老发行版的
> webkit2gtk-4.1 (GTK3)，可自行编译 webview 库并用 `WEBVIEW_PATH` 指向它：
> ```bash
> git clone --recursive https://github.com/webview/webview && cd webview
> cmake -B build -DCMAKE_BUILD_TYPE=Release -DWEBVIEW_BUILD_SHARED_LIBRARY=ON \
>       -DWEBVIEW_WEBKITGTK_API=4.1
> cmake --build build -j
> # 之后运行: WEBVIEW_PATH=/path/to/webview/build/library/libwebview.so ./dic.ts ...
> ```

## 离线词典

把 https://github.com/yanyingwang/goldendict 的百度网盘 `dicts/Oxford9`
移到 `~/.local/share/golden-dict/Oxford`（含 `.mdx/.mdd`，支持 `词干.N.mdd`
多分卷）。用环境变量 `GOLDENDICT_DIR` 可指向其它目录。

## 用法

```bash
bun dic.ts "apple"        # 单词 → WebView 本地词典
bun dic.ts "long time no see"   # 短语 → WebView 在线词典(拼接页)
bun dic.ts -t "apple"     # CLI 纯文本输出(未收录自动 Bing 翻译)
bun dic.ts -t "hello world"     # CLI: 短语会弹出 WebView 在线词典窗口
bun dic.ts --purge-cache  # 清空索引缓存
bun dic.ts -h             # 帮助
```

快捷键（替换原 Python 版入口）：

```bash
bash -lc 'bun ~/.local/script/dic.ts "$(/usr/bin/wl-paste -n -p)"'
```

（注意：入口需要 `cd` 到本目录或用绝对路径，`bun dic.ts` 依赖同目录
`node_modules`。）

## 配置 `~/.config/dic/config.json`

```json
{
 "zoom": 1.0,
 "width": 760,
 "height": 860,
 "maximized": false
}
```

- `zoom`：Ctrl+=/-/0 自动保存（0.5~3.0），也可手工改
- `width`/`height`：窗口大小（手工配置；本版没有 GTK 原生钩子，
  不再自动记忆窗口几何，关闭时是什么下次仍按配置来）
- `maximized`：`true` 时启动即最大化（通过 GTK4 FFI 实现）

## 环境变量

| 变量 | 说明 |
|---|---|
| `GOLDENDICT_DIR` | 词典目录（默认 `~/.local/share/golden-dict/Oxford`） |
| `DIC_DUMP` | 置 1 时把最近一次词条 HTML/CSS 写到 `/tmp/dic_last.html|css` |
| `WEBVIEW_PATH` | 指向自编译 webview 动态库（老发行版 GTK3 路线） |

## 与原 Python 版的差异

| 项 | 原版 | 本版 |
|---|---|---|
| GUI | pywebview(WebKitGTK4.x) + PySide6/FreeSimpleGUI 双窗口 | 仅系统 WebView 窗口（GTK4 WebKitGTK6.0） |
| `--native` | 强制 Qt 原生窗口 | 已移除（提示后转 WebView） |
| dicres:// | WebKit 自定义协议 | 内置 127.0.0.1 随机端口 HTTP 服务 |
| 词典解析 | mdict-mquery（sqlite 缓存） | js-mdict + 自实现磁盘索引缓存（`~/.cache/dic/index`） |
| HTML 处理 | lxml | htmlparser2 + dom-serializer |
| 窗口尺寸记忆 | GTK 钩子自动记忆 | 改为配置文件手工配置（width/height/maximized） |
| DevTools | Ctrl+Shift+I 原生 Inspector 开关 | 右键 → 检查元素（debug 模式自带） |
| Esc 关窗 | GTK 钩子兜底（iframe 内也有效） | 仅 JS 层（焦点在跨域 iframe 内时可能失效） |
| 缩放 | WebKit 原生 zoom_level | CSS zoom（效果等同） |

## 故障排查

- **启动报 `libwebkitgtk-6.0.so.4: cannot open shared object file`**：
  未装 GTK4 运行时，见安装步骤 1；或老发行版请走 `WEBVIEW_PATH` 自编译路线。
- **CLI 启动慢**：首次会构建索引缓存（打印 `[构建] 索引缓存(一次性): …`），
  之后走缓存。词典文件更新后缓存自动失效重建。
- **发音失败**：确认装有 mpv/ffplay/vlc 之一；日志在 `/tmp/dic_play.log`。
- **崩溃**：日志在 `/tmp/dic-crash.log`。
