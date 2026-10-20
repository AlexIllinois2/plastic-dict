# plastic-dict

丐版 GoldenDict — **Rust + Dioxus Native (Blitz) 重写版**。

本地 MDX/MDD 词典（牛津 9）+ Bing 在线翻译 + 在线词典，打包为**单一可执行文件**：
没有 Bun/Node 运行时，没有 GTK/WebKit 系统依赖，也没有 JS 引擎——GUI 的
HTML/CSS 由 Blitz（Stylo + Taffy + Vello/WGPU）原生渲染。

**便携包模式**：词典、配置、缓存、日志等所有数据都放在本二进制所在目录下，
整个目录拷走即迁移，不写注册表、不写用户主目录。

## 功能

- **本地词典**：MDX v1.2 / v2.0 + MDD 资源库
  - zlib 压缩块；LZO / 加密（Salsa20、fast-XOR 索引加密不支持）给出明确报错
  - 磁盘索引缓存 `cache/index`（自研紧凑格式，词典
    size/mtime 变化自动失效重建）
  - 多分卷 MDD（`词干.mdd` / `词干.1.mdd` / …），图片内联为 `data:` URI
  - 词典 CSS 里内嵌/放在 MDD 里的 webfont（如牛津的 Optima）启动时自动注册
  - 发音按钮：Windows 走 winmm MCI（进程内播放）；其它平台依次尝试
    `mpv` → `paplay` → `aplay` → `ffplay` → `cvlc`，最后交给系统默认程序
- **在线翻译**：Bing Edge API（CLI 查不到词时自动回退）
- **在线词典**：有道 / 必应——服务端抓取页面 → 净化 → 阅读器排版，
  并可一键"在浏览器中打开"（Blitz 无 JS 引擎/iframe，见下方差异表）
  - 净化会摘掉导航/页头页脚/登录/搜索框/目录条/广告位、空壳块（含空的表格
    单元格）和固定高度内联样式，远程图片一并丢掉（渲染端只有 `data:` 通道，
    留着只会按 `width/height` 占一片空白）
- **GUI**：查词框、音标与发音按钮、Ctrl 缩放、站点快链、多页堆叠
- **CLI 纯文本模式**（`-t`）：适合脚本/快捷键取词

## 构建

```bash
# Windows（必须 MSVC 工具链，GNU 工具链缺 gcc/dlltool）
cargo +stable-x86_64-pc-windows-msvc build --release

# Linux / macOS
cargo build --release
```

产物是 `target/release/plastic-dict`（Windows 下为 `.exe`），单文件、
无系统 GUI/浏览器依赖。Rust 版本要求见 `Cargo.toml` 的 `rust-version`。

## 便携包目录结构

```
plastic-dict(.exe)      # 二进制本体
dicts/                  # 词典 (.mdx/.mdd, 同名 .css 自动拼接; 支持 词干.N.mdd 多分卷)
config/config.json      # 配置 (zoom/width/height/maximized)
config/extra.css        # 可选: 用户 CSS 补丁, 追加在词典 CSS 之后
cache/index/            # 索引缓存 (.pdc, 词典 size/mtime 变化自动失效)
cache/audio/            # 发音落地
logs/                   # crash.log / play.log / DIC_DUMP 输出
```

