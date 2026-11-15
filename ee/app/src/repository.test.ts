import { describe, expect, test } from "bun:test";
import { grantPath } from "./grant";
import { placeOf, sharedBy } from "./place";
import type { Listed } from "./proof";
import { ListingFeed, standingOf, upsert, waitsOn } from "./repository";
import { maySignIn, onlyViews } from "./review";

const listed = (branch: string, seq: number, active: number, more: Partial<Listed> = {}): Listed => ({
  work: { repository: "owner/repo", branch },
  path: `/owner/repo/work/${branch}`,
  seq,
  active,
  pictures: 0,
  notes: 0,
  weakened: false,
  ...more,
});

describe("the repository page", () => {
  test("a work that changes moves to the top; an older change of it is dropped", () => {
    const state = { repository: "owner/repo", works: [listed("b", 4, 20), listed("a", 7, 10)] };
    const moved = upsert(state, listed("a", 8, 30));
    expect(moved.works.map((w) => w.work.branch)).toEqual(["a", "b"]);
    expect(upsert(moved, listed("a", 6, 40))).toBe(moved);
    const added = upsert(moved, listed("c", 1, 25));
    expect(added.works.map((w) => w.work.branch)).toEqual(["a", "c", "b"]);
  });

  test("a work that changes before the listing arrives is kept until it comes", () => {
    const feed = new ListingFeed();
    expect(feed.take({ kind: "work", ...listed("a", 9, 50) })).toBeUndefined();
    const works = feed.take({ kind: "snapshot", repository: "owner/repo", works: [listed("b", 2, 40), listed("a", 8, 30)] });
    expect(works?.works.map((w) => [w.work.branch, w.seq])).toEqual([
      ["a", 9],
      ["b", 2],
    ]);
  });

  test("each work says where its newest run stands and what waits on it", () => {
    const ci = { kind: "ci" as const, job: "proof" };
    expect(standingOf(listed("a", 1, 1))).toBe("none");
    expect(standingOf(listed("a", 1, 1, { latest: { head: "h", source: ci, state: "running", failed: 0 } }))).toBe("running");
    expect(standingOf(listed("a", 1, 1, { latest: { head: "h", source: ci, state: "done", failed: 2 } }))).toBe("failed");
    expect(standingOf(listed("a", 1, 1, { latest: { head: "h", source: ci, state: "done", failed: 0 } }))).toBe("passed");
    expect(standingOf(listed("a", 1, 1, { latest: { head: "h", source: ci, state: "cancelled", failed: 0 } }))).toBe("cancelled");
    expect(waitsOn(listed("a", 1, 1, { pictures: 1, notes: 2, weakened: true }))).toEqual([
      "1 picture to look at",
      "2 open notes",
      "a weakening to accept",
    ]);
    expect(waitsOn(listed("a", 1, 1))).toEqual([]);
  });
});

describe("where the page is", () => {
  test("a path names a repository, a work, or a share link", () => {
    expect(placeOf("/owner/repo")).toEqual({ kind: "repository", repository: "owner/repo" });
    expect(placeOf("/owner/repo/work/feat/x")).toEqual({ kind: "work" });
    expect(placeOf("/owner/repo/pull/3")).toEqual({ kind: "work" });
    expect(placeOf("/-/share/abc")).toEqual({ kind: "share", token: "abc" });
    expect(sharedBy("/owner/repo/pull/3")).toBeUndefined();
  });

  test("through a share link the files are asked for with its token", () => {
    expect(grantPath("r1", undefined)).toBe("/api/v2/runs/r1/grant");
    expect(grantPath("r1", "t0k")).toBe("/api/v2/runs/r1/grant?share=t0k");
  });

  test("a share link only views and asks no one to sign in", () => {
    expect(onlyViews({ login: null, shared: true })).toBe(true);
    expect(maySignIn({ login: null, shared: true })).toBe(false);
    expect(maySignIn({ login: null })).toBe(true);
    expect(onlyViews({ login: "writer", role: "write" })).toBe(false);
  });
});
