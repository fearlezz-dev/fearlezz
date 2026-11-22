import { describe, expect, test } from "bun:test";
import { Feed, apply, applyWork, namedTests, stateOf } from "./live";
import type { Case, Check, Live, PictureVerdict, Review, Run, Snapshot } from "./proof";

const run: Run = {
  id: "r1",
  head: "abc1234",
  base: "main",
  source: { kind: "ci", job: "proof", run_id: 1, run_attempt: 1, runner_name: "runner" },
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

describe("review", () => {
  const verdict = (by: string): PictureVerdict => ({
    check: "screens",
    picture: "card.png",
    key: "still:b:a",
    by,
    at: 1,
    run: "r1",
    head: "abc1234",
  });

  test("a verdict joins the work's, replacing one on the same content", () => {
    const one = applyWork(snapshot, 4, { kind: "verdict", verdict: verdict("writer") });
    const two = applyWork(one, 5, { kind: "verdict", verdict: verdict("admin") });
    expect(two.verdicts?.map((v) => v.by)).toEqual(["admin"]);
    expect(applyWork(two, 5, { kind: "verdict", verdict: verdict("x") })).toBe(two);
  });

  test("a verdict taken back leaves the work's, only that content's", () => {
    const one = applyWork(snapshot, 4, { kind: "verdict", verdict: verdict("writer") });
    const other = { ...verdict("writer"), key: "still:c:d" };
    const two = applyWork(one, 5, { kind: "verdict", verdict: other });
    const three = applyWork(two, 6, { kind: "withdrawn", check: "screens", picture: verdict("x").picture, key: "still:b:a" });
    expect(three.verdicts).toEqual([other]);
  });

  test("a verdict that resolved notes takes them off the page", () => {
    const note = { id: "n1", check: "screens", picture: verdict("x").picture, run: "r1", head: "abc1234", side: "after" as const, region: { x: 0, y: 0, w: 1, h: 1 }, words: "tight", by: "w", at: 1, state: "open" as const };
    const noted = { ...snapshot, notes: [note, { ...note, id: "n2", picture: "other.png" }] };
    const next = applyWork(noted, 4, { kind: "verdict", verdict: verdict("writer"), resolved: [{ ...note, state: "resolved", resolved_by: "writer" }] });
    expect(next.notes?.map((n) => n.id)).toEqual(["n2"]);
  });

  test("a note takes back the looks right on its picture, only that picture's", () => {
    const other = { ...verdict("writer"), picture: "other.png" };
    const one = applyWork(applyWork(snapshot, 4, { kind: "verdict", verdict: verdict("writer") }), 5, { kind: "verdict", verdict: other });
    const note = { id: "n1", check: "screens", picture: "card.png", run: "r1", head: "abc1234", side: "after" as const, region: { x: 0, y: 0, w: 1, h: 1 }, words: "tight", by: "w", at: 1, state: "open" as const };
    const next = applyWork(one, 6, { kind: "note", note });
    expect(next.verdicts).toEqual([other]);
    expect(next.notes?.map((n) => n.id)).toEqual(["n1"]);
  });

  test("edited notes take their new words", () => {
    const note = { id: "n1", check: "screens", picture: "card.png", run: "r1", head: "abc1234", side: "after" as const, region: { x: 0, y: 0, w: 1, h: 1 }, words: "tight", by: "w", at: 1, state: "open" as const };
    const noted = { ...snapshot, notes: [note, { ...note, id: "n2", words: "dark" }] };
    const next = applyWork(noted, 4, { kind: "edited", notes: [{ ...note, words: "more room" }] });
    expect(next.notes?.map((n) => n.words)).toEqual(["more room", "dark"]);
  });

  test("the review and the waiting jobs are replaced as they come", () => {
    const title = { read: "", write: "", admin: "" };
    const review: Review = { head: "abc1234", state: "waits", pictures: [], weakened: [], title };
    const next = applyWork(snapshot, 4, { kind: "review", review });
    expect(next.review?.state).toBe("waits");
    const waiting = applyWork(next, 5, { kind: "waiting", waiting: [{ name: "proof", head: "def", since: 1 }] });
    expect(waiting.waiting?.map((j) => j.name)).toEqual(["proof"]);
  });

  test("a look brings the keys of its pictures, and a cancelled run is cancelled", () => {
    const look = { check: "tests", base_commit: "b", base_result: "passed", items: [] };
    const next = apply(snapshot, 4, "r1", { kind: "finished", check: passed("tests"), look, keys: { "card.png": "k" } });
    expect(next.runs[0]!.checks[1]!.keys).toEqual({ "card.png": "k" });
    const done = apply(next, 5, "r1", { kind: "done", checks: [], looks: [look], keys: { tests: { "card.png": "k2" } } });
    expect(done.runs[0]!.checks[1]!.keys).toEqual({ "card.png": "k2" });
    expect(apply(done, 6, "r1", { kind: "cancelled" }).runs[0]!.state).toBe("cancelled");
  });

  test("work events before the snapshot wait for it too", () => {
    const feed = new Feed();
    expect(feed.take({ kind: "work", seq: 4, change: { kind: "verdict", verdict: verdict("writer") } })).toBeUndefined();
    const state = feed.take({ kind: "snapshot", ...snapshot });
    expect(state?.verdicts?.length).toBe(1);
  });
});

describe("namedTests", () => {
  const ran = (cases: Case[]): Check => ({ ...passed("tests"), outcome: "failed", cases });

  test("failed tests come first, then those listed flaky; a pass is not named", () => {
    const named = namedTests(
      ran([
        { name: "listed", failed: false, flaky: true },
        { name: "adds", failed: false },
        { name: "logs in", failed: true },
        { name: "retried", failed: true, flaky: true },
      ]),
    );
    expect(named.tests).toEqual([
      { name: "logs in", state: "failed" },
      { name: "retried", state: "flaky" },
      { name: "listed", state: "listed" },
    ]);
    expect(named.more).toBe(0);
  });

  test("many failures are cut short with how many more", () => {
    const cases = Array.from({ length: 13 }, (_, i) => ({ name: `t${i}`, failed: true }));
    const named = namedTests(ran(cases));
    expect(named.tests.length).toBe(10);
    expect(named.more).toBe(3);
  });

  test("a check that has not ended names nothing", () => {
    expect(namedTests(undefined)).toEqual({ tests: [], more: 0 });
  });
});