把 [goldendict](https://github.com/yanyingwang/goldendict) 百度网盘
`dicts/Oxford9` 里的内容放进 `dicts/` 即可（首次启动也会自动建好这些目录）。
用环境变量 `GOLDENDICT_DIR` 可把词典指到其它目录（不动包内结构时使用）。

CSS 覆盖顺序是 **词典自带 CSS → 程序内置修正（`src/gui/style.rs` 的
`OXFORD9_CSS`）→ `config/extra.css`**，同特异性下后者生效。词典某个元素的排版
在 Blitz 下不对时，不用改 `.mdx/.mdd`，在 `config/extra.css` 里加规则即可。

## 用法

```bash
plastic-dict "apple"             # 单词 → GUI 本地词典
plastic-dict "long time no see"  # 短语/中文 → GUI 在线词典
plastic-dict -t "apple"          # CLI 纯文本（未收录自动 Bing 翻译）
plastic-dict -t "hello world"    # CLI：短语仍会弹出 GUI 在线词典窗口
plastic-dict --purge-cache       # 清空索引缓存
plastic-dict -h                  # 帮助
```

快捷键（GUI）：

| 键 | 作用 |
|---|---|
| `Enter` | 查词 |
| `Esc` | 关闭窗口 |
| `Ctrl` `=` / `+` | 放大 |
| `Ctrl` `-` | 缩小 |
| `Ctrl` `0` | 恢复 100% |

缩放值（0.5~3.0）自动存入配置文件；工具栏也有 `-` / 百分比 / `+` 按钮。

## 配置 `config/config.json`

```json
{
  "zoom": 1.0,
  "width": 760,
  "height": 860,
  "maximized": false
}
```

- `zoom`：快捷键/工具按钮自动保存
- `width` / `height`：窗口大小（手工配置；不再自动记忆几何）
- `maximized`：`true` 时启动即最大化

## 环境变量

| 变量 | 说明 |
|---|---|
| `GOLDENDICT_DIR` | 词典目录（默认 `<exe>/dicts`，支持 `~`） |
| `DIC_DUMP` | 置 `1` 时把最近一次词条（净化后）的 HTML/CSS 写到 `logs/last.html` / `logs/last.css`；在线词典页写到 `logs/last_web_0.html`（有道）/ `last_web_1.html`（必应），并打印 `[web]` 剥离子项计数 |

## Blitz 渲染适配

Blitz 0.2 是"能排版但不完整"的引擎，下面是这台引擎上实测出来的差异，以及本程序
的处理方式（`src/gui/style.rs` 与 `src/gui/fonts.rs`）：

| 问题 | 现象 | 处理 |
|---|---|---|
| 表格布局越界崩溃 | 词典 CSS 的 `display:table*` 触发 `blitz-dom` panic | `localize_css` 统一降级：`table-cell`→`inline-block`，其余→`block` |
| `@font-face` 对内联 `<style>` 不生效 | 内嵌的 Optima/oalecd9 不加载，字体退回衬线 | 启动时扫词典 CSS，按 CSS 里的 family 名 + 字重/字形自己注册（`gui/fonts.rs`） |
| 无逐字形回退（只有 emoji 有） | `⇒` `➤` 等符号画成豆腐块 | 对这些标记元素显式指定 `Segoe UI Symbol` 系列字体 |
| inline 盒的 `padding`/`margin` 不生效 | `/rʌn//rʌn/`、`[I]to move`、`verb,noun` 挤在一起 | 在净化阶段插入真实空格（`prepare::sanitize_html`），不能用 CSS 间距 |
| 行内盒首尾空白被裁掉 | 空格塞进行内盒尾部会被当"行尾"删掉 | 空格必须落在两个盒之间（`</gram-g> <def>`），不能塞进盒内 |
| 不绘制普通 inline 盒的背景 | 小节标题 `shcut-blk` 是白字 + 挂子元素上的蓝底 → 白字白底整块隐形 | 把底色提到块级元素上（`shcut-blk { background: #08d }`） |
| UA 样式表按 `a:link` 给链接上色/加下划线 | 词典正文每词都是 `<a>`，整段文字像网页链接一样花 | `a, a:link, a:visited { color: inherit; text-decoration: none }`，悬停才给下划线 |
| 不支持 `float` | 图片/词族框不会浮动，按块级排版 | 接受（内容不丢，只是位置与 GoldenDict 不同） |
| 没有滚动条 | 长词条看不到滚动位置 | 滚轮/触摸板可滚动（`blitz-shell` 已处理 MouseWheel），无条可拖 |
| 不支持 `zoom` | 缩放会失效 | 按倍数改写 CSS 里的 px/pt 字号 |

## 与 Bun + WebView 版的差异

原 Bun/TypeScript 版保留在 `bak/`（更早的 Python/TS 单文件）与 `legacy/`
（Bun 版）目录，仅供对照。

| 项 | 旧版（Bun + webview-bun） | 本版（Rust + Blitz） |
|---|---|---|
| 运行时 | Bun + node_modules（js-mdict 等 6 个依赖） | 单一 Rust 二进制 |
| GUI | GTK4 WebKitGTK WebView + 本地 HTTP 服务 + JS 注入 | Dioxus Native + Blitz 原生渲染，无 WebView、无 JS |
| 词典解析 | js-mdict（npm 包，已下架） | 自实现解析器（移植 js-mdict 7.0.0 逻辑，含格式细节修正） |
| 索引缓存 | JSON（sha1 路径键） | 自研紧凑二进制 `PDIDX02`（fnv1a 路径键） |
| 资源引用 | `127.0.0.1` 随机端口 HTTP 服务 `/res/...` | `data:` URI 内联（blitz-shell data-uri provider） |
| 在线词典 | iframe 堆叠（WebView 内渲染） | 服务端抓取 → 净化（剥导航/广告/空壳/远程图）→ 阅读器排版；外链走系统浏览器 |
| HTML 处理 | htmlparser2 + dom-serializer | html5ever + markup5ever_rcdom（自写序列化与样式提取） |
| 缩放 | WebKit 原生 `zoom_level` | CSS `font-size` 缩放（Blitz 不支持 zoom 属性） |
| Esc 关窗 | JS 层监听（焦点在跨域 iframe 内时失效） | 键盘事件直达根 div（`autofocus`），无 iframe 问题 |
| DevTools | 右键 → 检查元素 | 无 Inspector，用 `DIC_DUMP=1` 落盘排查 |
| 打包体积 | ~几十 MB + node_modules | release 约 30 MB 单文件 |

## 项目结构

```
src/
  main.rs        入口 / CLI 分发 / panic hook
  mdict/         MDX/MDD 解析 (parse.rs) + 词典池/查词 (mod.rs) + 索引缓存 (cache.rs)
  gui/           Dioxus Native 应用 (app.rs) + Blitz 启动与 net provider (mod.rs/net.rs)
                 + 样式与词典 CSS 修正 (style.rs) + 内嵌字体注册 (fonts.rs)
  textrender.rs  词条 HTML → CLI 纯文本
  prepare.rs     词条 HTML/CSS 净化与资源化 (data URI)
  dom.rs         html5ever 封装 (解析/序列化/属性)
  online.rs      在线词典站点抓取
  translate.rs   Bing 翻译
  config.rs / paths.rs / util.rs
examples/
  boxdump.rs     无头布局探针: 对 dump 出来的 HTML/CSS 跑一次布局并打印每个节点的盒子
tools/
  make_fixture.py  生成 MDX/MDD 测试夹具 (testdata/)
  grab_window.py   截取 GUI 窗口 (纯 ctypes GDI, 可指定窗口尺寸)
  crop_png.py      截图裁剪/放大 (纯 zlib, 便于像素级核对渲染)
  send_click.py    向窗口发一次点击 (验证按钮/发音等交互)
  send_wheel.py    向窗口发滚轮 (Blitz 无滚动条, 只能这样验证滚动)
testdata/       夹具词典 (3 词条 + 2 资源, 2 key blocks + 2 record blocks)
shots/          GUI 验收截图（真实牛津 9: run / apple / 中文在线页）
bak/ legacy/    旧版源码 (Python / Bun+TS), 仅供参考
```

## 测试

```bash
cargo test                              # 38 个单元测试
python tools/make_fixture.py            # 重新生成 testdata/ 夹具
```

夹具覆盖 MDX v2.0 多块解析、跨 record block 查词、大小写不敏感、
MDD 资源定位、CSS `url()` 三种引号形态的 data URI 重写、
`@font-face` 解析（内嵌 base64 / 路径 / 字重字形）、在线页噪声剥离等端到端路径。

渲染问题排查用无头探针，比截图猜快得多：

```bash
DIC_DUMP=1 ./plastic-dict "apple"
cargo run --release --example boxdump -- \
    logs/last.html logs/last.css 760 860 --min-x=600
```

它按 `div.root > .toolbar + .content > [style, 词条]` 复刻 GUI 结构，打印每个节点的
`x/y/w/h` 与文本片段，能直接看出是哪个元素溢出了行宽。

## 故障排查

- **CLI 首次启动慢**：首次会构建索引缓存，之后走缓存；词典文件更新后
  缓存自动失效重建。`--purge-cache` 可手动清空。
- **启动比查词还慢**：启动时要扫词典 CSS 注册内嵌 webfont，会顺带加载一次
  MDD 索引（实测牛津 9 约 0.1s，缓存命中时）；没有 `@font-face` 的词典不触发。
- **提示 LZO / 加密不支持**：该词典用了暂不支持的压缩或加密，需先用
  其它工具转换（如重新打包为 zlib）。
- **发音没声音**：Windows 用系统 MCI，一般无需额外组件；Linux 需装
  `mpv`/`ffplay`/`vlc` 之一。日志在 `logs/play.log`。
- **崩溃**：日志在 `logs/crash.log`。
- **渲染异常**：`DIC_DUMP=1 plastic-dict "word"` 后看 `logs/last.html` /
  `logs/last.css`，再用 `examples/boxdump.rs` 打印布局盒子；纯样式问题优先在
  `config/extra.css` 里覆盖。注意 Blitz 对 inline 盒的 `padding`/`margin`
  不生效，需要间距只能在 HTML 里插空格。
- **在线页仍是整页导航**：`DIC_DUMP=1` 会落 `logs/last_web_0.html`（有道）/
  `last_web_1.html`（必应），看残留元素的 class/id，补进
  `prepare::WEB_NOISE_EXACT` / `prune_web_noise` 的关键字表。
