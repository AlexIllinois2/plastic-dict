import { e2 } from "./utils";

export function webShell(word: string, content: string): string {
  return `<!DOCTYPE html>
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
<input id="q" value="${e2(word, true)}" placeholder="输入单词后回车…" autocomplete="off" autofocus>
<button id="lookup-button">查词</button>
</div>
<div id="content">${content}</div>
<script>
var MEDIA_EXT = /\\.(mp3|wav|ogg|oga|aac|m4a|spx|opus|wma)(\\?|#|$)/i;
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
function stripScheme(s) { return String(s).replace(/^[a-z][a-z0-9+.\\-]*:\\/\\//i, ''); }
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
  dicApi('play', ref).then(function(r){ if (r) flash(String(r)); })
    .catch(function(err){ flash('⛔ play: ' + err); });
}
function handler(e) {
  if (e.__dicHandled) return;
  var _sel = window.getSelection && window.getSelection();
  if (_sel && _sel.type === 'Range' && String(_sel).length) return;
  e.__dicHandled = true;
  var node = e.target;
  if (!node || node.nodeType !== 1) return;
  while (node && node !== document) {
    if (node.nodeType === 1) {
      var ref = soundRef(node);
      if (ref) {
        e.preventDefault(); e.stopPropagation();
        if (e.stopImmediatePropagation) e.stopImmediatePropagation();
        flash('🔊 ' + ref); callPlay(ref); return;
      }
      if ((node.tagName || '').toLowerCase() === 'a') {
        var h = String(node.getAttribute('href') || '').trim();
        if (h && !/^(https?:|#|javascript:|mailto:|about:)/i.test(h)) {
          e.preventDefault(); e.stopPropagation();
          if (e.stopImmediatePropagation) e.stopImmediatePropagation();
          if (MEDIA_EXT.test(h.toLowerCase()) || h.toLowerCase().indexOf('sound://') === 0) {
            flash('🔊 ' + stripScheme(h)); callPlay(stripScheme(h));
          } else {
            flash('⛔ 非音频链接: ' + h + '\\n' + String(node.outerHTML || '').substring(0, 120));
          }
          return;
        }
        if (h) return;
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
      if (node.nodeType === 1 && soundRef(node) && node === e.target) { e.preventDefault(); return; }
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
      el.removeAttribute('href'); el.removeAttribute('target'); el.removeAttribute('onclick');
    }
  }
  var es = root.querySelectorAll('[data-src-mp3],[src-mp3]');
  for (var j = 0; j < es.length; j++) {
    var e2 = es[j];
    var ref = e2.getAttribute('data-src-mp3') || e2.getAttribute('src-mp3') || '';
    if (ref && !e2.getAttribute('data-sound')) e2.setAttribute('data-sound', stripScheme(ref));
  }
}
function fixSound(root) {
  if (!root || !root.querySelectorAll) return;
  var els = root.querySelectorAll('[data-sound]');
  for (var i = 0; i < els.length; i++) {
    var a = els[i];
    if (a.__dicDone) continue;
    a.__dicDone = true;
    var ref = a.getAttribute('data-sound') || '';
    if (!ref) continue;
    if (!String(a.textContent).trim() && !a.querySelector('img')) {
      var base = ref.replace(/\\.mp3$/i, ''), au = null;
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
          p.setAttribute('data-sound', ref); done = true;
        }
        break;
      }
    }
    if (!done) {
      var par = a.parentElement;
      if (par && par !== root && !par.getAttribute('data-sound')) {
        var ks = par.querySelectorAll('[data-sound]');
        if (ks.length === 1 && String(par.textContent).replace(/\\s+/g, '').length <= 40) {
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
var observer = new MutationObserver(function() { normalizeBoth(document.getElementById('content')); });
observer.observe(document.documentElement, {childList: true, subtree: true});
function applyCss(css) {
  var st = document.getElementById('dict-css');
  if (st) st.textContent = css || '';
}
function doLookup() {
  var word = document.getElementById('q').value.trim();
  if (!word) return;
  fetch('/api/lookup?q=' + encodeURIComponent(word))
    .then(function(r) { return r.json(); })
    .then(function(r) {
      if (!r || r.switch) return;
      document.getElementById('content').innerHTML = r.html || '';
      applyCss(r.css || '');
      normalizeBoth(document.getElementById('content'));
    }).catch(function(err) { flash('⛔ ' + err); });
}
document.getElementById('lookup-button').addEventListener('click', doLookup);
document.getElementById('q').addEventListener('keydown', function(e) {
  if (e.key === 'Enter') { e.preventDefault(); doLookup(); return; }
  if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); try { dicApi('close'); } catch (err) {} }
}, true);
normalizeBoth(document.getElementById('content'));
</script>
</body>
</html>`;
}

