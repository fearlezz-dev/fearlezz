//! The proof page: one static page that shows everything a proof holds. Its verdict and
//! what waits for a person (a weaker check, pictures to approve, with the approve command in
//! the developer's own words) come first; then whether anything broke (every check, its
//! tests, findings, red before green and output); then whether it looks right: counts that
//! filter, pictures grouped by screen, viewport or theme, five ways to compare a before with
//! its after, films on one clock with lanes of where they differ, and a decision per picture.
//! A look alone gets the same page with only its pictures.

use std::fmt::Write as _;
use std::path::Path;

use anyhow::Result;

use crate::findings::Level;
use crate::look::{Change, Item, Look};
use crate::media::{self, Box4, Comparison, Pair};
use crate::page::esc;
use crate::proof::{Check, Proof, Ran, Verdict};

/// Width of the frames taken from films for the page.
const FRAME_W: usize = 720;

/// The kinds of change in the order they are shown.
const ORDER: [Change; 6] = [
    Change::Changed,
    Change::Timing,
    Change::New,
    Change::Gone,
    Change::Unreadable,
    Change::Same,
];

/// The ways to compare a still: key and label.
const MODES: [(&str, &str); 5] = [
    ("side", "Side by side"),
    ("slider", "Slider"),
    ("onion", "Onion skin"),
    ("diff", "Diff"),
    ("blink", "Blink"),
];

const STYLE: &str = r#"
main { max-width: 1180px; margin: 0 auto; padding: 44px 16px 120px; }
a { color: inherit; text-underline-offset: 3px; }
code { font: .92em ui-monospace, "SF Mono", Menlo, monospace; }
.q { color: var(--quiet); }

/* The top: what it is about, the verdict, and a meter of everything that counts. */
.top { margin: 0 0 56px; max-width: 72ch; }
.about-pr { margin: 0 0 14px; font-size: 18px; }
.about-pr .repo { color: var(--quiet); }
.about-pr a, .about-pr b { font-weight: 700; }
h1 { font-size: clamp(32px, 6vw, 48px); margin: 0; }
.line { margin: 12px 0 0; font-size: 19px; color: var(--quiet); }
.meter { display: flex; gap: 5px; margin: 22px 0 0; max-width: 560px; }
.meter span { flex: 1; height: 10px; border-radius: 5px; background: var(--ok); transform-origin: left;
  animation: grow .55s var(--ease) both; }
.meter span.bad { background: var(--changed); }
.meter span.wait { background: var(--timing); }
.meter span:nth-child(2) { animation-delay: 40ms; } .meter span:nth-child(3) { animation-delay: 80ms; }
.meter span:nth-child(4) { animation-delay: 120ms; } .meter span:nth-child(n+5) { animation-delay: 160ms; }
.commit { margin: 14px 0 0; font-size: 15px; color: var(--quiet); }

/* A part of the page answers one question. */
.part { margin: 0 0 64px; }
.part > h2 { font-size: 28px; margin: 0 0 18px; display: flex; align-items: center; gap: 12px; flex-wrap: wrap; }
.part > h2 .word { font-size: 16px; }
.sum { margin: -6px 0 18px; color: var(--quiet); }

/* What waits for a person. */
.rows { display: grid; gap: 16px; }
.row { background: var(--mat); border-radius: 18px; padding: 20px 22px; box-shadow: var(--shadow); }
.row .what p { margin: 6px 0 0; max-width: 72ch; }
.row .fix { color: var(--quiet); }
.approve { display: grid; gap: 14px 22px; grid-template-columns: auto 1fr auto; align-items: center;
  transition: box-shadow .3s; }
.peek { display: flex; gap: 8px; }
.peek img { width: 96px; height: 64px; object-fit: cover; border-radius: 8px; background: var(--sunk);
  box-shadow: 0 0 0 1px var(--line); }
.approve .said { grid-column: 1 / 3; }
.approve .cmd { grid-column: 1 / -1; }

/* Whether anything broke: one line per check, its details a click away. */
.checks { background: var(--mat); border-radius: 18px; box-shadow: var(--shadow); padding: 8px 0; }
.chk > summary:hover { background: color-mix(in srgb, var(--sunk) 45%, transparent); }
.chk > summary { display: grid; grid-template-columns: 14px auto 1fr auto; gap: 14px; align-items: center;
  padding: 14px 22px; color: var(--ink); }
.chk > summary::before { display: none; }
.chk > summary .meta { color: var(--quiet); }
.badges { display: flex; gap: 8px; flex-wrap: wrap; justify-content: flex-end; }
.dot { width: 12px; height: 12px; border-radius: 50%; background: var(--ok); }
.dot.bad { background: var(--changed); } .dot.off { background: var(--line); }
.chk .body { padding: 0 22px 20px 50px; display: grid; gap: 10px; animation: rise .3s var(--ease) both; }
.chk .body p { margin: 0; max-width: 76ch; }
.chk .why { font-weight: 700; }
.findings { list-style: none; margin: 0; padding: 0; display: grid; gap: 8px; }
.findings li { display: flex; gap: 10px; align-items: baseline; flex-wrap: wrap; }
.cases ul { margin: 8px 0 0; padding-left: 20px; columns: 2 320px; font-size: 15px; }
.cases li.failed { color: var(--changed); font-weight: 700; }
.chk pre { margin: 8px 0 0; background: var(--sunk); border-radius: 10px; padding: 12px 14px; overflow-x: auto;
  font: 13px/1.5 ui-monospace, "SF Mono", Menlo, monospace; }
.out p { margin: 8px 0 0; font-size: 15px; }

/* Whether it looks right: one block per look. */
.look { margin: 0 0 48px; }
.look-head { display: flex; flex-wrap: wrap; align-items: center; gap: 10px 14px; margin: 0 0 18px; }
.look-head h3 { font-size: 24px; margin: 0; }
.about { margin-left: auto; font-size: 15px; }
.about p { margin: 8px 0 0; max-width: 56ch; color: var(--quiet); }

.counts { display: flex; flex-wrap: wrap; gap: 10px; margin: 0 0 18px; }
.count { display: grid; grid-template-columns: auto auto; align-items: baseline; gap: 0 10px;
  min-width: 112px; padding: 10px 18px 12px; border-radius: 14px; background: var(--mat);
  border: 2px solid transparent; text-align: left; box-shadow: var(--shadow);
  transition: transform .2s var(--ease), border-color .2s, opacity .25s; }
.count:hover { transform: translateY(-2px); }
.count:active { transform: scale(.97); }
.count b { font-size: 32px; line-height: 1.1; }
.count span { color: var(--ink); }
.count[aria-pressed="true"] { border-color: currentColor; }
.counts.filtering .count[aria-pressed="false"] { opacity: .55; }

.bar { display: flex; flex-wrap: wrap; gap: 10px; align-items: center; }
.bar > span { color: var(--quiet); }

.group-head { display: flex; align-items: center; gap: 10px; margin: 40px 0 14px; font-size: 21px; }
.group-head .n { font-size: 15px; font-weight: 700; color: var(--quiet); background: var(--sunk);
  border-radius: 999px; padding: 0 10px; }

.card { background: var(--mat); border-radius: 18px; padding: 22px; margin: 0 0 22px; box-shadow: var(--shadow);
  outline: none; transition: box-shadow .3s var(--ease), opacity .3s; scroll-margin-top: 24px; }
.card:focus-visible { box-shadow: 0 0 0 3px var(--focus), var(--shadow); }
.card[data-decided="ok"] { box-shadow: 0 0 0 2.5px var(--ok), var(--shadow); }
.card[data-decided="no"] { box-shadow: 0 0 0 2.5px var(--changed), var(--shadow); }
.card.is-flaky { opacity: .6; }
.card.is-flaky:hover, .card.is-flaky:focus-within { opacity: 1; }
.head { display: flex; flex-wrap: wrap; gap: 8px 10px; align-items: center; margin: 0 0 16px; }
.head h2 { margin: 0 6px 0 0; font-size: 24px; }
.head .badge { margin-left: auto; width: 30px; height: 30px; border-radius: 50%; display: grid;
  place-items: center; color: var(--on-accent); transform: scale(0); transition: transform .35s var(--ease), background .2s; }
.card[data-decided="ok"] .badge { background: var(--ok); transform: scale(1); }
.card[data-decided="no"] .badge { background: var(--changed); transform: scale(1); }
.badge svg { width: 16px; height: 16px; }
.badge .x { display: none; }
.card[data-decided="no"] .badge .tick { display: none; }
.card[data-decided="no"] .badge .x { display: block; }

.modes { margin: 0 0 14px; }
.stage { position: relative; display: grid; gap: 16px; }
.stage.side { grid-template-columns: 1fr 1fr; }
.stage.one { max-width: 760px; }
.stage figure { margin: 0; min-width: 0; }
.stage figcaption { font-weight: 700; margin: 0 0 6px; transition: color .2s; }
.frame { position: relative; line-height: 0; background: var(--sunk); border-radius: 10px; overflow: hidden;
  isolation: isolate; box-shadow: 0 0 0 1px var(--line); }
.frame img, .frame video { width: 100%; height: auto; max-height: 640px; object-fit: contain; display: block; }
.frame.sized img, .frame.sized video { height: 100%; max-height: none; }
.frame .over { position: absolute; inset: 0; }
.frame .over img { width: 100%; height: 100%; max-height: none; }
.mark { position: absolute; outline: 3px solid var(--changed); outline-offset: 2px; border-radius: 3px;
  pointer-events: none; animation: mark-in .5s var(--ease) both; }
