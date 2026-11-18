import { describe, expect, test } from "bun:test";
import { grantPath } from "./grant";
import { placeOf, sharedBy } from "./place";
import type { Listed } from "./proof";
import { ListingFeed, upsert } from "./repository";
import { maySignIn, onlyViews } from "./review";

const listed = (branch: string, seq: number, active: number, more: Partial<Listed> = {}): Listed => ({
  work: { repository: "owner/repo", branch },
  path: `/owner/repo/work/${branch}`,
  seq,
  active,
  pictures: 0,
  notes: 0,
  weakened: false,
  standing: "none",
  word: "no run yet",
  waits: [],
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
    expect(onlyViews("shared")).toBe(true);
    expect(maySignIn("shared")).toBe(false);
    expect(maySignIn({ login: null })).toBe(true);
    expect(onlyViews({ login: "writer", role: "write" })).toBe(false);
  });
});
