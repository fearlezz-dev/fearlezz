import { describe, expect, test } from "bun:test";
import { Feed, apply, stateOf } from "./live";
import type { Check, Live, Run, Snapshot } from "./proof";

const run: Run = {
  id: "r1",
  head: "abc1234",
  base: "main",
  source: { kind: "ci", job: "proof" },
  state: "running",
  opened: 100,
  checks: [{ id: "fmt" }, { id: "tests" }],
};
const snapshot: Snapshot = {
  seq: 3,
  work: { repository: "owner/repo", branch: "feat/x" },
  runs: [run],
};
const passed = (id: string): Check => ({
  id,
  run: "true",
  stage: "working",
  outcome: "passed",
  seconds: 1.5,
  findings_elsewhere: 0,
});

describe("apply", () => {
  test("a check moves from planned to running to passed", () => {
    expect(stateOf(snapshot.runs[0]!.checks[0]!)).toBe("planned");
    const started = apply(snapshot, 4, "r1", { kind: "started", check: "fmt", at: 101 });
    expect(stateOf(started.runs[0]!.checks[0]!)).toBe("running");
    const ended = apply(started, 5, "r1", { kind: "finished", check: passed("fmt") });
    expect(stateOf(ended.runs[0]!.checks[0]!)).toBe("passed");
    expect(ended.seq).toBe(5);
  });

  test("an event the snapshot holds already changes nothing", () => {
    const same = apply(snapshot, 3, "r1", { kind: "started", check: "fmt", at: 101 });
    expect(same).toBe(snapshot);
  });

  test("a check the run did not plan joins at the end", () => {
    const next = apply(snapshot, 4, "r1", { kind: "finished", check: passed("extra") });
    expect(next.runs[0]!.checks.map((c) => c.id)).toEqual(["fmt", "tests", "extra"]);
  });

  test("a new run comes first, and done closes a run with its final entries", () => {
    const opened = apply(snapshot, 4, "r2", {
      kind: "opened",
      work: { ...snapshot.work, pull: { number: 7, title: "Seven" } },
      run: { ...run, id: "r2" },
    });
    expect(opened.runs.map((r) => r.id)).toEqual(["r2", "r1"]);
    expect(opened.work.pull?.number).toBe(7);
    const done = apply(opened, 5, "r2", { kind: "done", checks: [passed("fmt"), passed("tests")], looks: [] });
    expect(done.runs[0]!.state).toBe("done");
    expect(done.runs[0]!.checks.map(stateOf)).toEqual(["passed", "passed"]);
    expect(done.runs[1]!.state).toBe("running");
  });
});

describe("Feed", () => {
  test("events before the snapshot wait for it, and those it holds are dropped", () => {
    const feed = new Feed();
    const early: Live = { kind: "event", seq: 3, run: "r1", change: { kind: "started", check: "fmt", at: 1 } };
    const later: Live = { kind: "event", seq: 4, run: "r1", change: { kind: "started", check: "tests", at: 2 } };
    expect(feed.take(early)).toBeUndefined();
    expect(feed.take(later)).toBeUndefined();
    const state = feed.take({ kind: "snapshot", ...snapshot });
    expect(state?.seq).toBe(4);
    expect(state?.runs[0]!.checks.map(stateOf)).toEqual(["planned", "running"]);
  });
});