@keyframes mark-in { from { opacity: 0; transform: scale(1.25); } to { opacity: 1; transform: none; } }
.marks { position: absolute; inset: 0; pointer-events: none; }
.missing { padding: 40px 16px; text-align: center; color: var(--quiet); line-height: 1.5; }

/* Slider: drag across the picture. */
.slide { cursor: ew-resize; touch-action: pan-y; user-select: none; }
.slide .over { clip-path: inset(0 0 0 50%); }
.slide .handle { position: absolute; top: 0; bottom: 0; left: 50%; width: 0; }
.slide .handle::before { content: ""; position: absolute; top: 0; bottom: 0; left: -1.5px; width: 3px; background: var(--changed); }
.slide .grip { position: absolute; top: 50%; left: 0; width: 34px; height: 34px; margin: -17px 0 0 -17px;
  border-radius: 50%; background: var(--mat); border: 3px solid var(--changed); box-shadow: var(--shadow);
  transition: transform .2s var(--ease); }
.slide:hover .grip, .slide .handle:focus-visible .grip { transform: scale(1.12); }
.slide .handle:focus-visible { outline: none; }
.slide.glide .over { transition: clip-path .35s var(--ease); }
.slide.glide .handle { transition: left .35s var(--ease); }
.ends { display: flex; justify-content: space-between; font-weight: 700; margin: 0 0 6px; }

/* Onion skin: the after over the before. */
input[type=range] { width: 100%; accent-color: var(--changed); margin: 12px 0 0; }

/* Diff: the difference of the two pictures, the changed pixels in the change colour. */
.diffview { --inv: 1; --tint: screen; }
@media (prefers-color-scheme: dark) { .diffview { --inv: 0; --tint: multiply; } }
.diffview .blend { position: relative; filter: grayscale(1) brightness(8) contrast(2) invert(var(--inv)); }
.diffview .blend .over { mix-blend-mode: difference; }
.diffview .tint { position: absolute; inset: 0; background: var(--changed); mix-blend-mode: var(--tint); }

/* Blink: the before and after in turn, in place. */
.blink .over { transition: opacity .07s linear; }
.blink.show-before .over { opacity: 0; }

/* Film: both films on one clock, with lanes of where they differ. */
.timeline { position: relative; height: 66px; margin: 18px 0 8px; user-select: none; touch-action: none; cursor: pointer; border-radius: 8px; }
.lane { position: absolute; left: 0; right: 0; height: 24px; border-radius: 7px; background: var(--sunk); overflow: hidden; }
.lane.before { top: 4px; } .lane.after { top: 36px; }
.lane i { position: absolute; top: 3px; bottom: 3px; border-radius: 4px; background: var(--line);
  transform-origin: left; animation: grow .6s var(--ease) both; }
.lane i.diff { background: var(--changed); }
@keyframes grow { from { transform: scaleX(0); } to { transform: none; } }
.lane em { position: absolute; left: 8px; top: 0; font-style: normal; font-size: 13px; font-weight: 700; line-height: 24px; }
.playhead { position: absolute; top: 0; bottom: 0; left: 0; width: 3px; margin-left: -1.5px; background: var(--ink); border-radius: 2px; }
.clock { display: flex; gap: 10px; align-items: center; flex-wrap: wrap; }
.clock .t { min-width: 7ch; font-weight: 700; }
.chip { border: 1.5px solid var(--line); background: var(--mat); border-radius: 10px; padding: 6px 14px;
  transition: border-color .2s, transform .15s var(--ease), background .2s; }
.chip:hover { border-color: var(--quiet); }
.chip:active { transform: scale(.96); }
.chip.jump { border-color: var(--changed); color: var(--changed); font-weight: 700; }
.frames { margin: 18px 0 0; }
.frames h3 { font-size: 17px; margin: 18px 0 10px; }

.more { margin: 16px 0 0; font-size: 15px; }
.more p { margin: 8px 0 0; color: var(--quiet); }

.decide { display: flex; flex-wrap: wrap; gap: 10px; align-items: center; margin: 18px 0 0; }
.decide button { border: 2px solid var(--line); background: none; border-radius: 12px; padding: 8px 18px;
  font-weight: 700; transition: border-color .2s, background .2s, color .2s, transform .15s var(--ease); }
.decide button:hover { border-color: var(--quiet); }
.decide button:active { transform: scale(.95); }
.decide .yes[aria-pressed="true"] { background: var(--ok); border-color: var(--ok); color: var(--on-accent); }
.decide .no[aria-pressed="true"] { background: var(--changed); border-color: var(--changed); color: var(--on-accent); }
.decide .flake { margin-left: auto; border-style: dashed; color: var(--quiet); font-weight: 400; }
.decide .flake[aria-pressed="true"] { color: var(--ink); border-style: solid; }
.note { flex-basis: 100%; display: grid; grid-template-rows: 0fr; transition: grid-template-rows .3s var(--ease); }
.note > div { visibility: hidden; transition: visibility .3s; }
.note.open { grid-template-rows: 1fr; }
.note.open > div { visibility: visible; }
.note > div { overflow: hidden; }
.note textarea { width: 100%; max-width: 680px; margin-top: 4px; font: inherit; padding: 10px 12px; border-radius: 12px;
  border: 2px solid var(--line); background: var(--surface); color: var(--ink); resize: vertical; }
.note textarea:focus { border-color: var(--changed); outline: none; }

.keys { margin: 40px 0 0; font-size: 15px; }
.keys p { margin: 8px 0 0; }
kbd { font: 14px ui-monospace, Menlo, monospace; border: 1px solid var(--line); border-radius: 6px;
  padding: 0 6px; background: var(--mat); box-shadow: 0 1px 0 var(--line); }

/* Approve: progress, the developer's words, and the command or feedback to copy. */
.progress { display: flex; align-items: center; gap: 10px; font-weight: 700; white-space: nowrap; }
.ring { width: 38px; height: 38px; transform: rotate(-90deg); }
.ring circle { fill: none; stroke-width: 4; }
.ring .track { stroke: var(--sunk); }
.ring .fill { stroke: var(--ok); stroke-linecap: round; stroke-dasharray: 100; stroke-dashoffset: 100;
  transition: stroke-dashoffset .5s var(--ease), stroke .3s; }
.approve.blocked .ring .fill { stroke: var(--changed); }
.said { display: flex; gap: 10px; align-items: center; min-width: 0; }
.said span { white-space: nowrap; }
.said input { flex: 1; min-width: 0; font: inherit; padding: 8px 12px; border-radius: 12px;
  border: 2px solid var(--line); background: var(--surface); color: var(--ink); transition: border-color .2s; }
.said input:focus { border-color: var(--ok); outline: none; }
.approve.blocked .said { opacity: .45; }
.go { display: flex; gap: 8px; }
.go button { position: relative; border: 0; border-radius: 12px; padding: 10px 18px; font-weight: 700;
  display: inline-flex; align-items: center; gap: 8px; white-space: nowrap;
  transition: background .25s, transform .15s var(--ease), box-shadow .3s; }
.go button:active { transform: scale(.96); }
.go .primary { background: var(--ok); color: var(--on-accent); }
.approve.blocked .go .primary { background: var(--changed); }
.go .second { background: var(--sunk); }
.go svg { width: 18px; height: 18px; display: none; }
.go svg path { fill: none; stroke: currentColor; stroke-width: 3; stroke-linecap: round; stroke-linejoin: round;
  stroke-dasharray: 24; stroke-dashoffset: 24; }
.go .done svg { display: block; }
.go .done svg path { animation: draw .45s var(--ease) forwards; }
.go .done { animation: pop .5s var(--ease); }
@keyframes draw { to { stroke-dashoffset: 0; } }
@keyframes pop { 0% { box-shadow: 0 0 0 0 var(--ring); } 35% { transform: scale(1.06); }
  100% { transform: none; box-shadow: 0 0 0 16px transparent; } }
.go .primary { --ring: color-mix(in srgb, var(--ok) 55%, transparent); }
.approve.blocked .go .primary { --ring: color-mix(in srgb, var(--changed) 55%, transparent); }
.go .second { --ring: color-mix(in srgb, var(--ink) 30%, transparent); }
.cmd { font: 14px/1.4 ui-monospace, "SF Mono", Menlo, monospace; color: var(--quiet);
  overflow-x: auto; white-space: nowrap; margin: 0; }

@media (max-width: 760px) {
  main { padding-top: 24px; }
  .stage.side { grid-template-columns: 1fr; }
  .card { padding: 16px; border-radius: 16px; }
  .approve { grid-template-columns: 1fr; }
  .approve .said { grid-column: auto; }
  .said span { display: none; }
  .go .second { display: none; }
  .cmd { display: none; }
  .chk > summary { grid-template-columns: 14px 1fr auto; padding: 12px 16px; }
  .chk > summary .badges { grid-column: 2 / -1; justify-content: flex-start; }
  .chk .body { padding: 0 16px 16px 44px; }
  .row { padding: 16px; }
  .decide .flake { margin-left: 0; }
  .about { margin-left: 0; flex-basis: 100%; }
  .count { min-width: 0; padding: 8px 14px 10px; }
  .count b { font-size: 26px; }
}
@media (prefers-reduced-motion: reduce) { .lane i, .mark, .meter span { animation: none; } }
"#;

