// What changed between two texts, for a person to see at a glance: the lines of each, kept,
// taken out or put in, and inside a line that was replaced, the words that differ.

/** A piece of a line: a word or the space between words, marked when it differs. */
export type Piece = { text: string; differs: boolean };

/** One line of a diff: kept from both, only in the old text, or only in the new. */
export type Line = { kind: "same" | "was" | "now"; pieces: Piece[] };

/** How many cells a longest common subsequence table may have; past it the two count as
 * sharing nothing, so a large text never stalls the page. */
const CELLS = 4_000_000;

/** The longest common subsequence of `a` and `b`, as pairs of indexes: the lines both start
 * and end with, and between them the longest common subsequence of the rest while its table
 * stays small. */
function common<T>(a: T[], b: T[]): [number, number][] {
  let start = 0;
  while (start < a.length && start < b.length && a[start] === b[start]) start++;
  let end = 0;
  while (end < a.length - start && end < b.length - start && a[a.length - 1 - end] === b[b.length - 1 - end]) end++;
  const pairs: [number, number][] = [];
  for (let k = 0; k < start; k++) pairs.push([k, k]);
  const midA = a.slice(start, a.length - end);
  const midB = b.slice(start, b.length - end);
  if (midA.length * midB.length <= CELLS) {
    for (const [i, j] of table(midA, midB)) pairs.push([i + start, j + start]);
  }
  for (let k = end; k > 0; k--) pairs.push([a.length - k, b.length - k]);
  return pairs;
}

/** The longest common subsequence of `a` and `b` by the full table. */
function table<T>(a: T[], b: T[]): [number, number][] {
  const n = a.length;
  const m = b.length;
  const len: number[][] = Array.from({ length: n + 1 }, () => new Array<number>(m + 1).fill(0));
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      len[i]![j] = a[i] === b[j] ? len[i + 1]![j + 1]! + 1 : Math.max(len[i + 1]![j]!, len[i]![j + 1]!);
    }
  }
  const pairs: [number, number][] = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (a[i] === b[j]) {
      pairs.push([i, j]);
      i++;
      j++;
    } else if (len[i + 1]![j]! >= len[i]![j + 1]!) i++;
    else j++;
  }
  return pairs;
}

/** Words, the spaces between them and each other sign on its own (a quote, a bracket), so
 * joining them gives the line back and a quote never drags the word it touches into a mark. */
const words = (line: string) => line.match(/\s+|[\w.-]+|[^\s\w.-]/g) ?? [];

/** Neighbouring pieces with the same mark joined, so one change reads as one mark. */
function joined(list: Piece[]): Piece[] {
  const out: Piece[] = [];
  for (const p of list) {
    const last = out[out.length - 1];
    if (last && last.differs === p.differs) last.text += p.text;
    else out.push({ ...p });
  }
  return out;
}

/** The pieces of an old and a new line, each marked where it is not in the other. */
function pieces(was: string, now: string): [Piece[], Piece[]] {
  const a = words(was);
  const b = words(now);
  const kept = common(a, b);
  const inA = new Set(kept.map(([i]) => i));
  const inB = new Set(kept.map(([, j]) => j));
  return [
    joined(a.map((text, i) => ({ text, differs: !inA.has(i) }))),
    joined(b.map((text, j) => ({ text, differs: !inB.has(j) }))),
  ];
}

const whole = (text: string, differs: boolean): Piece[] => [{ text, differs }];

/** A run of kept lines folded away, by how many. */
export type Fold = { count: number };

/** The lines of a diff with each run of kept lines longer than twice `context` folded to a
 * count, `context` kept lines staying on each side of a change. */
export function folded(lines: Line[], context: number): (Line | Fold)[] {
  const out: (Line | Fold)[] = [];
  let k = 0;
  while (k < lines.length) {
    if (lines[k]!.kind !== "same") {
      out.push(lines[k]!);
      k++;
      continue;
    }
    let end = k;
    while (end < lines.length && lines[end]!.kind === "same") end++;
    const keepBefore = k === 0 ? 0 : context;
    const keepAfter = end === lines.length ? 0 : context;
    if (end - k > keepBefore + keepAfter) {
      out.push(...lines.slice(k, k + keepBefore));
      out.push({ count: end - k - keepBefore - keepAfter });
      out.push(...lines.slice(end - keepAfter, end));
    } else {
      out.push(...lines.slice(k, end));
    }
    k = end;
  }
  return out;
}

/** The lines of `now` against `was` (either may be absent): kept lines, and each run of lines
 * taken out followed by lines put in, where a taken out line and the put in line at the same
 * place in the run show the words that differ. */
export function diff(was: string | undefined | null, now: string | undefined | null): Line[] {
  const a = was ? was.replace(/\n$/, "").split("\n") : [];
  const b = now ? now.replace(/\n$/, "").split("\n") : [];
  const lines: Line[] = [];
  const flush = (out: string[], into: string[]) => {
    const paired = Math.min(out.length, into.length);
    const outPieces = out.map((l, k) => (k < paired ? pieces(l, into[k]!)[0] : whole(l, false)));
    const intoPieces = into.map((l, k) => (k < paired ? pieces(out[k]!, l)[1] : whole(l, false)));
    for (const p of outPieces) lines.push({ kind: "was", pieces: p });
    for (const p of intoPieces) lines.push({ kind: "now", pieces: p });
  };
  let i = 0;
  let j = 0;
  for (const [ki, kj] of [...common(a, b), [a.length, b.length] as [number, number]]) {
    flush(a.slice(i, ki), b.slice(j, kj));
    if (ki < a.length) lines.push({ kind: "same", pieces: whole(a[ki]!, false) });
    i = ki + 1;
    j = kj + 1;
  }
  return lines;
}
