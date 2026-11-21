// What a person said that the host has not settled yet: a verdict, a note, an edit. The page
// shows each at once, as the event the host will send, applied through the same `applyWork`
// as the host's own events, so a click never waits for the host on the far side of the
// world. A refusal drops it, and the page shows the work as the host has it again.

import { createSignal } from "solid-js";
import type { Said } from "./api";
import { applyWork } from "./live";
import type { Note, PictureVerdict, Snapshot, WorkChange } from "./proof";

/** How long a settled hope shows at most when no event follows it: the host sends its event
 * before it answers, but over the live connection, which may come first or after. */
const SETTLING_MS = 3000;

type Hope = { change: WorkChange; settled: boolean };
const [all, setAll] = createSignal<Hope[]>([]);

/** The work as the page shows it: as the host has it, with what a person said since. A note
 * still under the page's own id steps aside once the host's own event brought it, which
 * usually comes before the host answers, so it never shows twice. */
export function hoped(state: Snapshot, changes: readonly WorkChange[] = all().map((h) => h.change)): Snapshot {
  const arrived = (change: WorkChange) => change.kind === "note" && change.note.id.startsWith(PENDING) && (state.notes ?? []).some((n) => same(n, change.note));
  return changes.filter((c) => !arrived(c)).reduce((s, change) => ({ ...applyWork(s, s.seq + 1, change), seq: s.seq }), state);
}

/** Whether two notes say the same: on the same picture and side, the same region and moment,
 * the same words, by the same person. */
const same = (a: Note, b: Note) =>
  a.check === b.check &&
  a.picture === b.picture &&
  a.side === b.side &&
  a.by === b.by &&
  a.words === b.words &&
  (a.time ?? null) === (b.time ?? null) &&
  (["x", "y", "w", "h"] as const).every((k) => Math.abs(a.region[k] - b.region[k]) < 1e-6);

/** The live connection brought the work as the host has it now, with the events of the hopes
 * it settled: those are done. */
export const seen = () => setAll((h) => (h.some((o) => o.settled) ? h.filter((o) => !o.settled) : h));

/** Shows `change` until `ask` settles it: dropped at once when the host refuses (the error it
 * gave is the answer), or, once it took it, as the host's own event (`settled`, from what the
 * host answered, when it says) until the next live event or a moment passes. */
export async function hoping<T>(
  change: WorkChange,
  ask: () => Promise<Said<T>>,
  settled: (answer: T) => WorkChange | undefined = () => undefined,
): Promise<string | undefined> {
  const hope: Hope = { change, settled: false };
  setAll((h) => [...h, hope]);
  const drop = () => setAll((h) => h.filter((o) => o !== hope));
  const said = await ask();
  if (!said.ok) {
    drop();
    return said.error;
  }
  const taken: Hope = { change: settled(said.answer) ?? change, settled: true };
  setAll((h) => h.map((o) => (o === hope ? taken : o)));
  setTimeout(() => setAll((h) => h.filter((o) => o !== taken)), SETTLING_MS);
  return undefined;
}

/** Looks right on `verdict`'s content, which resolves the picture's open notes `open`. */
export function verdictChange(verdict: PictureVerdict, open: Note[]): WorkChange {
  return { kind: "verdict", verdict, resolved: open.map((n) => resolvedBy(n, verdict.by)) };
}

/** Note `note` resolved by `by`. */
export const resolvedBy = (note: Note, by: string): Note => ({ ...note, state: "resolved", resolved_by: by });

/** A note not on the host yet, under an id of the page's own until the host names it. */
let made = 0;
const PENDING = "pending-";
export const pendingId = () => `${PENDING}${++made}`;
