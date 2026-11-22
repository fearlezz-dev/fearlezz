// Full screen never scrolls: the pictures get the height the window has left once everything
// else on it (the stage's head, the strip, a film's timeline, the review below) is laid out, so
// stepping through pictures always shows the whole of each with its review.

/** The least a picture is given, so a window crowded with notes still shows one. */
const LEAST = 120;

/** The height each picture in full screen `table` may take: the window's height less what the
 * rest takes, shared by the rows of pictures (side by side is one row, stacked on a phone two).
 * None when it shows no picture. */
export function fitOf(table: HTMLElement): number | undefined {
  const frames = [...table.querySelectorAll<HTMLElement>(".layers .frame")];
  if (!frames.length) return undefined;
  const rows = new Map<number, number>();
  for (const f of frames) {
    const box = f.getBoundingClientRect();
    const row = Math.round(box.top);
    rows.set(row, Math.max(rows.get(row) ?? 0, box.height));
  }
  const pictures = [...rows.values()].reduce((a, b) => a + b, 0);
  const style = getComputedStyle(table);
  const top = table.getBoundingClientRect().top - table.scrollTop;
  const bottom = Math.max(...[...table.children].map((c) => c.getBoundingClientRect().bottom));
  const used = bottom - top + Number.parseFloat(style.paddingBottom);
  // The space the pictures are centred in is theirs to take, not the rest's.
  const centred = table.querySelector<HTMLElement>(".stage > :last-child");
  const room = centred ? Number.parseFloat(getComputedStyle(centred).marginTop) + Number.parseFloat(getComputedStyle(centred).marginBottom) : 0;
  const rest = used - room - pictures;
  return Math.max(LEAST, Math.floor((table.clientHeight - rest) / rows.size));
}

/** Keeps `--fit` on full screen `table` as its pictures' height while it is laid out anew: a
 * picture or note arriving, the window resized. Stops when the returned function is called. */
export function fitting(table: HTMLElement): () => void {
  let frame = 0;
  const fit = () => {
    cancelAnimationFrame(frame);
    frame = requestAnimationFrame(() => {
      const h = fitOf(table);
      if (h === undefined) return;
      const was = Number.parseFloat(table.style.getPropertyValue("--fit"));
      // What the rest takes does not depend on the pictures' height, so this settles at once.
      if (!(Math.abs(was - h) < 1)) table.style.setProperty("--fit", `${h}px`);
    });
  };
  const sizes = new ResizeObserver(fit);
  const watch = () => {
    sizes.disconnect();
    sizes.observe(table);
    for (const c of table.children) sizes.observe(c);
    for (const c of table.querySelectorAll(".stage > *, .verdict > *")) sizes.observe(c);
  };
  const changes = new MutationObserver(() => {
    watch();
    fit();
  });
  changes.observe(table, { childList: true, subtree: true });
  watch();
  fit();
  return () => {
    cancelAnimationFrame(frame);
    sizes.disconnect();
    changes.disconnect();
    table.style.removeProperty("--fit");
  };
}
