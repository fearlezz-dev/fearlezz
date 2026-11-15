// What a page knows of its work: the snapshot the live connection sends first, with each
// later event applied in turn. Events the snapshot already holds (by sequence number) are
// dropped, so a page opened while a run goes misses nothing and counts nothing twice.

import type { Change, Check, Live, Look, Ran, Run, RunCheck, Snapshot, WorkChange } from "./proof";

/** A check as a person reads it: not started yet, running, or how it came out. */
export type CheckState = "planned" | "running" | Ran;

export function stateOf(check: RunCheck): CheckState {
  if (check.entry) return check.entry.outcome;
  if (check.started != null) return "running";
  return "planned";
}

/** The snapshot with the event `seq` of run `run` applied, or the same snapshot when it holds
 * that event already. */
export function apply(state: Snapshot, seq: number, run: string, change: Change): Snapshot {
  if (seq <= state.seq) return state;
  const next = { ...state, seq };
  if (change.kind === "opened") {
    const others = state.runs.filter((r) => r.id !== change.run.id);
    return { ...next, work: change.work, runs: [change.run, ...others] };
  }
  return {
    ...next,
    runs: state.runs.map((r) => (r.id === run ? changeRun(r, change) : r)),
  };
}

/** The snapshot with the work's event `seq` applied, or the same snapshot when it holds it. */
export function applyWork(state: Snapshot, seq: number, change: WorkChange): Snapshot {
  if (seq <= state.seq) return state;
  const next = { ...state, seq };
  switch (change.kind) {
    case "verdict": {
      const v = change.verdict;
      const same = (o: typeof v) => o.check === v.check && o.picture === v.picture && o.key === v.key;
      return { ...next, verdicts: [...(state.verdicts ?? []).filter((o) => !same(o)), v] };
    }
    case "review":
      return { ...next, review: change.review ?? null };
    case "waiting":
      return { ...next, waiting: change.waiting };
  }
}

function changeRun(run: Run, change: Change): Run {
  switch (change.kind) {
    case "opened":
      return change.run;
    case "started":
      return { ...run, checks: upsert(run.checks, change.check, { started: change.at }) };
    case "finished": {
      const checks = withEntry(run.checks, change.check);
      return { ...run, checks: change.look ? withLook(checks, change.look, change.keys ?? {}) : checks };
    }
    case "done": {
      const keys = change.keys ?? {};
      return {
        ...run,
        state: "done",
        checks: change.looks.reduce(
          (checks, look) => withLook(checks, look, keys[look.check] ?? {}),
          change.checks.reduce(withEntry, run.checks),
        ),
      };
    }
    case "cancelled":
      return { ...run, state: "cancelled" };
  }
}

function withEntry(checks: RunCheck[], entry: Check): RunCheck[] {
  return upsert(checks, entry.id, { entry });
}

function withLook(checks: RunCheck[], look: Look, keys: Record<string, string>): RunCheck[] {
  return upsert(checks, look.check, { look, keys });
}

/** The checks with `id` changed by `fields`, or added at the end when the run did not plan it. */
function upsert(checks: RunCheck[], id: string, fields: Partial<RunCheck>): RunCheck[] {
  if (!checks.some((c) => c.id === id)) return [...checks, { id, ...fields }];
  return checks.map((c) => (c.id === id ? { ...c, ...fields } : c));
}

/** What a page's connection hands on: the work as of now, or that it is not connected. */
export type Status = { kind: "connecting" } | { kind: "live"; state: Snapshot } | { kind: "gone"; why: string };

/** Takes the messages of one live connection in order, keeping events that arrive before
 * the snapshot (the room takes a socket before it reads the snapshot) until it comes. */
export class Feed {
  private state: Snapshot | undefined;
  private early: Exclude<Live, { kind: "snapshot" }>[] = [];

  /** The work after `message`, once the snapshot came. */
  take(message: Live): Snapshot | undefined {
    if (message.kind === "snapshot") {
      const { kind: _, ...snapshot } = message;
      this.state = this.early.reduce(step, snapshot as Snapshot);
      this.early = [];
    } else if (this.state) {
      this.state = step(this.state, message);
    } else {
      this.early.push(message);
    }
    return this.state;
  }
}

function step(state: Snapshot, message: Exclude<Live, { kind: "snapshot" }>): Snapshot {
  return message.kind === "event"
    ? apply(state, message.seq, message.run, message.change)
    : applyWork(state, message.seq, message.change);
}

/** Keeps a live connection to the work at `path` (the page's own path) open, handing on each
 * state, and opens a fresh one, with a fresh snapshot, when it drops. */
export function connect(path: string, on: (status: Status) => void): () => void {
  let socket: WebSocket | undefined;
  let stopped = false;
  let wait = 500;
  const open = () => {
    const scheme = location.protocol === "https:" ? "wss:" : "ws:";
    const feed = new Feed();
    socket = new WebSocket(`${scheme}//${location.host}/live${path}`);
    socket.onmessage = (event) => {
      const state = feed.take(JSON.parse(String(event.data)) as Live);
      if (state) {
        wait = 500;
        on({ kind: "live", state });
      }
    };
    socket.onclose = (event) => {
      if (stopped) return;
      if (event.code === 1008) {
        on({ kind: "gone", why: event.reason || "This work is not here." });
        return;
      }
      on({ kind: "connecting" });
      setTimeout(open, wait);
      wait = Math.min(wait * 2, 10_000);
    };
  };
  open();
  return () => {
    stopped = true;
    socket?.close();
  };
}
