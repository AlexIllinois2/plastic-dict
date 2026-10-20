//! GUI 样式: 应用外壳 CSS + 牛津词典词条修正 CSS (移植自 TS 版 shell.ts)

/// 应用外壳 (工具栏/内容区/Toast) — 与原版 #bar/#content/dic-toast 视觉一致
pub const SHELL_CSS: &str = r#"
html, body { margin: 0; padding: 0; width: 100%; height: 100%; }
.root { width: 100%; height: 100%; display: flex; flex-direction: column;
        font-family: system-ui, 'Noto Sans CJK SC', 'Microsoft YaHei', '微软雅黑',
                     'Segoe UI Emoji', 'Noto Color Emoji', sans-serif;
        font-size: 15px; color: #222; background: #fff; }
.toolbar { display: flex; align-items: center; gap: 6px; padding: 8px;
           background: #f5f5f5; border-bottom: 1px solid #ddd; }
.toolbar input { flex: 1; min-width: 140px; font-size: 15px; padding: 6px 10px;
                 border: 1px solid #ccc; border-radius: 6px; background: #fff; color: #222; }
.toolbar button, .toolbar a { padding: 6px 12px; border: 1px solid #ccc; border-radius: 6px;
                 background: #fff; cursor: pointer; font-size: 14px;
                 text-decoration: none; color: #222; white-space: nowrap;
                 flex: 0 0 auto; }
.toolbar button:hover, .toolbar a:hover { background: #eee; }
.toolbar .zoom-val { border: none; background: none; padding: 0 2px; min-width: 44px;
                 text-align: center; font-size: 13px; color: #555; cursor: default; }
.content { flex: 1; overflow-y: auto; padding: 14px 18px; }
.content img { max-width: 100%; }
[data-sound] { cursor: pointer; }
.hint { color: #888; padding: 24px 8px; line-height: 1.8; }
.toast { position: fixed; right: 12px; top: 52px; background: #333; color: #fff;
         padding: 6px 10px; border-radius: 6px; font-size: 12px; z-index: 999;
         max-width: 70%; word-break: break-all; white-space: pre-wrap; }
.online-page { border-bottom: 1px solid #e3e3e3; padding-bottom: 16px; margin-bottom: 16px; }
.online-page h3 { margin: 12px 0 6px; font-size: 15px; }
.online-page .open-link { display: inline-block; margin: 0 0 8px; font-size: 13px; }
"#;

/// 牛津词条 CSS 修正 (移植自 TS OXFORD9_CSS, #content → .content)
/// 注意: 该 CSS 会与词典自带 CSS 拼接后一起交给 prepare_entry 做字号缩放,
///       因此这里用普通选择器, 不用 !important 保护 (与原版一致的部分除外)。
///       拼接顺序是 词典CSS → 本 CSS → config/extra.css, 同特异性下后者覆盖前者。
pub const OXFORD9_CSS: &str = r#"
pron-g-blk br { display: none !important; }
[data-sound] { cursor: pointer; text-decoration: none; }
.content { font-size: 17px; line-height: 1.65; }
.content, .content * { user-select: text; }
[data-sound], [data-sound] * { cursor: pointer; }

/* --- Blitz 渲染适配 (GoldenDict/WebKit 下不需要) --- */

/* Blitz 只对 emoji 做字形回退, 普通符号缺字形就直接画豆腐块。
   词典用 <sdsymb>➤</sdsymb> 和 <arrow>➡</arrow> 当小节/参见标记,
   Optima/Georgia 都没有这两个字形 → 显式给一个有覆盖的字体。 */
sdsymb, sdsymb a, arrow { font-family: 'Segoe UI Symbol', 'Segoe UI', sans-serif; }

/* 例句前的箭头是 x-wr:before 的 content:"\021D2" (⇒), 同理需要指定字体 */
x-wr:before { font-family: 'Segoe UI Symbol', 'Segoe UI', sans-serif; }

/* 英/美两套音标粘成 /rʌn//rʌn/: 词典 CSS 把 BrE/NAmE 标签 display:none,
   注释里靠 <br> 分行, 而 br 又被上面第一条去掉了。把标签显示出来即可区分.
   标签之间的空隙由 prepare::sanitize_html 插入的空白提供 —— Blitz 0.2.4
   不给 inline 盒算 padding/margin, CSS 间距在这里是无效的。 */
pron-g-blk brelabel, pron-g-blk namelabel { display: inline; }

/* 词典把释义/例句里每个词都包成 <a>; Blitz 的 UA 样式表是 a:link 级别
   (0,1,1) 的"链接蓝 + 下划线", 特异性压过词典自己的 a{color:...},
   于是整段正文每个词都带下划线、发蓝。还原成正文外观, 悬停才给下划线。 */
a, a:link, a:visited { color: inherit; text-decoration: none; }
a:hover { text-decoration: underline; }

/* 英文释义与中文译文之间的 <chnsep> 就是一个空格, 会被空白折叠吃掉
   (渲染成 "flesh苹果"), 改成固定宽度的空隙 */
chnsep { display: inline-block; width: .3em; }

/* 词条里的关键字标记: HTML 自带 🔑 emoji, CSS 又用 coresym 的 \e900 靠
   margin-left:-12.5px 叠上去。emoji 比 12.5px 宽, 叠不准 → 一大一小两个钥匙并排。
   直接去掉图标字覆盖层, 只留 emoji (不依赖 webfont, 也不会缺字形)。 */
symbol[type=key]:after { display: none; }

/* 词典把例句写成 inline-block, 里面还套一个 display:block 的中文译文 chn。
   Blitz 算 inline-block 的收缩宽度时会被内部块级子元素带偏, 结果是英文例句
   提前折行、箭头被挤到单独一行。改成纯 inline, 换行交给原文自带的 <br>。 */
x-g-blk x { display: inline; }
x-g-blk x chn { display: inline; }

/* 小节标题 (➤ move fast on foot): 词典给 shcut-blk 设了 color:#fff, 蓝底却挂在
   子元素 sdsymb/shcut 的 inline 背景上 (shcut-blk sdsymb,shcut-blk shcut{background:#08d})。
   Blitz 不画这层 inline 背景, 于是白字白底 → 整块标题看不见。把底色提到块级
   shcut-blk 上, 恢复成"白字蓝条"的小节标题。 */
shcut-blk { background: #08d; padding-left: 5px; }
"#;