export const OXFORD9_CSS = `
pron-g-blk br { display: none !important; }
[data-sound] { cursor: pointer; text-decoration: none; }
#content { font-size: 17px !important; line-height: 1.65 !important; }
html, body, #content, #content * {
    -webkit-user-select: text !important;
    user-select: text !important;
}
[data-sound], [data-sound] * { cursor: pointer; }
`;

export const INIT_SCRIPT = `
(function(){
  function flash(msg) {
    var t = document.getElementById('dic-toast');
    if (!t) {
      t = document.createElement('div');
      t.id = 'dic-toast';
      t.style.cssText = 'position:fixed;right:12px;top:52px;background:#333;color:#fff;padding:6px 10px;border-radius:6px;font-size:12px;z-index:2147483600;max-width:70%;word-break:break-all;display:none;white-space:pre-wrap;';
      (document.body || document.documentElement).appendChild(t);
    }
    t.textContent = msg;
    t.style.display = 'block';
    clearTimeout(t._tm);
    t._tm = setTimeout(function(){ t.style.display = 'none'; }, 3500);
  }
  window.flash = flash;
  function applyZoom(z) { try { document.documentElement.style.zoom = z; } catch (err) {} }
  var DIC_ZOOM = 1.0;
  if (window.dicApi) {
    try { dicApi('getZoom').then(function(z){ DIC_ZOOM = Number(z) || 1.0; applyZoom(DIC_ZOOM); }); } catch (err) {}
  }
  document.addEventListener('keydown', function(e) {
    if (e.ctrlKey && e.shiftKey && (e.key === 'I' || e.key === 'i')) {
      e.preventDefault(); try { dicApi('devtools'); } catch (err) {} return;
    }
    if (e.key === 'Escape') {
      e.preventDefault(); try { dicApi('close'); } catch (err) {} return;
    }
    if (e.ctrlKey && !e.altKey && !e.metaKey) {
      var k = e.key;
      if (k === '=' || k === '+') { DIC_ZOOM = Math.min(3, +(DIC_ZOOM + 0.1).toFixed(2)); }
      else if (k === '-') { DIC_ZOOM = Math.max(0.5, +(DIC_ZOOM - 0.1).toFixed(2)); }
      else if (k === '0') { DIC_ZOOM = 1.0; }
      else if (k === 'ArrowDown') { window.scrollBy(0, Math.round(window.innerHeight*0.9)); e.preventDefault(); return; }
      else if (k === 'ArrowUp') { window.scrollBy(0, -Math.round(window.innerHeight*0.9)); e.preventDefault(); return; }
      else if (k === 'PageDown') { window.scrollBy(0, Math.round(window.innerHeight*0.9)); e.preventDefault(); return; }
      else if (k === 'PageUp') { window.scrollBy(0, -Math.round(window.innerHeight*0.9)); e.preventDefault(); return; }
      else if (k === 'Home') { window.scrollTo(0,0); e.preventDefault(); return; }
      else if (k === 'End') { window.scrollTo(0,document.body.scrollHeight); e.preventDefault(); return; }
      else { return; }
      e.preventDefault(); e.stopPropagation();
      applyZoom(DIC_ZOOM);
      try { dicApi('saveCfg', { zoom: DIC_ZOOM }); } catch (err) {}
      flash('🔍 缩放 ' + Math.round(DIC_ZOOM * 100) + '%');
    }
  }, true);
  document.addEventListener('DOMContentLoaded', function() {
    if (!window.dicApi) return;
    try {
      dicApi('onlineBarData').then(function(d) {
        if (!d || !d.sites || !d.sites.length) return;
        var b = document.getElementById('dic-online-bar');
        if (b) b.remove();
        b = document.createElement('div');
        b.id = 'dic-online-bar';
        b.style.cssText = 'position:fixed;top:0;left:0;right:0;height:36px;background:#eef3fa;border-bottom:1px solid #dbe5f0;display:flex;align-items:center;gap:6px;padding:0 8px;z-index:2147483000;font-size:13px;';
        function mk(label, fn) {
          var btn = document.createElement('button');
          btn.textContent = label;
          btn.style.cssText = 'padding:2px 8px;font-size:12px;cursor:pointer;';
          btn.onclick = fn;
          return btn;
        }
        d.sites.forEach(function(it, i) { b.appendChild(mk(it.n, function(){ dicApi('onlineOpen', i); })); });
        b.appendChild(mk('⌂', function(){ dicApi('onlineHome'); }));
        b.appendChild(mk('✕', function(){ dicApi('close'); }));
        document.body.appendChild(b);
        document.body.style.paddingTop = '44px';
      });
    } catch (err) {}
  });
})();
`;