/// Everything on the page that moves or decides.
const SCRIPT: &str = r#"
(function () {
var WORD = { changed: 'Changed', timing: 'Timing only', new: 'New', gone: 'Gone', unreadable: 'Could not read', same: 'Unchanged' };

function el(tag, cls, kids) {
  var e = document.createElement(tag);
  if (cls) e.className = cls;
  (kids || []).forEach(function (k) { if (k) e.appendChild(typeof k === 'string' ? document.createTextNode(k) : k); });
  return e;
}
function anim(node, frames, opts) {
  if (calm.matches || !node.animate) return Promise.resolve();
  var a = node.animate(frames, Object.assign({ duration: 220, easing: 'cubic-bezier(.2,.8,.2,1)' }, opts));
  return a.finished.catch(function () {});
}
function quote(s) { return '"' + s.replace(/(["\\$`])/g, '\\$1') + '"'; }
function copyText(text) {
  if (navigator.clipboard && window.isSecureContext) return navigator.clipboard.writeText(text).catch(function () { return copyByHand(text); });
  return copyByHand(text);
}
function copyByHand(text) {
  var t = el('textarea'); t.value = text; t.style.position = 'fixed'; t.style.opacity = '0';
  document.body.appendChild(t); t.select();
  try { document.execCommand('copy'); } catch (e) {}
  t.remove();
  return Promise.resolve();
}

// ----- compare a still ------------------------------------------------------------------
function picture(src) { var i = new Image(); i.src = src; i.alt = ''; i.draggable = false; return i; }
function marks(frame, cmp) {
  sized(frame, cmp);
  var w = +cmp.dataset.w, h = +cmp.dataset.h, boxes = JSON.parse(cmp.dataset.marks || '[]');
  if (!w || !h) return;
  boxes.forEach(function (b) {
    var m = el('div', 'mark');
    m.style.cssText = 'left:' + b[0] / w * 100 + '%;top:' + b[1] / h * 100 + '%;width:' + (b[2] - b[0]) / w * 100 + '%;height:' + (b[3] - b[1]) / h * 100 + '%';
    frame.appendChild(m);
  });
}
function fig(label, frame) { return el('figure', '', [el('figcaption', '', [label]), frame]); }
// A frame as large as its picture shows, so outlines sit on the picture and not on its mat.
function sized(frame, cmp) {
  var w = +cmp.dataset.w, h = +cmp.dataset.h;
  if (!w || !h) return frame;
  frame.classList.add('sized');
  frame.style.aspectRatio = w + ' / ' + h;
  frame.style.width = 'min(100%, ' + (640 * w / h) + 'px)';
  return frame;
}
function over(src) { return el('div', 'over', [picture(src)]); }

var BUILD = {
  side: function (cmp) {
    var b = el('div', 'frame', [picture(cmp.dataset.before)]), a = el('div', 'frame', [picture(cmp.dataset.after)]);
    marks(b, cmp); marks(a, cmp);
    return { cls: 'side', kids: [fig('Before', b), fig('After', a)] };
  },
  slider: function (cmp) {
    var top = over(cmp.dataset.after);
    var grip = el('span', 'grip');
    var handle = el('div', 'handle', [grip]);
    handle.tabIndex = 0;
    handle.setAttribute('role', 'slider');
    handle.setAttribute('aria-label', 'Where the before ends');
    handle.setAttribute('aria-valuemin', '0'); handle.setAttribute('aria-valuemax', '100');
    var frame = el('div', 'frame slide', [picture(cmp.dataset.before), top, handle]);
    marks(frame, cmp);
    var at = 50;
    function put(v, glide) {
      at = Math.max(0, Math.min(100, v));
      frame.classList.toggle('glide', !!glide);
      top.style.clipPath = 'inset(0 0 0 ' + at + '%)';
      handle.style.left = at + '%';
      handle.setAttribute('aria-valuenow', Math.round(at));
    }
    function fromPointer(e) { var r = frame.getBoundingClientRect(); return (e.clientX - r.left) / r.width * 100; }
    var dragging = false;
    frame.addEventListener('pointerdown', function (e) { dragging = true; frame.setPointerCapture(e.pointerId); put(fromPointer(e), true); });
    frame.addEventListener('pointermove', function (e) { if (dragging) put(fromPointer(e), false); });
    frame.addEventListener('pointerup', function () { dragging = false; });
    frame.addEventListener('pointercancel', function () { dragging = false; });
    handle.addEventListener('keydown', function (e) {
      var step = e.shiftKey ? 10 : 2;
      if (e.key === 'ArrowLeft') { put(at - step, true); e.preventDefault(); }
      if (e.key === 'ArrowRight') { put(at + step, true); e.preventDefault(); }
    });
    put(50);
    var ends = el('div', 'ends', [el('span', '', ['Before']), el('span', '', ['After'])]);
    return { cls: 'one', kids: [el('figure', '', [ends, frame])] };
  },
  onion: function (cmp) {
    var top = over(cmp.dataset.after);
    top.style.opacity = .5;
    var frame = el('div', 'frame', [picture(cmp.dataset.before), top]);
    marks(frame, cmp);
    var range = el('input');
    range.type = 'range'; range.min = 0; range.max = 100; range.value = 50;
    range.setAttribute('aria-label', 'How much of the after shows');
    range.addEventListener('input', function () { top.style.opacity = range.value / 100; });
    return { cls: 'one', kids: [fig('Before under after', frame), range] };
  },
  diff: function (cmp) {
    var blend = el('div', 'blend', [picture(cmp.dataset.before), over(cmp.dataset.after)]);
    var frame = el('div', 'frame diffview', [blend, el('div', 'tint')]);
    marks(frame, cmp);
    return { cls: 'one', kids: [fig('What differs', frame)] };
  },
  blink: function (cmp) {
    var cap = el('figcaption', '', ['After']);
    var frame = el('div', 'frame blink', [picture(cmp.dataset.before), over(cmp.dataset.after)]);
    marks(frame, cmp);
    var paused = false;
    frame.addEventListener('pointerenter', function () { paused = true; });
    frame.addEventListener('pointerleave', function () { paused = false; });
    var timer = setInterval(function () {
      if (!frame.isConnected) { clearInterval(timer); return; }
      if (paused) return;
      var before = frame.classList.toggle('show-before');
      cap.textContent = before ? 'Before' : 'After';
    }, 650);
    return { cls: 'one', kids: [el('figure', '', [cap, frame])] };
  },
};
function setMode(cmp, mode) {
  var seg = cmp.querySelector('.modes');
  seg.querySelectorAll('button').forEach(function (b) { b.setAttribute('aria-pressed', String(b.dataset.mode === mode)); });
  placePill(seg);
  var stage = cmp.querySelector('.stage');
  var from = stage.offsetHeight;
  anim(stage, [{ opacity: 1, transform: 'none' }, { opacity: 0, transform: 'scale(.985)' }], { duration: 110 }).then(function () {
    var built = BUILD[mode](cmp);
    stage.className = 'stage ' + built.cls;
    stage.textContent = '';
    built.kids.forEach(function (k) { stage.appendChild(k); });
    var to = stage.offsetHeight;
    anim(stage, [{ opacity: 0, transform: 'scale(.985)', height: from + 'px' }, { opacity: 1, transform: 'none', height: to + 'px' }], { duration: 280 });
  });
  cmp.dataset.mode = mode;
}
document.querySelectorAll('.cmp').forEach(function (cmp) {
  cmp.querySelectorAll('[data-mode]').forEach(function (b) {
    b.addEventListener('click', function () { if (cmp.dataset.mode !== b.dataset.mode) setMode(cmp, b.dataset.mode); });
  });
});
document.querySelectorAll('details').forEach(function (d) {
  d.addEventListener('toggle', function () { if (d.open) d.querySelectorAll('.seg').forEach(placePill); });
});

// ----- films ------------------------------------------------------------------------
document.querySelectorAll('.film').forEach(function (film) {
  var vids = film.querySelectorAll('video'), vb = vids[0], va = vids[1];
  vb.muted = va.muted = true;
  var total = +film.dataset.total || 1, tb = +film.dataset.tb, ta = +film.dataset.ta;
  var steps = JSON.parse(film.dataset.steps || '[]');
  var w = +film.dataset.w, h = +film.dataset.h;
  var tl = film.querySelector('.timeline'), head = film.querySelector('.playhead');
  var t = film.querySelector('.t'), play = film.querySelector('.play');
  var layers = film.querySelectorAll('.marks');
  var shownStep = -1;
  function showMarks(s) {
    var k = -1;
    steps.forEach(function (st, i) { if (st[0] <= s + 0.001) k = i; });
    if (k === shownStep) return;
    shownStep = k;
    layers.forEach(function (layer) {
      layer.textContent = '';
      if (k < 0 || !w || !h) return;
      steps[k][1].forEach(function (b) {
        var m = el('div', 'mark');
        m.style.cssText = 'left:' + b[0] / w * 100 + '%;top:' + b[1] / h * 100 + '%;width:' + (b[2] - b[0]) / w * 100 + '%;height:' + (b[3] - b[1]) / h * 100 + '%';
        layer.appendChild(m);
      });
    });
  }
  function show(s) {
    head.style.left = s / total * 100 + '%';
    t.textContent = s.toFixed(2) + ' s';
    tl.setAttribute('aria-valuenow', s.toFixed(2));
    showMarks(s);
  }
  function seek(s) {
    s = Math.max(0, Math.min(total, s));
    vb.currentTime = Math.min(s, Math.max(0, tb - 0.01));
    va.currentTime = Math.min(s, Math.max(0, ta - 0.01));
    show(s);
  }
  function at(e) { var r = tl.getBoundingClientRect(); seek((e.clientX - r.left) / r.width * total); }
  tl.addEventListener('pointerdown', function (e) { stop(); tl.setPointerCapture(e.pointerId); at(e); });
  tl.addEventListener('pointermove', function (e) { if (e.buttons) at(e); });
  tl.addEventListener('keydown', function (e) {
    if (e.key === 'ArrowRight') { stop(); seek(va.currentTime + 0.1); e.preventDefault(); }
    if (e.key === 'ArrowLeft') { stop(); seek(va.currentTime - 0.1); e.preventDefault(); }
  });
  var raf = 0;
  function tick() { show(va.currentTime); if (!va.paused) raf = requestAnimationFrame(tick); }
  function stop() { vb.pause(); va.pause(); play.textContent = 'Play both'; cancelAnimationFrame(raf); }
  play.addEventListener('click', function () {
    if (va.paused) {
      if (va.currentTime >= ta - 0.05) seek(0);
      vb.play(); va.play(); play.textContent = 'Pause'; tick();
    } else stop();
  });
  va.addEventListener('ended', stop);
  va.addEventListener('timeupdate', function () { show(va.currentTime); });
  film.querySelectorAll('[data-seek]').forEach(function (b) {
    b.addEventListener('click', function () { stop(); seek(+b.dataset.seek); });
  });
  va.addEventListener('loadeddata', function () { seek(0); });
});

// ----- one look: its filter, groups, decisions and approval ------------------------------
document.querySelectorAll('.look').forEach(function (section) {
var CHECK = section.dataset.check;
var KEY = 'fearlezz:' + location.pathname + ':' + CHECK;
var saved = {};
try { saved = JSON.parse(localStorage.getItem(KEY)) || {}; } catch (e) {}
['d', 'notes', 'flaky'].forEach(function (k) { if (!saved[k]) saved[k] = {}; });
function save() { try { localStorage.setItem(KEY, JSON.stringify(saved)); } catch (e) {} }
var cards = Array.prototype.slice.call(section.querySelectorAll('.card'));
var list = section.querySelector('.list');
var state = { filter: null, group: 'change' };
function flaky(card) { return card.dataset.flaky === 'config' || !!saved.flaky[card.dataset.name]; }
function needsLook(card) { return card.dataset.change !== 'same' && !flaky(card); }

// ----- filter and group ---------------------------------------------------------------
var allSame = cards.every(function (c) { return c.dataset.change === 'same'; });
function shown(card) {
  if (state.filter) return card.dataset.change === state.filter;
  return allSame || card.dataset.change !== 'same';
}
function groupOf(card) {
  if (state.group === 'change') return WORD[card.dataset.change];
  return card.dataset[state.group] || 'Other';
}
function layout() {
  list.querySelectorAll('.group-head').forEach(function (h) { h.remove(); });
  var groups = [], by = {};
  cards.forEach(function (c) {
    var on = shown(c);
    c.hidden = !on;
    if (!on) return;
    var g = groupOf(c);
    if (!by[g]) { by[g] = []; groups.push(g); }
    by[g].push(c);
  });
  groups.forEach(function (g) {
    var n = el('span', 'n num');
    var h = el('h2', 'group-head', [g, n]);
    list.appendChild(h);
    countTo(n, by[g].length);
    by[g].forEach(function (c) { list.appendChild(c); });
  });
}
function relayout() {
  anim(list, [{ opacity: 1 }, { opacity: 0, transform: 'translateY(4px)' }], { duration: 120 }).then(function () {
    layout();
    anim(list, [{ opacity: 0, transform: 'translateY(6px)' }, { opacity: 1, transform: 'none' }], { duration: 260 });
  });
}
var counts = section.querySelector('.counts');
section.querySelectorAll('[data-filter]').forEach(function (b) {
  b.addEventListener('click', function () {
    state.filter = state.filter === b.dataset.filter ? null : b.dataset.filter;
    section.querySelectorAll('[data-filter]').forEach(function (o) { o.setAttribute('aria-pressed', String(o.dataset.filter === state.filter)); });
    counts.classList.toggle('filtering', !!state.filter);
    relayout();
  });
});
section.querySelectorAll('[data-group]').forEach(function (b) {
  b.addEventListener('click', function () {
    state.group = b.dataset.group;
    section.querySelectorAll('[data-group]').forEach(function (o) { o.setAttribute('aria-pressed', String(o === b)); });
    placePill(b.parentNode);
    relayout();
  });
});

// ----- decide -------------------------------------------------------------------------
function paint(card) {
  var name = card.dataset.name, d = saved.d[name] || '';
  card.dataset.decided = d;
  card.classList.toggle('is-flaky', flaky(card));
  card.querySelectorAll('.decide button').forEach(function (b) {
    var on = b.dataset.decide === 'flaky' ? flaky(card) : b.dataset.decide === d;
    b.setAttribute('aria-pressed', String(on));
  });
  var note = card.querySelector('.note');
  if (note) note.classList.toggle('open', d === 'no');
}
function decide(card, v) {
  var name = card.dataset.name;
  if (v === 'flaky') {
    if (card.dataset.flaky === 'config') return;
    saved.flaky[name] = !saved.flaky[name];
  } else saved.d[name] = saved.d[name] === v ? '' : v;
  save(); paint(card); dock();
  if (v === 'no' && saved.d[name] === 'no') setTimeout(function () { card.querySelector('textarea').focus({ preventScroll: true }); }, 200);
  if (v === 'ok' && saved.d[name] === 'ok') setTimeout(function () { step(1, card); }, 380);
}
cards.forEach(function (card) {
  card.decide = function (v) { decide(card, v); };
  card.querySelectorAll('.decide button').forEach(function (b) {
    b.addEventListener('click', function () { decide(card, b.dataset.decide); });
  });
  var ta = card.querySelector('textarea');
  if (ta) {
    ta.value = saved.notes[card.dataset.name] || '';
    ta.addEventListener('input', function () { saved.notes[card.dataset.name] = ta.value; save(); dock(); });
  }
  paint(card);
});

// ----- approve ------------------------------------------------------------------------
var row = document.querySelector('.approve[data-for="' + CHECK.replace(/["\\]/g, '\\$&') + '"]');
function dock() {
  if (!row) return;
  var said = row.querySelector('.said input');
  var need = cards.filter(needsLook);
  var seen = need.filter(function (c) { return saved.d[c.dataset.name]; });
  var wrong = need.filter(function (c) { return saved.d[c.dataset.name] === 'no'; });
  var marked = cards.filter(function (c) { return c.dataset.flaky !== 'config' && saved.flaky[c.dataset.name]; });
  countTo(row.querySelector('.seen'), seen.length);
  row.querySelector('.need').textContent = need.length;
  var fill = row.querySelector('.ring .fill');
  fill.style.strokeDashoffset = need.length ? 100 - seen.length / need.length * 100 : 0;
  row.classList.toggle('blocked', wrong.length > 0);
  var copy = row.querySelector('.copy'), copy2 = row.querySelector('.copy2'), cmd = row.querySelector('.cmd');
  var flakyLine = marked.length ? 'Mark as flaky in fearlezz.toml: ' + marked.map(function (c) { return c.dataset.name; }).join(', ') : '';
  if (wrong.length) {
    var text = 'These pictures of ' + CHECK + ' are not right:\n' + wrong.map(function (c) {
      var note = (saved.notes[c.dataset.name] || '').trim();
      return '- ' + c.dataset.name + (note ? ': ' + note : '');
    }).join('\n') + (flakyLine ? '\n' + flakyLine : '');
    copy.dataset.copy = text; copy.dataset.label = 'Copy feedback for the agent';
    copy2.hidden = true;
    cmd.textContent = text.replace(/\n/g, '  ');
  } else {
    var words = said.value.trim() || 'looks good';
    var c = 'fearlezz say approve ' + CHECK + ' --said ' + quote(words);
    copy.dataset.copy = c; copy.dataset.label = 'Copy approve command';
    copy2.hidden = false; copy2.dataset.label = 'Copy as message';
    copy2.dataset.copy = 'The ' + CHECK + ' look is right. In my words: ' + quote(words) + (flakyLine ? '\n' + flakyLine : '');
    cmd.textContent = c;
  }
  [copy, copy2].forEach(function (b) { if (!b.classList.contains('done')) b.querySelector('span').textContent = b.dataset.label; });
}
if (row) {
  [row.querySelector('.copy'), row.querySelector('.copy2')].forEach(function (b) {
    b.addEventListener('click', function () {
      copyText(b.dataset.copy).then(function () {
        b.classList.remove('done'); void b.offsetWidth; b.classList.add('done');
        b.querySelector('span').textContent = 'Copied';
        clearTimeout(b.timer);
        b.timer = setTimeout(function () { b.classList.remove('done'); dock(); }, 1800);
      });
    });
  });
  row.querySelector('.said input').addEventListener('input', dock);
}

dock();
layout();
});

// ----- keys ---------------------------------------------------------------------------
var current = null;
var cards = Array.prototype.slice.call(document.querySelectorAll('.card'));
function step(d, from) {
  var open = cards.filter(function (c) { return !c.hidden; });
  var i = open.indexOf(from || current);
  var next = open[Math.max(0, Math.min(open.length - 1, i + d))];
  if (!next) return;
  current = next;
  next.focus({ preventScroll: true });
  next.scrollIntoView({ behavior: calm.matches ? 'auto' : 'smooth', block: 'start' });
}
document.addEventListener('focusin', function (e) { var c = e.target.closest && e.target.closest('.card'); if (c) current = c; });
document.addEventListener('keydown', function (e) {
  if (/INPUT|TEXTAREA/.test(e.target.tagName) || e.metaKey || e.ctrlKey || e.altKey) return;
  if (e.key === 'j') step(1);
  else if (e.key === 'k') step(-1);
  else if (current && current.dataset.change !== 'same' && (e.key === 'y' || e.key === 'n' || e.key === 'f')) current.decide({ y: 'ok', n: 'no', f: 'flaky' }[e.key]);
  else if (current && /^[1-5]$/.test(e.key)) {
    var cmp = current.querySelector('.cmp');
    var mode = ['side', 'slider', 'onion', 'diff', 'blink'][+e.key - 1];
    if (cmp && cmp.dataset.mode !== mode) setMode(cmp, mode);
  }
});

riseIn(document.querySelectorAll('.top, .part > h2, .row, .card:not([hidden])'));
})();
"#;

/// Writes the page of one look into its folder as `index.html`: the same page as a proof's,
/// with only its pictures to show.
pub(crate) fn write(dir: &Path, look: &mut Look) -> Result<()> {
    let proof = Proof::of_look(look.clone());
    let html = page(dir, &proof)?;
    std::fs::write(dir.join("index.html"), html)?;
    look.report = dir.join("index.html");
    Ok(())
}

/// Writes the proof page into the bundle `dir` as `index.html`. `proof` names its files
/// relative to `dir`, as `Proof::write` leaves them.
pub(crate) fn write_proof(dir: &Path, proof: &Proof) -> Result<()> {
    let html = page(dir, proof)?;
    std::fs::write(dir.join("index.html"), html)?;
    Ok(())
}

/// The whole page: what it is about and where it stands, what waits for a person, whether
/// anything broke, and whether it looks right. Each part shows only when the proof has
/// something for it; the important things are open, the rest a click away.
fn page(dir: &Path, proof: &Proof) -> Result<String> {
    let mut html = String::new();
    let title = match (&proof.pull, proof.looks.as_slice(), proof.checks.is_empty()) {
        (Some(pull), _, _) => format!("Proof: {}", pull.title),
        (None, [look], true) => format!("Look: {}", look.check),
        _ => "Proof".to_string(),
    };
    write!(
        html,
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{}</title><style>{}{STYLE}</style></head><body><main>",
        esc(&title),
        crate::page::STYLE
    )?;
    top(&mut html, proof)?;
    waits(&mut html, proof)?;
    breaks(&mut html, dir, proof)?;
    looks(&mut html, dir, proof)?;
    write!(
        html,
        "</main><script>{}{SCRIPT}</script></body></html>",
        crate::page::SCRIPT
    )?;
    Ok(html)
}

/// What the proof is about, its verdict in one line, and a meter of everything that counts.
fn top(html: &mut String, proof: &Proof) -> std::fmt::Result {
    html.push_str("<header class=\"top\">");
    if let Some(pull) = &proof.pull {
        let name = format!("#{} {}", pull.number, pull.title);
        html.push_str("<p class=\"about-pr\">");
        if let Some(repo) = &proof.repository {
            write!(html, "<span class=\"repo\">{}</span> ", esc(repo))?;
        }
        match &pull.url {
            Some(url) => write!(html, "<a href=\"{}\">{}</a>", esc(url), esc(&name))?,
            None => write!(html, "<b>{}</b>", esc(&name))?,
        }
        html.push_str("</p>");
    }
    let (verdict, line) = verdict(proof);
    write!(html, "<h1>{}</h1>", esc(&verdict))?;
    if !line.is_empty() {
        write!(html, "<p class=\"line\">{}</p>", esc(&line))?;
    }
    let ran: Vec<&Check> = proof.ran_here().collect();
    let waiting = proof.weakened.len() + proof.waits().len();
    if !ran.is_empty() {
        html.push_str("<div class=\"meter\" aria-hidden=\"true\">");
        for c in &ran {
            let class = if c.outcome == Ran::Failed || c.unstable {
                "bad"
            } else {
                "ok"
            };
            write!(html, "<span class=\"{class}\"></span>")?;
        }
        for _ in 0..waiting {
            html.push_str("<span class=\"wait\"></span>");
        }
        html.push_str("</div>");
    }
    if !proof.commit.is_empty() {
        write!(
            html,
            "<p class=\"commit\">Commit {}</p>",
            esc(&proof.commit)
        )?;
    }
    html.push_str("</header>");
    Ok(())
}

/// The verdict and one sentence on why.
fn verdict(proof: &Proof) -> (String, String) {
    let waits = proof.waits();
    if proof.checks.is_empty() {
        let need: usize = proof.looks.iter().map(Look::to_look_at).sum();
        let base = proof
            .looks
            .first()
            .map(|l| match &l.no_before {
                Some(why) => format!("There is no before: {why}. Every picture counts as new."),
                None => format!(
                    "Compared with the same check on the base commit {}.",
                    l.base_commit.get(..10).unwrap_or(&l.base_commit)
                ),
            })
            .unwrap_or_default();
        let title = match need {
            0 => "Nothing to look at".to_string(),
            1 => "1 picture to look at".to_string(),
            n => format!("{n} pictures to look at"),
        };
        return (title, base);
    }
    let ran = proof.ran_here().count();
    let failed = proof
        .ran_here()
        .filter(|c| c.outcome == Ran::Failed)
        .count();
    let fixes = proof.blockers().len() - proof.weakened.len();
    let person = proof.weakened.len() + waits.len();
    let things = |n: usize| {
        if n == 1 {
            "1 thing".to_string()
        } else {
            format!("{n} things")
        }
    };
    if fixes > 0 {
        let line = if failed > 0 {
            format!("{failed} of {ran} checks failed.")
        } else {
            "Every check passed, but the proof does not hold yet.".to_string()
        };
        return (format!("Not ready: {} to fix", things(fixes)), line);
    }
    if person > 0 {
        let mut what = Vec::new();
        if !waits.is_empty() {
            what.push("approve changed pictures");
        }
        if !proof.weakened.is_empty() {
            what.push("accept a weaker check");
        }
        return (
            format!("Not ready: {} wait for you", things(person)),
            format!(
                "Every check passed. A person with write access has to {}.",
                what.join(" and ")
            ),
        );
    }
    (
        "Ready to merge".to_string(),
        format!("All {ran} checks passed and no picture waits for a person."),
    )
}

/// What only a person can settle: a weaker check, and pictures to approve.
fn waits(html: &mut String, proof: &Proof) -> std::fmt::Result {
    let waits = proof.waits();
    if proof.weakened.is_empty() && waits.is_empty() {
        return Ok(());
    }
    html.push_str(
        "<section class=\"part\" id=\"waits\"><h2>Waits for you</h2><div class=\"rows\">",
    );
    for item in &proof.weakened {
        write!(
            html,
            "<div class=\"row weak\"><div class=\"what\"><b>{}</b> <span class=\"word timing\">Weaker</span><p>{}</p><p class=\"fix\">{}</p></div></div>",
            prose(&capital(&item.what)),
            prose(&capital(&item.why)),
            prose(&capital(&item.fix))
        )?;
    }
    for look in &waits {
        approve(html, look)?;
    }
    html.push_str("</div></section>");
    Ok(())
}

/// One look's pictures to approve: a peek at the first, how many were looked at, and the
/// approve command in the developer's own words.
fn approve(html: &mut String, look: &Look) -> std::fmt::Result {
    let need = look.to_look_at();
    let check = esc(&look.check);
    write!(
        html,
        "<div class=\"row approve\" data-for=\"{check}\"><div class=\"peek\">"
    )?;
    if let Some(item) = look
        .items
        .iter()
        .find(|i| i.change != Change::Same && !i.flaky)
    {
        for src in [&item.before, &item.after].into_iter().flatten() {
            let lower = src.to_ascii_lowercase();
            if [".mp4", ".webm", ".mov", ".m4v"]
                .iter()
                .any(|e| lower.ends_with(e))
            {
                html.push_str("<span class=\"tag\">Film</span>");
                break;
            }
            write!(html, "<img src=\"{}\" alt=\"\">", esc(src))?;
        }
    }
    let command = format!("fearlezz say approve {} --said \"looks good\"", look.check);
    write!(
        html,
        "</div><div class=\"what\"><b>Pictures of <code>{check}</code></b> <span class=\"q\">{}</span><p><a href=\"#look-{check}\">Look at them</a></p></div><div class=\"progress\"><svg class=\"ring\" viewBox=\"0 0 38 38\" aria-hidden=\"true\"><circle class=\"track\" cx=\"19\" cy=\"19\" r=\"15.9\" pathLength=\"100\"/><circle class=\"fill\" cx=\"19\" cy=\"19\" r=\"15.9\" pathLength=\"100\"/></svg><span><span class=\"num seen\">0</span> of <span class=\"num need\">{need}</span> looked at</span></div><label class=\"said\"><span>In your words</span><input value=\"looks good\" autocomplete=\"off\"></label><div class=\"go\"><button type=\"button\" class=\"primary copy\" data-copy=\"{cmd}\"><svg viewBox=\"0 0 24 24\" aria-hidden=\"true\"><path d=\"M5 12.5l4.5 4.5L19 7.5\"/></svg><span>Copy approve command</span></button><button type=\"button\" class=\"second copy2\"><svg viewBox=\"0 0 24 24\" aria-hidden=\"true\"><path d=\"M5 12.5l4.5 4.5L19 7.5\"/></svg><span>Copy as message</span></button></div><p class=\"cmd\">{cmd}</p></div>",
        esc(&look.counts()),
        cmd = esc(&command)
    )
}

/// Whether anything broke: every check, the failed ones open, the rest a click away.
fn breaks(html: &mut String, dir: &Path, proof: &Proof) -> Result<()> {
    if proof.checks.is_empty() {
        return Ok(());
    }
    let broke = proof.blockers().len() > proof.weakened.len();
    let ran: Vec<&Check> = proof.ran_here().collect();
    let passed = ran.iter().filter(|c| c.outcome == Ran::Passed).count();
    let tests: u64 = ran.iter().filter_map(|c| c.tests).sum();
    let seconds: f64 = ran.iter().map(|c| c.seconds).sum();
    let not_run = proof.checks.len() - ran.len();
    write!(
        html,
        "<section class=\"part\" id=\"break\"><h2>Did anything break? <span class=\"word {}\">{}</span></h2><p class=\"sum num\">{} ran, {passed} passed",
        if broke { "changed" } else { "same" },
        if broke { "Yes" } else { "No" },
        ran.len()
    )?;
    if tests > 0 {
        write!(html, ", {tests} tests")?;
    }
    write!(html, ", {}", duration(seconds))?;
    if not_run > 0 {
        write!(html, ", {not_run} not run")?;
    }
    html.push_str("</p><div class=\"checks\">");
    // What needs attention first: failed, then anything with something to read.
    let rank = |c: &Check| {
        if c.outcome == Ran::Failed || c.unstable || c.red.as_ref().is_some_and(|r| r.blocks) {
            0
        } else if c.outcome == Ran::NotRun {
            3
        } else if !c.findings.is_empty() || c.red.is_some() {
            1
        } else {
            2
        }
    };
    let mut order: Vec<&Check> = proof.checks.iter().collect();
    order.sort_by_key(|c| rank(c));
    for c in order {
        check(html, dir, c, rank(c) == 0)?;
    }
    html.push_str("</div></section>");
    Ok(())
}

fn check(html: &mut String, dir: &Path, c: &Check, open: bool) -> Result<()> {
    let dot = match c.outcome {
        Ran::Failed => "bad",
        Ran::NotRun => "off",
        Ran::Passed if c.unstable => "bad",
        Ran::Passed => "ok",
    };
    write!(
        html,
        "<details class=\"chk\" id=\"check-{id}\"{}><summary><span class=\"dot {dot}\"></span><b>{id}</b><span class=\"meta num\">",
        if open { " open" } else { "" },
        id = esc(&c.id)
    )?;
    match (c.outcome, c.tests) {
        (Ran::NotRun, _) => html.push_str("not run"),
        (_, Some(1)) => html.push_str("1 test"),
        (_, Some(n)) => write!(html, "{n} tests")?,
        (_, None) => html.push_str(&duration(c.seconds)),
    }
    html.push_str("</span><span class=\"badges\">");
    if c.outcome == Ran::Failed {
        html.push_str("<span class=\"word changed\">Failed</span>");
    }
    if c.unstable {
        html.push_str("<span class=\"word changed\">Unstable</span>");
    }
    if let Some(red) = &c.red {
        let (class, label) = match red.verdict {
            Verdict::Red | Verdict::Unbuilt => ("same", "Red before green"),
            Verdict::NotRed if red.blocks => ("changed", "Not red"),
            Verdict::NotRed => ("gone", "Not red"),
            Verdict::BaseFails => ("gone", "Base fails"),
            Verdict::DidNotRun if red.blocks => ("changed", "Red did not run"),
            Verdict::DidNotRun => ("gone", "Red did not run"),
        };
        write!(html, "<span class=\"word {class}\">{label}</span>")?;
    }
    let gates = c.findings.iter().filter(|f| f.level == Level::Gate).count();
    let flags = c.findings.len() - gates;
    if gates > 0 {
        write!(
            html,
            "<span class=\"word changed num\">{gates} blocking</span>"
        )?;
    }
    if flags > 0 {
        write!(
            html,
            "<span class=\"word timing num\">{flags} to read</span>"
        )?;
    }
    html.push_str("</span></summary><div class=\"body\">");
    if let Some(f) = &c.failure {
        write!(
            html,
            "<p class=\"why\">{}</p><p class=\"fix\">{}</p>",
            prose(&capital(&f.why)),
            prose(&capital(&f.fix))
        )?;
    }
    if let Some(why) = &c.not_run {
        write!(html, "<p class=\"why\">{}</p>", prose(&capital(why)))?;
    }
    if let Some(red) = &c.red {
        write!(html, "<p>{}</p>", prose(&red.note(&c.id)))?;
    }
    if !c.findings.is_empty() {
        html.push_str("<ul class=\"findings\">");
        for f in &c.findings {
            let (class, word) = match f.level {
                Level::Gate => ("changed", "blocks"),
                Level::Flag => ("timing", "flag"),
                Level::Hint => ("gone", "hint"),
            };
            write!(
                html,
                "<li><span class=\"word {class}\">{word}</span><code>{}:{}</code> {}<span class=\"q\"> {}</span>",
                esc(&f.file),
                f.line,
                esc(&f.message),
                esc(&f.rule)
            )?;
            if let Some(fix) = &f.fix {
                write!(html, "<span class=\"q\">: {}</span>", esc(fix))?;
            }
            html.push_str("</li>");
        }
        html.push_str("</ul>");
    }
    if c.findings_elsewhere > 0 {
        write!(
            html,
            "<p class=\"q\">{} more on lines this change did not touch.</p>",
            c.findings_elsewhere
        )?;
    }
    if !c.cases.is_empty() {
        let failed = c.cases.iter().filter(|t| t.failed).count();
        write!(
            html,
            "<details class=\"cases\"{}><summary>{} tests{}</summary><ul>",
            if failed > 0 { " open" } else { "" },
            c.cases.len(),
            if failed > 0 {
                format!(", {failed} failed")
            } else {
                String::new()
            }
        )?;
        let mut cases: Vec<_> = c.cases.iter().collect();
        cases.sort_by_key(|t| !t.failed);
        for t in cases {
            write!(
                html,
                "<li class=\"{}\">{}</li>",
                if t.failed { "failed" } else { "passed" },
                esc(&t.name)
            )?;
        }
        html.push_str("</ul></details>");
    }
    if let Some(rel) = &c.output {
        let tail = crate::run::tail(&dir.join(rel), 40);
        let shown = c.outcome == Ran::Failed;
        write!(
            html,
            "<details class=\"out\"{}><summary>{}</summary><pre>",
            if shown { " open" } else { "" },
            if shown {
                "The end of what it printed"
            } else {
                "What it printed"
            }
        )?;
        for line in &tail {
            writeln!(html, "{}", esc(line))?;
        }
        write!(
            html,
            "</pre><p><a href=\"{}\">Everything it printed</a></p></details>",
            esc(rel)
        )?;
    }
    write!(
        html,
        "<details class=\"cmdline\"><summary>Command</summary><pre>{}</pre></details></div></details>",
        esc(&c.run)
    )?;
    Ok(())
}

/// Whether it looks right: every look with its pictures, the unchanged ones a filter away.
fn looks(html: &mut String, dir: &Path, proof: &Proof) -> Result<()> {
    if proof.looks.is_empty() {
        return Ok(());
    }
    html.push_str("<section class=\"part\" id=\"look\">");
    if !proof.checks.is_empty() {
        html.push_str("<h2>Does it look right to you?</h2>");
    }
    let mut n = 0;
    for look in &proof.looks {
        let check = esc(&look.check);
        write!(
            html,
            "<section class=\"look\" id=\"look-{check}\" data-check=\"{check}\"><div class=\"look-head\"><h3>{check}</h3><span class=\"q\">{}</span>",
            esc(&look.counts())
        )?;
        if look.no_before.is_some() {
            html.push_str("<span class=\"word new\">No before</span>");
        }
        // A note is something to know before approving, so its details start open.
        write!(
            html,
            "<details class=\"about\"{}><summary>Base</summary><p>{}</p>",
            if look.note.is_some() { " open" } else { "" },
            esc(&match &look.no_before {
                Some(why) => format!("There is no before: {why}. Every picture counts as new."),
                None => format!(
                    "Compared with the same check on the base commit {}.",
                    look.base_commit.get(..10).unwrap_or(&look.base_commit)
                ),
            })
        )?;
        if let Some(note) = &look.note {
            write!(html, "<p class=\"note-text\">{}.</p>", esc(&capital(note)))?;
        }
        html.push_str("</details></div>");
        counts(html, look)?;
        group_bar(html, look)?;
        html.push_str("<div class=\"list\">");
        for item in &look.items {
            card(html, dir, n, item)?;
            n += 1;
        }
        html.push_str("</div></section>");
    }
    html.push_str("<details class=\"keys\" id=\"keys\"><summary>Keys</summary><p><kbd>j</kbd> <kbd>k</kbd> next and back, <kbd>1</kbd> to <kbd>5</kbd> view, <kbd>y</kbd> looks right, <kbd>n</kbd> not right, <kbd>f</kbd> flaky</p></details></section>");
    Ok(())
}

/// Text with `code` in backticks, as Fearlezz writes its messages.
fn prose(text: &str) -> String {
    let mut out = String::new();
    for (k, part) in text.split('`').enumerate() {
        if k % 2 == 1 {
            out.push_str("<code>");
            out.push_str(&esc(part));
            out.push_str("</code>");
        } else {
            out.push_str(&esc(part));
        }
    }
    out
}

fn capital(text: &str) -> String {
    let mut out = text.to_string();
    if let Some(first) = out.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    out
}

/// `0.4 s`, `12 s` or `6 min 12 s`.
fn duration(seconds: f64) -> String {
    if seconds < 10.0 {
        return format!("{seconds:.1} s");
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a positive duration in whole seconds"
    )]
    let whole = seconds.round() as u64;
    match (whole / 60, whole % 60) {
        (0, s) => format!("{s} s"),
        (m, 0) => format!("{m} min"),
        (m, s) => format!("{m} min {s} s"),
    }
}

/// One button per kind of change there is; each filters the list to its kind.
fn counts(html: &mut String, look: &Look) -> std::fmt::Result {
    html.push_str("<nav class=\"counts\" aria-label=\"Filter\">");
    for c in ORDER {
        let n = look.items.iter().filter(|i| i.change == c).count();
        if n == 0 {
            continue;
        }
        let label = c.label();
        write!(
            html,
            "<button type=\"button\" class=\"count {label}\" data-filter=\"{label}\" aria-pressed=\"false\"><b class=\"num\" data-count=\"{n}\">{n}</b><span>{}</span></button>",
            word(c)
        )?;
    }
    html.push_str("</nav>");
    Ok(())
}

/// Group by change, and by each part of the names that differs between pictures.
fn group_bar(html: &mut String, look: &Look) -> std::fmt::Result {
    let parts: Vec<Parts> = look.items.iter().map(|i| Parts::of(&i.name)).collect();
    let differs = |f: fn(&Parts) -> &str| {
        let mut seen: Vec<&str> = parts.iter().map(f).filter(|v| !v.is_empty()).collect();
        seen.sort_unstable();
        seen.dedup();
        seen.len() > 1
    };
    let mut options = vec![("change", "Change")];
    if differs(|p| &p.screen) {
        options.push(("screen", "Screen"));
    }
    if differs(|p| &p.viewport) {
        options.push(("viewport", "Viewport"));
    }
    if differs(|p| &p.theme) {
        options.push(("theme", "Theme"));
    }
    if options.len() < 2 {
        return Ok(());
    }
    html.push_str("<div class=\"bar\"><span>Group by</span><div class=\"seg\" role=\"group\" aria-label=\"Group by\">");
    for (k, (key, label)) in options.iter().enumerate() {
        write!(
            html,
            "<button type=\"button\" data-group=\"{key}\" aria-pressed=\"{}\">{label}</button>",
            k == 0
        )?;
    }
    html.push_str("</div></div>");
    Ok(())
}

/// A picture name in parts: the viewport and theme it names, and the screen (the rest).
struct Parts {
    screen: String,
    viewport: String,
    theme: String,
}

impl Parts {
    fn of(name: &str) -> Parts {
        const VIEWPORTS: [&str; 11] = [
            "desktop", "mobile", "phone", "tablet", "laptop", "wide", "narrow", "ipad", "iphone",
            "android", "watch",
        ];
        const THEMES: [&str; 4] = ["light", "dark", "high-contrast", "contrast"];
        let mut out = Parts {
            screen: String::new(),
            viewport: String::new(),
            theme: String::new(),
        };
        let mut rest = Vec::new();
        // A file name shows without its extension.
        let bare = match name.rsplit_once('.') {
            Some((stem, ext)) if !stem.is_empty() && !ext.contains('/') && ext.len() <= 4 => stem,
            _ => name,
        };
        for seg in bare.split('/') {
            let lower = seg.to_ascii_lowercase();
            let is_size = lower.split_once('x').is_some_and(|(w, h)| {
                !w.is_empty()
                    && !h.is_empty()
                    && (w.to_owned() + h).bytes().all(|b| b.is_ascii_digit())
            });
            if out.viewport.is_empty() && (VIEWPORTS.contains(&lower.as_str()) || is_size) {
                out.viewport = seg.to_string();
            } else if out.theme.is_empty() && THEMES.contains(&lower.as_str()) {
                out.theme = seg.to_string();
            } else {
                rest.push(seg);
            }
        }
        out.screen = if rest.is_empty() {
            bare.to_string()
        } else {
            rest.join("/")
        };
        out
    }
}

fn card(html: &mut String, dir: &Path, n: usize, item: &Item) -> Result<()> {
    let p = Parts::of(&item.name);
    let label = item.change.label();
    write!(
        html,
        "<article class=\"card\" id=\"c{n}\" tabindex=\"-1\" data-name=\"{}\" data-change=\"{label}\" data-screen=\"{}\" data-viewport=\"{}\" data-theme=\"{}\"{}><div class=\"head\"><h2>{}</h2>",
        esc(&item.name),
        esc(&p.screen),
        esc(&p.viewport),
        esc(&p.theme),
        if item.flaky {
            " data-flaky=\"config\""
        } else {
            ""
        },
        esc(&p.screen)
    )?;
    for tag in [&p.viewport, &p.theme] {
        if !tag.is_empty() {
            write!(html, "<span class=\"tag\">{}</span>", esc(tag))?;
        }
    }
    if item.kind == "film" {
        html.push_str("<span class=\"tag\">Film</span>");
    }
    write!(
        html,
        "<span class=\"word {label}\">{}</span>",
        word(item.change)
    )?;
    if item.flaky {
        html.push_str("<span class=\"word gone\">Flaky</span>");
    }
    html.push_str("<span class=\"badge\" aria-hidden=\"true\"><svg class=\"tick\" viewBox=\"0 0 16 16\"><path d=\"M3 8.5l3.2 3L13 4.5\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2.4\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/></svg><svg class=\"x\" viewBox=\"0 0 16 16\"><path d=\"M4 4l8 8M12 4l-8 8\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2.4\" stroke-linecap=\"round\"/></svg></span></div>");

    let film = item.kind == "film" || matches!(&item.comparison, Some(c) if !c.still);
    match (&item.before, &item.after, &item.comparison) {
        (Some(b), Some(a), Some(c)) if film && c.size_before == c.size_after => {
            film_view(html, b, a, c)?;
            if c.verdict == media::Verdict::Changed {
                // One film whose frames cannot be cut keeps its summary; the page still shows
                // every other item.
                let mut frames = String::new();
                match film_frames(&mut frames, dir, n, item, c) {
                    Ok(()) => html.push_str(&frames),
                    Err(e) => write!(
                        html,
                        "<p class=\"missing\">The frames of this film could not be cut: {}</p>",
                        esc(&format!("{e:#}"))
                    )?,
                }
            }
        }
        (Some(b), Some(a), Some(c)) => {
            let area: Vec<Box4> = if c.size_before == c.size_after {
                c.moments.iter().map(|m| m.area).collect()
            } else {
                Vec::new()
            };
            compare(html, b, a, c.size_after, &area)?;
        }
        (Some(b), Some(a), None) => compare(html, b, a, (0, 0), &[])?,
        (b, a, _) => single(html, b.as_deref(), a.as_deref())?,
    }
    write!(
        html,
        "<details class=\"more\"><summary>Details</summary><p>{}</p></details>",
        esc(&plain_summary(item))
    )?;
    if item.change != Change::Same {
        html.push_str("<div class=\"decide\"><button type=\"button\" class=\"yes\" data-decide=\"ok\" aria-pressed=\"false\">Looks right</button><button type=\"button\" class=\"no\" data-decide=\"no\" aria-pressed=\"false\">Not right</button>");
        write!(
            html,
            "<button type=\"button\" class=\"flake\" data-decide=\"flaky\" aria-pressed=\"{}\"{}>Flaky</button>",
            item.flaky,
            if item.flaky { " disabled" } else { "" }
        )?;
        html.push_str("<label class=\"note\"><div><textarea rows=\"2\" placeholder=\"What is wrong?\" aria-label=\"What is wrong\"></textarea></div></label></div>");
    }
    html.push_str("</article>");
    Ok(())
}

fn word(change: Change) -> &'static str {
    match change {
        Change::Changed => "Changed",
        Change::Timing => "Timing only",
        Change::New => "New",
        Change::Gone => "Gone",
        Change::Unreadable => "Could not read",
        Change::Same => "Unchanged",
    }
}

/// The comparison in plain words, behind the Details click.
fn plain_summary(item: &Item) -> String {
    let Some(c) = &item.comparison else {
        return match item.change {
            Change::New => "Not in the base.".into(),
            Change::Gone => "In the base, not anymore.".into(),
            _ => item.summary.clone(),
        };
    };
    match c.verdict {
        media::Verdict::Same => "Looks the same.".into(),
        media::Verdict::Timing => format!(
            "Same pictures, different timing: {:.2} s long, was {:.2} s.",
            c.total_after, c.total_before
        ),
        media::Verdict::Changed if c.size_before != c.size_after => format!(
            "The size changed from {}×{} to {}×{}.",
            c.size_before.0, c.size_before.1, c.size_after.0, c.size_after.1
        ),
        media::Verdict::Changed if c.still => format!("{}.", item.summary),
        media::Verdict::Changed => {
            let at: Vec<String> = c
                .moments
                .iter()
                .map(|m| format!("{:.2} s", m.after.0))
                .collect();
            format!("Changes at {}.", at.join(", "))
        }
    }
}

/// A before and its after, with the ways to compare them. Side by side is drawn here; the
/// script draws the other views from the data on the block.
fn compare(
    html: &mut String,
    before: &str,
    after: &str,
    (w, h): (usize, usize),
    marks: &[Box4],
) -> std::fmt::Result {
    let boxes = serde_json::to_string(marks).unwrap_or_else(|_| "[]".into());
    write!(
        html,
        "<div class=\"cmp\" data-mode=\"side\" data-before=\"{}\" data-after=\"{}\" data-w=\"{w}\" data-h=\"{h}\" data-marks=\"{}\"><div class=\"seg modes\" role=\"group\" aria-label=\"View\">",
        esc(before),
        esc(after),
        esc(&boxes)
    )?;
    for (k, (key, label)) in MODES.iter().enumerate() {
        write!(
            html,
            "<button type=\"button\" data-mode=\"{key}\" aria-pressed=\"{}\">{label}</button>",
            k == 0
        )?;
    }
    html.push_str("</div><div class=\"stage side\">");
    for (label, src) in [("Before", before), ("After", after)] {
        write!(
            html,
            "<figure><figcaption>{label}</figcaption><div class=\"frame{}>{}",
            sized(w, h),
            media_tag(src, label)
        )?;
        marks_html(html, marks, (w, h))?;
        html.push_str("</div></figure>");
    }
    html.push_str("</div></div>");
    Ok(())
}

/// A new or gone picture: the one side there is.
fn single(html: &mut String, before: Option<&str>, after: Option<&str>) -> std::fmt::Result {
    html.push_str("<div class=\"stage one\">");
    match (after, before) {
        (Some(src), _) => write!(
            html,
            "<figure><figcaption>After</figcaption><div class=\"frame\">{}</div></figure>",
            media_tag(src, "After")
        )?,
        (None, Some(src)) => write!(
            html,
            "<figure><figcaption>Before</figcaption><div class=\"frame\">{}</div></figure>",
            media_tag(src, "Before")
        )?,
        (None, None) => html.push_str("<p class=\"missing\">Neither side could be read.</p>"),
    }
    html.push_str("</div>");
    Ok(())
}

/// The rest of a frame's class attribute: as large as its picture shows, at most 640 pixels
/// high, so outlines sit on the picture.
fn sized(w: usize, h: usize) -> String {
    if w == 0 || h == 0 {
        return "\"".into();
    }
    let max_w = 640.0 * w as f64 / h as f64;
    format!(" sized\" style=\"aspect-ratio:{w}/{h};width:min(100%,{max_w:.0}px)\"")
}

fn marks_html(html: &mut String, marks: &[Box4], (w, h): (usize, usize)) -> std::fmt::Result {
    if w == 0 || h == 0 {
        return Ok(());
    }
    let pct = |v: u32, of: usize| f64::from(v) * 100.0 / of as f64;
    for b in marks {
        write!(
            html,
            "<div class=\"mark\" style=\"left:{:.2}%;top:{:.2}%;width:{:.2}%;height:{:.2}%\"></div>",
            pct(b[0], w),
            pct(b[1], h),
            pct(b[2].saturating_sub(b[0]), w),
            pct(b[3].saturating_sub(b[1]), h)
        )?;
    }
    Ok(())
}

/// Both films on one clock, with a lane per film: grey where the pictures match, the change
/// colour where they differ, and a jump to each changed moment.
fn film_view(html: &mut String, before: &str, after: &str, c: &Comparison) -> std::fmt::Result {
    let total = c.total_before.max(c.total_after).max(0.01);
    // When each pair starts in the after film, and what changed there.
    let steps: Vec<(f64, &Vec<Box4>)> = c.pairs.iter().map(|p| (p.after_at, &p.boxes)).collect();
    let steps = serde_json::to_string(&steps).unwrap_or_else(|_| "[]".into());
    write!(
        html,
        "<div class=\"film\" data-total=\"{total:.3}\" data-tb=\"{:.3}\" data-ta=\"{:.3}\" data-w=\"{}\" data-h=\"{}\" data-steps=\"{}\"><div class=\"stage side\">",
        c.total_before,
        c.total_after,
        c.size_after.0,
        c.size_after.1,
        esc(&steps)
    )?;
    for (label, src) in [("Before", before), ("After", after)] {
        write!(
            html,
            "<figure><figcaption>{label}</figcaption><div class=\"frame{}><video src=\"{}\" muted playsinline preload=\"auto\" aria-label=\"{label}\"></video><div class=\"marks\"></div></div></figure>",
            sized(c.size_after.0, c.size_after.1),
            esc(src)
        )?;
    }
    write!(
        html,
        "</div><div class=\"timeline\" role=\"slider\" tabindex=\"0\" aria-label=\"Time\" aria-valuemin=\"0\" aria-valuemax=\"{total:.2}\" aria-valuenow=\"0\">"
    )?;
    for (side, label) in [("before", "Before"), ("after", "After")] {
        write!(html, "<div class=\"lane {side}\">")?;
        let end = if side == "before" {
            c.total_before
        } else {
            c.total_after
        };
        for (start, stop, differs) in lane(&c.pairs, side == "before", end) {
            write!(
                html,
                "<i class=\"{}\" style=\"left:{:.3}%;width:{:.3}%\"></i>",
                if differs { "diff" } else { "same" },
                start / total * 100.0,
                (stop - start).max(0.0) / total * 100.0
            )?;
        }
        write!(html, "<em>{label}</em></div>")?;
    }
    html.push_str("<div class=\"playhead\"></div></div><div class=\"clock\"><button type=\"button\" class=\"chip play\">Play both</button><span class=\"t num\">0.00 s</span>");
    for m in &c.moments {
        write!(
            html,
            "<button type=\"button\" class=\"chip jump\" data-seek=\"{:.3}\">{:.2} s</button>",
            m.after.0, m.after.0
        )?;
    }
    html.push_str("</div></div>");
    Ok(())
}

/// The stretches of one film's lane: each run of pairs on the same state of that film, from
/// its start to the next state's start, differing when any of its pairs differs.
fn lane(pairs: &[Pair], before: bool, end: f64) -> Vec<(f64, f64, bool)> {
    let mut out: Vec<(usize, f64, bool)> = Vec::new();
    for p in pairs {
        let (state, at) = if before {
            (p.before, p.before_at)
        } else {
            (p.after, p.after_at)
        };
        match out.last_mut() {
            Some((s, _, differs)) if *s == state => *differs |= p.px > 0,
            _ => out.push((state, at, p.px > 0)),
        }
    }
    let starts: Vec<f64> = out.iter().map(|(_, at, _)| *at).collect();
    out.iter()
        .enumerate()
        .map(|(k, (_, at, differs))| (*at, starts.get(k + 1).copied().unwrap_or(end), *differs))
        .collect()
}

/// For each changed moment of a film: the matched before and after frames, outlined, with
/// the same ways to compare as a still. Behind a click.
fn film_frames(html: &mut String, dir: &Path, n: usize, item: &Item, c: &Comparison) -> Result<()> {
    let (Some(before), Some(after)) = (&item.before, &item.after) else {
        return Ok(());
    };
    // The most changed pair of each run of changed pairs stands for its moment.
    let mut picks: Vec<&Pair> = Vec::new();
    let mut best: Option<&Pair> = None;
    for p in &c.pairs {
        if p.px > 0 {
            if best.is_none_or(|b| p.px > b.px) {
                best = Some(p);
            }
        } else {
            picks.extend(best.take());
        }
    }
    picks.extend(best);
    if picks.is_empty() {
        return Ok(());
    }
    let frames_dir = dir.join("frames");
    let want_b: Vec<usize> = picks.iter().map(|p| p.before_frame).collect();
    let want_a: Vec<usize> = picks.iter().map(|p| p.after_frame).collect();
    let fb = media::extract(
        &dir.join(before),
        &want_b,
        FRAME_W,
        &frames_dir,
        &format!("{n}-b"),
    )?;
    let fa = media::extract(
        &dir.join(after),
        &want_a,
        FRAME_W,
        &frames_dir,
        &format!("{n}-a"),
    )?;
    let rel = |list: &[(usize, std::path::PathBuf)], frame: usize| -> Option<String> {
        list.iter()
            .find(|(f, _)| *f == frame)
            .and_then(|(_, p)| Some(format!("frames/{}", p.file_name()?.to_string_lossy())))
    };
    write!(
        html,
        "<details class=\"frames\"><summary>Changed frames ({})</summary>",
        picks.len()
    )?;
    for (k, p) in picks.iter().enumerate() {
        let title = match c.moments.get(k) {
            Some(m) if (m.after.0 - m.before.0).abs() >= 0.01 => {
                format!("At {:.2} s, was {:.2} s", m.after.0, m.before.0)
            }
            Some(m) => format!("At {:.2} s", m.after.0),
            None => format!("Change {}", k + 1),
        };
        write!(html, "<h3>{}</h3>", esc(&title))?;
        let (Some(b), Some(a)) = (rel(&fb, p.before_frame), rel(&fa, p.after_frame)) else {
            continue;
        };
        compare(html, &b, &a, c.size_after, &p.boxes)?;
    }
    html.push_str("</details>");
    Ok(())
}

fn media_tag(src: &str, label: &str) -> String {
    let lower = src.to_ascii_lowercase();
    if [".mp4", ".webm", ".mov", ".m4v"]
        .iter()
        .any(|e| lower.ends_with(e))
    {
        format!(
            "<video src=\"{}\" autoplay loop muted playsinline controls aria-label=\"{label}\"></video>",
            esc(src)
        )
    } else {
        format!("<img src=\"{}\" alt=\"{label}\">", esc(src))
    }
}
