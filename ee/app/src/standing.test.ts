import { describe, expect, test } from "bun:test";
import type { Check, Review, ReviewPicture, Run, RunCheck } from "./proof";
import { headline } from "./standing";

const passed = (id: string): RunCheck => ({ id, entry: { id, outcome: "passed", seconds: 1 } as Check });
const failed = (id: string): RunCheck => ({ id, entry: { id, outcome: "failed", seconds: 1 } as Check });
const run = (checks: RunCheck[], more: Partial<Run> = {}): Run => ({
  id: "r1",
  head: "8f3c2a1aaaa",
  base: "main",
  source: { kind: "ci", job: "proof", run_id: 1, run_attempt: 1, runner_name: "runner" },
  state: "done",
  opened: 1,
  checks,
  ...more,
});
const waiting = { check: "screens", name: "card.png", key: "k", standing: "waits" } as ReviewPicture;
const review = (state: Review["state"], more: Partial<Review> = {}): Review => ({
  head: "8f3c2a1aaaa",
  state,
  pictures: [],
  weakened: [],
  title: { read: `read: ${state}`, write: `write: ${state}`, admin: `admin: ${state}` },
  ...more,
});
const writer = { login: "w", role: "write" } as const;
const admin = { login: "a", role: "admin" } as const;

describe("headline", () => {
  test("failed checks lead, whatever the review says, and every part agrees", () => {
    const line = headline(run([passed("lint"), failed("test"), failed("look")]), review("waits", { pictures: [waiting] }), writer);
    expect(line).toEqual({ tone: "bad", icon: "cross", word: "2 failed", detail: "test and look", title: "2 checks failed at 8f3c2a1" });
  });

  test("a running run counts its checks and names the ones running", () => {
    const line = headline(run([passed("lint"), { id: "test", started: 5 }, { id: "look" }], { state: "running" }), undefined, writer);
    expect(line.tone).toBe("run");
    expect(line.word).toBe("Proving 1 of 3");
    expect(line.detail).toBe("test running");
  });

  test("pictures waiting for a look wait for a writer, and for a look from a reader", () => {
    const r = review("waits", { pictures: [waiting, waiting] });
    expect(headline(run([passed("look")]), r, writer)).toMatchObject({ tone: "look", word: "Waits for you", detail: "2 pictures to look at", title: "write: waits" });
    expect(headline(run([passed("look")]), r, undefined)).toMatchObject({ word: "Waits for a look", title: "read: waits" });
    const again = { ...waiting, changed_since: { verdict: null, notes: [] } } as ReviewPicture;
    expect(headline(run([passed("look")]), review("waits", { pictures: [waiting, again] }), writer).detail).toBe("2 pictures to look at, 1 changed since a look");
    const noted = { ...waiting, standing: "needs_change", changed_since: { notes: ["n1"] } } as ReviewPicture;
    expect(headline(run([passed("look")]), review("needs_change", { pictures: [noted] }), writer).detail).toBe("1 picture needs change, 1 changed since a look");
  });

  test("a weakening waiting for an accept is a decision, for an admin to take", () => {
    const r = review("failed", { weakened: [{} as Review["weakened"][number]] });
    expect(headline(run([passed("lint")]), r, admin)).toMatchObject({ tone: "look", word: "Waits for you" });
    expect(headline(run([passed("lint")]), r, writer)).toMatchObject({ tone: "look", word: "Waits for an admin" });
  });

  test("a local run is unproven and waits for no one", () => {
    const local = run([passed("lint")], { source: { kind: "local", login: "claude" } });
    expect(headline(local, review("waits", { pictures: [waiting] }), writer)).toMatchObject({
      tone: "local",
      word: "Unproven",
      title: "Unproven: claude's local run of 8f3c2a1: its checks passed",
    });
  });

  test("an approved head with nothing to review is proved", () => {
    expect(headline(run([passed("lint")]), review("approved"), writer)).toMatchObject({ tone: "ok", word: "Proved", detail: "Every check passed, nothing to review" });
  });
});
