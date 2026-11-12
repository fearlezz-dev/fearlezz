//! What the static pages share: colours, type, controls and motion, and HTML escaping.
//! Pages are single files with nothing fetched from the network.

/// A light table: pictures and cards sit on mats on a quiet surface. Each kind of change has
/// one colour, used for its count, its word, its outlines and its marks on a timeline.
pub(crate) const STYLE: &str = r#"
:root {
  --surface:#e7eaee; --mat:#ffffff; --sunk:#dce0e5; --ink:#121417; --quiet:#58606c; --line:#cdd3da;
  --changed:#c8106f; --timing:#9a5300; --new:#1f5fd1; --gone:#626a76; --same:#1d7a4a;
  --ok:#1d7a4a; --focus:#1f5fd1; --on-accent:#ffffff;
  --shadow:0 1px 2px rgba(18,20,23,.06), 0 10px 28px rgba(18,20,23,.09);
  --ease:cubic-bezier(.2,.8,.2,1);
  color-scheme: light dark;
}
@media (prefers-color-scheme: dark) {
  :root {
    --surface:#101318; --mat:#1a1e25; --sunk:#252a33; --ink:#eceef2; --quiet:#a3aab5; --line:#323943;
    --changed:#ff4fa6; --timing:#f5a524; --new:#7aa7ff; --gone:#9aa1ad; --same:#4fd08a;
    --ok:#4fd08a; --focus:#7aa7ff; --on-accent:#0d0f12;
    --shadow:0 1px 2px rgba(0,0,0,.5), 0 12px 32px rgba(0,0,0,.4);
  }
}
* { box-sizing: border-box; }
[hidden] { display: none !important; }
html { -webkit-text-size-adjust: 100%; }
body { margin: 0; background: var(--surface); color: var(--ink);
  font: 17px/1.5 "Atkinson Hyperlegible", system-ui, -apple-system, "Segoe UI", sans-serif; }
h1, h2, h3 { font-family: ui-rounded, "SF Pro Rounded", "Atkinson Hyperlegible", system-ui, sans-serif;
  letter-spacing: -0.01em; line-height: 1.15; overflow-wrap: anywhere; }
button { font: inherit; color: inherit; cursor: pointer; }
:focus-visible { outline: 3px solid var(--focus); outline-offset: 2px; border-radius: 8px; }
.num { font-variant-numeric: tabular-nums; }

/* A row of choices with a pill that slides to the chosen one. */
.seg { position: relative; display: inline-flex; gap: 2px; padding: 3px; background: var(--sunk);
  border-radius: 12px; max-width: 100%; overflow-x: auto; scrollbar-width: none; vertical-align: middle; }
.seg::-webkit-scrollbar { display: none; }
.seg button { position: relative; z-index: 1; border: 0; background: none; padding: 6px 14px;
  border-radius: 9px; white-space: nowrap; color: var(--quiet); transition: color .2s; }
.seg button:hover { color: var(--ink); }
.seg button[aria-pressed="true"] { color: var(--ink); font-weight: 700; }
.seg .pill { position: absolute; z-index: 0; top: 3px; bottom: 3px; left: 0; width: 0; border-radius: 9px;
  background: var(--mat); box-shadow: 0 1px 3px rgba(0,0,0,.14);
  transition: transform .34s var(--ease), width .34s var(--ease); }

/* The word for a state, in its colour. */
.word { display: inline-block; font-weight: 700; font-size: 15px; padding: 1px 10px; border-radius: 999px;
  border: 1.5px solid currentColor; white-space: nowrap; }
.changed { color: var(--changed); } .timing { color: var(--timing); } .new { color: var(--new); }
.gone { color: var(--gone); } .same { color: var(--same); } .unreadable { color: var(--changed); }
.tag { display: inline-block; font-size: 15px; padding: 1px 9px; border-radius: 7px; background: var(--sunk);
  color: var(--quiet); white-space: nowrap; }

details > summary { cursor: pointer; color: var(--quiet); list-style: none; display: inline-flex;
  align-items: center; gap: 6px; border-radius: 8px; transition: color .2s; }
details > summary::-webkit-details-marker { display: none; }
details > summary::before { content: ""; width: 7px; height: 7px; border: solid currentColor;
  border-width: 0 2px 2px 0; transform: rotate(-45deg); transition: transform .25s var(--ease); }
details[open] > summary::before { transform: rotate(45deg); }
details > summary:hover { color: var(--ink); }

@keyframes rise { from { opacity: 0; transform: translateY(8px); } to { opacity: 1; transform: none; } }
.rise { animation: rise .45s var(--ease) both; }
@media (prefers-reduced-motion: reduce) {
  *, *::before, *::after { animation-duration: .01ms !important; animation-delay: 0s !important;
    transition-duration: .01ms !important; scroll-behavior: auto !important; }
}
"#;

/// Motion the pages share: pills that slide, counts that count up, and items that rise in
/// one after another when the page opens.
pub(crate) const SCRIPT: &str = r#"
var calm = window.matchMedia('(prefers-reduced-motion: reduce)');
function placePill(seg) {
  var on = seg.querySelector('[aria-pressed="true"]');
  var pill = seg.querySelector('.pill');
  if (!pill) { pill = document.createElement('span'); pill.className = 'pill'; seg.prepend(pill); }
  if (!on || !on.offsetWidth) { pill.style.width = '0'; return; }
  pill.style.width = on.offsetWidth + 'px';
  pill.style.transform = 'translateX(' + on.offsetLeft + 'px)';
}
function countTo(el, to) {
  var from = +el.dataset.shown || 0;
  el.dataset.shown = to;
  if (calm.matches || from === to) { el.textContent = to; return; }
  var start = performance.now(), took = 520;
  (function frame(now) {
    var k = Math.min(1, (now - start) / took), e = 1 - Math.pow(1 - k, 3);
    el.textContent = Math.round(from + (to - from) * e);
    if (k < 1) requestAnimationFrame(frame);
  })(start);
}
function riseIn(els) {
  Array.prototype.forEach.call(els, function (el, i) {
    el.style.animationDelay = Math.min(i, 10) * 45 + 'ms';
    el.classList.add('rise');
  });
}
document.querySelectorAll('[data-count]').forEach(function (el) { countTo(el, +el.dataset.count); });
window.addEventListener('load', function () { document.querySelectorAll('.seg').forEach(placePill); });
window.addEventListener('resize', function () { document.querySelectorAll('.seg').forEach(placePill); });
"#;

pub(crate) fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
