// The states the host's pages are shown in: one world each, built from a few plain pieces so a
// state reads as what makes it different. The types are the proof model's, so a change to the
// model that a state misses fails `tsc`.

import { readFileSync } from "node:fs";
import type {
  Live,
  Check,
  Comparison,
  Listed,
  Look,
  Marks,
  Note,
  Picture,
  PictureVerdict,
  Review,
  ReviewPicture,
  Run,
  RunCheck,
  Snapshot,
  Source,
  Viewer,
  Weakening,
} from "../src/proof";
import { NOW, type World } from "./host";
import { SIZE, SVG, screen } from "./screens";

const REPOSITORY = "seamlezz/fearlezz";
const HEAD = "8f3c2a1d9e7b4c6a5f0e1d2c3b4a59687f6e5d4c";
const OLD_HEAD = "2b7e91c04d8a3f6e5c1b0a9d8e7f6c5b4a392817";
export const WORK_PATH = `/${REPOSITORY}/pull/61`;
export const SHARE_PATH = "/-/share/k3x9q2";
export const REPOSITORY_PATH = `/${REPOSITORY}`;

export const SIGNED_OUT: Viewer = { login: null };
export const WRITER: Viewer = { login: "mara", role: "write" };
export const ADMIN: Viewer = { login: "jonas", role: "admin" };

const ci = (leg: string, run_id: number, job = leg === "linux" ? "proof" : `proof (${leg})`): Source => ({
  kind: "ci",
  job,
  run_url: `https://github.com/${REPOSITORY}/actions/runs/${run_id}`,
  run_id,
  run_attempt: 1,
  runner_name: "GitHub Actions 12",
  leg,
});

// The checks Fearlezz proves itself with, in the order a run plans them.
const IDS = ["settings", "rules", "commits", "secrets", "fmt", "clippy", "tests", "fmt/ee", "clippy/ee", "tests/ee", "ee-page-types", "ee-page-tests", "ee-page-look", "actionlint", "packaging"];
const SECONDS: Record<string, number> = { tests: 184.2, "tests/ee": 96.4, clippy: 41.7, "clippy/ee": 38.1, "ee-page-look": 22.9 };

function passed(id: string, extra: Partial<Check> = {}): Check {
  return { id, run: id, stage: "working", outcome: "passed", seconds: SECONDS[id] ?? 1.2 + id.length / 10, findings_elsewhere: 0, ...extra };
}

function run(id: string, head: string, state: Run["state"], checks: RunCheck[], source: Source = ci("linux", 1201), opened = NOW - 240): Run {
  return { id, head, base: "main", source, state, opened, checks };
}

/** Every check passed, `changes` replacing some. */
const allPassed = (changes: Record<string, Partial<RunCheck>> = {}): RunCheck[] =>
  IDS.map((id) => ({ id, started: NOW - 200, entry: passed(id), ...changes[id] }));

const work = { repository: REPOSITORY, branch: "feat/profile-save", pull: { number: 61, title: "feat: save the profile in one step", url: `https://github.com/${REPOSITORY}/pull/61` } };

function snapshot(runs: Run[], more: Partial<Snapshot> = {}): Snapshot {
  return { seq: 40, work, runs, ...more };
}

function review(state: Review["state"], title: string, more: Partial<Review> = {}): Review {
  return { head: HEAD, state, pictures: [], weakened: [], title: { read: title, write: title, admin: title }, ...more };
}

const live = (s: Snapshot): Live => ({ kind: "snapshot", ...s });

// The pictures of the look: a changed screen, a new one, a gone one, a changed text and some
// that stayed the same, each in light and dark where it has a theme.
const before = (dark = false) => screen({ accent: "#4b5563", rows: ["Name", "Email"], dark });
const after = (dark = false) => screen({ accent: "#4f46e5", rows: ["Name", "Email", "Time zone"], banner: "Saved. Your team sees the change now.", dark });
const FILES: NonNullable<World["files"]> = {
  "looks/profile/light/before.svg": { type: SVG, body: before() },
  "looks/profile/light/after.svg": { type: SVG, body: after() },
  "looks/profile/dark/before.svg": { type: SVG, body: before(true) },
  "looks/profile/dark/after.svg": { type: SVG, body: after(true) },
  "looks/team/after.svg": { type: SVG, body: screen({ accent: "#4f46e5", rows: ["Team name", "Invite by email", "Role"] }) },
  "looks/billing/before.svg": { type: SVG, body: screen({ accent: "#4b5563", rows: ["Card", "Billing email"] }) },
  "looks/security/light.svg": { type: SVG, body: screen({ accent: "#4b5563", rows: ["Password", "Two factor"] }) },
  "looks/security/dark.svg": { type: SVG, body: screen({ accent: "#4b5563", rows: ["Password", "Two factor"], dark: true }) },
  "looks/checkout/after.webm": { type: "video/webm", body: readFileSync(new URL("./fixtures/checkout.webm", import.meta.url)) },
};

// Real PNGs and VP9 films of the same made up screen, for what the page compares itself (runs
// other than the base), with the CLI's answers for them beside them.
const COMPARE = new URL("../../../crates/fearlezz/tests/fixtures/compare/", import.meta.url);
const fixture = (name: string) => readFileSync(new URL(name, COMPARE));
export const answerOf = (name: string): Comparison => JSON.parse(fixture(name).toString()) as Comparison;
const PNG = "image/png";
const WEBM = "video/webm";

function changedStill(name: string, at: string): Picture {
  return {
    name,
    kind: "still",
    change: "changed",
    summary: "Changed in 3 places.",
    before: `looks/${at}/before.svg`,
    after: `looks/${at}/after.svg`,
    comparison: {
      verdict: "changed",
      still: true,
      size_before: SIZE,
      size_after: SIZE,
      total_before: 0,
      total_after: 0,
      max_shift: 0,
      max_shift_at: null,
      states_before: 1,
      states_after: 1,
      // Each box is its left, top, right and bottom edge in pixels.
      moments: [
        { before: [0, 0], after: [0, 0], box: [312, 96, 968, 168], px: 21_000 },
        { before: [0, 0], after: [0, 0], box: [352, 444, 928, 518], px: 9_400 },
        { before: [0, 0], after: [0, 0], box: [352, 554, 500, 612], px: 5_544 },
      ],
      pairs: [],
    },
  };
}

const same = (name: string, file: string): Picture => ({ name, kind: "still", change: "same", summary: "The same as before.", before: file, after: file, comparison: null });

const LOOK: Look = {
  check: "ee-page-look",
  base_commit: OLD_HEAD,
  base_result: "passed",
  items: [
    changedStill("profile/desktop/light.png", "profile/light"),
    changedStill("profile/desktop/dark.png", "profile/dark"),
    changedStill("profile/mobile/light.png", "profile/light"),
    changedStill("profile/mobile/dark.png", "profile/dark"),
    { name: "team/desktop/light.png", kind: "still", change: "new", summary: "New: the base took no such picture.", before: null, after: "looks/team/after.svg", comparison: null },
    { name: "billing/desktop/light.png", kind: "still", change: "gone", summary: "Gone: this run took no such picture.", before: "looks/billing/before.svg", after: null, comparison: null },
    {
      name: "manifests/deployment.yaml",
      kind: "text",
      change: "changed",
      summary: "2 lines changed.",
      before: null,
      after: null,
      comparison: null,
      diff: [
        { kind: "fold", count: 12 },
        { kind: "same", text: "    spec:" },
        { kind: "same", text: "      containers:" },
        { kind: "same", text: "        - name: harbor" },
        { kind: "was", text: "          image: ghcr.io/seamlezz/harbor:1.8.2" },
        { kind: "now", text: "          image: ghcr.io/seamlezz/harbor:1.9.0" },
        { kind: "same", text: "          resources:" },
        { kind: "was", text: "            limits: { memory: 256Mi }" },
        { kind: "now", text: "            limits: { memory: 384Mi }" },
        { kind: "fold", count: 30 },
      ],
    },
    same("security/desktop/light.png", "looks/security/light.svg"),
    same("security/desktop/dark.png", "looks/security/dark.svg"),
  ],
};

// The same check when it failed on the base commit: no before, so every picture is new.
const FIRST_LOOK: Look = {
  ...LOOK,
  base_result: "failed",
  no_before:
    "the check fails on the base commit (log: /home/runner/work/harbor/harbor/.git/fearlezz/results/f7a951ab440ebfea4a41768fed8e9c0983eb912bd880701f366b90ff10e1f6db/000001795130100275407649-4489/output.log)",
  items: [
    ...LOOK.items.filter((p) => p.kind !== "text" && p.after).map((p) => ({ ...p, change: "new" as const, before: null, comparison: null, summary: "New: the base took no such picture." })),
    { name: "checkout/desktop.webm", kind: "film", change: "new", summary: "New: the base took no such film.", before: null, after: "looks/checkout/after.webm", comparison: null },
    { name: "checkout/mobile.webm", kind: "film", change: "new", summary: "New: the base took no such film.", before: null, after: "looks/checkout/after.webm", comparison: null },
    { name: "profile/desktop/light.webm", kind: "film", change: "new", summary: "New: the base took no such film.", before: null, after: "looks/checkout/after.webm", comparison: null },
    { name: "profile/desktop/dark.webm", kind: "film", change: "new", summary: "New: the base took no such film.", before: null, after: "looks/checkout/after.webm", comparison: null },
    { name: "profile/mobile/light.webm", kind: "film", change: "new", summary: "New: the base took no such film.", before: null, after: "looks/checkout/after.webm", comparison: null },
    { name: "profile/mobile/dark.webm", kind: "film", change: "new", summary: "New: the base took no such film.", before: null, after: "looks/checkout/after.webm", comparison: null },
  ],
};

const KEYS = Object.fromEntries(LOOK.items.map((p, i) => [p.name, `key-${i}`]));
// The first look's pictures, its films among them, each with what it shows.
const FIRST_KEYS = Object.fromEntries(FIRST_LOOK.items.map((p, i) => [p.name, KEYS[p.name] ?? `first-key-${i}`]));
const looked = (checks: RunCheck[]) => checks.map((c) => (c.id === "ee-page-look" ? { ...c, look: LOOK, keys: KEYS } : c));

const waitingPictures = (standing: ReviewPicture["standing"] = "waits"): ReviewPicture[] =>
  LOOK.items.filter((p) => p.change !== "same").map((p) => ({ check: "ee-page-look", name: p.name, key: KEYS[p.name]!, standing }));

const verdictOn = (name: string): PictureVerdict => ({ check: "ee-page-look", picture: name, key: KEYS[name]!, by: "mara", at: NOW - 120, run: "r2", head: HEAD });

const OLD_KEYS = Object.fromEntries(Object.keys(KEYS).map((name, i) => [name, `old-${i}`]));
const NOTE: Note = {
  id: "n1",
  check: "ee-page-look",
  picture: "profile/desktop/light.png",
  run: "r2",
  head: HEAD,
  side: "after",
  region: { x: 0.25, y: 0.13, w: 0.5, h: 0.07 },
  words: "The banner pushes the card down; keep the card where it was and show the banner as a toast.",
  by: "mara",
  at: NOW - 600,
  state: "open",
};

/** The note and a verdict given on the profile at the earlier push, before it changed. */
const AGAIN_NOTE: Note = { ...NOTE, run: "r1", head: OLD_HEAD, at: NOW - 5000 };
const AGAIN_VERDICT: PictureVerdict = { ...verdictOn("profile/desktop/dark.png"), key: OLD_KEYS["profile/desktop/dark.png"]!, run: "r1", head: OLD_HEAD, at: NOW - 5100 };
// The profile's film changed too since mara said it looks right, and a screen's films come first,
// so the page opens on it.
const AGAIN_FILM = "profile/desktop/light.webm";
const AGAIN_LOOK: Look = {
  ...LOOK,
  items: [
    ...LOOK.items,
    {
      name: AGAIN_FILM,
      kind: "film",
      change: "changed",
      summary: "Changed in 1 state.",
      before: "looks/checkout/after.webm",
      after: "looks/checkout/after.webm",
      comparison: {
        verdict: "changed",
        still: false,
        size_before: [640, 400],
        size_after: [640, 400],
        total_before: 2,
        total_after: 2,
        max_shift: 0,
        max_shift_at: null,
        states_before: 2,
        states_after: 2,
        moments: [{ before: [1, 2], after: [1, 2], box: [160, 120, 480, 220], px: 9_000 }],
        pairs: [
          { before: 0, after: 0, before_frame: 0, after_frame: 0, before_at: 0, after_at: 0, px: 0, boxes: [] },
          { before: 1, after: 1, before_frame: 30, after_frame: 30, before_at: 1, after_at: 1, px: 9_000, boxes: [[160, 120, 480, 220]] },
        ],
      },
    },
  ],
};
const AGAIN_FILM_VERDICT: PictureVerdict = { ...AGAIN_VERDICT, picture: AGAIN_FILM, key: "old-film" };
const lookedAgain = (keys: Record<string, string>) => (checks: RunCheck[]) =>
  checks.map((c) => (c.id === "ee-page-look" ? { ...c, look: AGAIN_LOOK, keys } : c));

const WEAKENED: Weakening[] = [
  {
    check: "tests",
    hows: ["it runs another command", "it no longer runs on macos"],
    was: '[check.tests]\nrun = "cargo test --workspace"\non = ["linux", "macos"]\n',
    now: '[check.tests]\nrun = "cargo test -p fearlezz"\non = ["linux"]\n',
  },
  { setting: "no_proof", hows: ["it newly says docs/** needs no proof"], was: 'no_proof = ["*.md"]\n', now: 'no_proof = ["*.md", "docs/**"]\n' },
];

const earlierRun = run("r0", OLD_HEAD, "done", allPassed({ tests: { entry: passed("tests", { outcome: "failed", exit_code: 101 }) } }), ci("linux", 1188), NOW - 5400);

/** The work page's states, by name: the still's screen is `work/<name>`. */
export const WORK: Record<string, { path?: string; world: World }> = {
  connecting: { world: {} },
  gone: { world: { gone: "This work is not here." } },
  "no-runs": { world: { live: live(snapshot([])), viewer: WRITER } },
  running: {
    world: {
      viewer: SIGNED_OUT,
      live: live(
        snapshot(
          [
            run(
              "r2",
              HEAD,
              "running",
              IDS.map((id, i): RunCheck => {
                if (i < 7) return { id, started: NOW - 200, entry: passed(id) };
                if (i < 9) return { id, started: NOW - 34 + i };
                return { id };
              }),
              ci("linux", 1201),
              NOW - 210,
            ),
            earlierRun,
          ],
          { waiting: [{ name: "proof (macos)", head: HEAD, since: NOW - 90 }], review: review("running", `CI is proving 8f3c2a1`) },
        ),
      ),
    },
  },
  proved: {
    world: {
      viewer: WRITER,
      live: live(
        snapshot(
          [
            run(
              "r2",
              HEAD,
              "done",
              allPassed({
                tests: {
                  entry: passed("tests", {
                    cases: [{ name: "queue::waits_behind_the_slot_holder", file: "crates/fearlezz/tests/queue.rs", failed: false, flaky: true }],
                  }),
                },
              }),
            ),
            earlierRun,
          ],
          { review: review("approved", "Proven at 8f3c2a1, with nothing to review") },
        ),
      ),
    },
  },
  failed: {
    world: {
      viewer: WRITER,
      live: live(
        snapshot(
          [
            run(
              "r2",
              HEAD,
              "done",
              allPassed({
                tests: {
                  entry: passed("tests", {
                    outcome: "failed",
                    exit_code: 101,
                    tests: 412,
                    cases: [
                      { name: "prove::reuses_a_pass_from_another_worktree", file: "crates/fearlezz/tests/prove.rs", failed: true },
                      { name: "look::a_gone_picture_still_shows", file: "crates/fearlezz/tests/look.rs", failed: true },
                      { name: "queue::waits_behind_the_slot_holder", file: "crates/fearlezz/tests/queue.rs", failed: true, flaky: true },
                    ],
                    failure: { what: "check `tests` failed", why: "its command ended with exit code 101", fix: "fix what it reports; the check page shows the end of its output" },
                    output: [
                      "running 412 tests",
                      "test look::a_changed_still_is_compared ... ok",
                      "test prove::reuses_a_pass_from_another_worktree ... FAILED",
                      "test look::a_gone_picture_still_shows ... FAILED",
                      "",
                      "---- prove::reuses_a_pass_from_another_worktree stdout ----",
                      "thread 'prove::reuses_a_pass_from_another_worktree' panicked at crates/fearlezz/tests/prove.rs:88:5:",
                      "assertion `left == right` failed: the second worktree ran the check again",
                      "  left: 2",
                      " right: 1",
                      "",
                      "test result: FAILED. 409 passed; 3 failed; 0 ignored; finished in 184.20s",
                    ].join("\n"),
                  }),
                },
                clippy: {
                  entry: passed("clippy", {
                    outcome: "failed",
                    exit_code: 101,
                    findings: [
                      { file: "crates/fearlezz/src/prove.rs", line: 212, level: "gate", rule: "clippy::redundant_clone", message: "redundant clone" },
                      { file: "crates/fearlezz/src/prove.rs", line: 240, level: "flag", rule: "clippy::needless_pass_by_value", message: "this argument is passed by value, but not consumed in the function body" },
                    ],
                    failure: {
                      what: "check `clippy` failed",
                      why: "blocking findings, such as crates/fearlezz/src/prove.rs:212 clippy::redundant_clone",
                      fix: "fix the findings; the check page lists them",
                    },
                  }),
                },
              }),
            ),
          ],
          { review: review("failed", "The proof of 8f3c2a1 failed") },
        ),
      ),
    },
  },
  "waits-for-you": {
    world: {
      viewer: WRITER,
      files: FILES,
      live: live(
        // Proved in two legs: the macOS leg opened last and has no pictures, so the page must
        // open on the Linux leg, whose pictures wait.
        snapshot([run("r3", HEAD, "done", [{ id: "tests/macos", started: NOW - 200, entry: passed("tests/macos") }], ci("macos", 1201), NOW - 230), run("r2", HEAD, "done", looked(allPassed())), earlierRun], {
          review: review("waits", "7 pictures wait for your look at 8f3c2a1", { pictures: waitingPictures(), preview: "https://pr-61.harbor-preview.dev" }),
        }),
      ),
    },
  },
  "needs-change": {
    world: {
      viewer: WRITER,
      files: FILES,
      live: live(
        snapshot([run("r2", HEAD, "done", looked(allPassed()))], {
          review: review("needs_change", "1 picture needs change at 8f3c2a1", {
            pictures: waitingPictures().map((p) => ({
              ...p,
              standing: p.name === NOTE.picture ? "needs_change" : ["profile/desktop/dark.png", "team/desktop/light.png"].includes(p.name) ? "looks_right" : "waits",
            })),
            notes: [NOTE],
          }),
          notes: [NOTE],
          verdicts: [verdictOn("profile/desktop/dark.png"), verdictOn("team/desktop/light.png")],
        }),
      ),
    },
  },
  // Pushed again after a review: the profile changed since mara said its dark theme looks right
  // and since the note on its light theme, so both say to look again.
  "look-again": {
    world: {
      viewer: WRITER,
      files: FILES,
      // The page compares what the dark theme and the film show now with what they showed when
      // mara said they look right, so those are real pictures, which it can decode.
      runFiles: {
        r2: { ...FILES, "looks/profile/dark/before.svg": { type: PNG, body: fixture("base.png") }, "looks/profile/dark/after.svg": { type: PNG, body: fixture("now.png") } },
        r1: { ...FILES, "looks/profile/dark/after.svg": { type: PNG, body: fixture("look.png") }, "looks/checkout/after.webm": { type: WEBM, body: fixture("look.webm") } },
      },
      live: live(
        snapshot(
          [
            run("r2", HEAD, "done", lookedAgain({ ...KEYS, [AGAIN_FILM]: "key-film" })(allPassed())),
            run("r1", OLD_HEAD, "done", lookedAgain({ ...OLD_KEYS, [AGAIN_FILM]: "old-film" })(allPassed()), ci("linux", 1188), NOW - 5400),
          ],
          {
          review: review("needs_change", "1 picture needs change at 8f3c2a1", {
            pictures: [...waitingPictures(), { check: "ee-page-look", name: AGAIN_FILM, key: "key-film", standing: "waits" as const, changed_since: { verdict: AGAIN_FILM_VERDICT } }].map((p) =>
              p.name === NOTE.picture
                ? { ...p, standing: "needs_change", changed_since: { notes: [AGAIN_NOTE.id] } }
                : p.name === "profile/desktop/dark.png"
                  ? { ...p, changed_since: { verdict: AGAIN_VERDICT } }
                  : p.name === "team/desktop/light.png"
                    ? { ...p, standing: "looks_right" }
                    : p,
            ),
            notes: [AGAIN_NOTE],
          }),
          notes: [AGAIN_NOTE],
          verdicts: [AGAIN_VERDICT, AGAIN_FILM_VERDICT, verdictOn("team/desktop/light.png")],
        },
        ),
      ),
    },
  },
  "weaker-proof": {
    world: {
      viewer: ADMIN,
      live: live(
        snapshot([run("r2", HEAD, "done", allPassed())], {
          review: {
            ...review("failed", "", { weakened: WEAKENED }),
            title: {
              read: "The proof of 8f3c2a1 failed: what it weakens waits for an admin's accept",
              write: "The proof of 8f3c2a1 failed: what it weakens waits for an admin's accept",
              admin: "The proof of 8f3c2a1 failed: what it weakens waits for your accept",
            },
          },
        }),
      ),
    },
  },
  local: {
    world: {
      viewer: WRITER,
      files: FILES,
      live: live(snapshot([run("l1", HEAD, "done", looked(allPassed()), { kind: "local", login: "claude" }, NOW - 60)])),
    },
  },
  // The check failed on the base, so every picture is new; proved in legs whose job, as on
  // GitHub, is the same name for each.
  "no-before": {
    world: {
      viewer: WRITER,
      files: FILES,
      live: live(
        snapshot(
          [
            run("r3", HEAD, "done", [{ id: "tests/macos", started: NOW - 200, entry: passed("tests/macos") }], ci("macos", 1201, "leg"), NOW - 230),
            run(
              "r2",
              HEAD,
              "done",
              allPassed().map((c) => (c.id === "ee-page-look" ? { ...c, look: FIRST_LOOK, keys: FIRST_KEYS } : c)),
              ci("linux", 1201, "leg"),
            ),
          ],
          {
            review: review("waits", "7 pictures wait for your look at 8f3c2a1", {
              pictures: FIRST_LOOK.items.map((p) => ({ check: "ee-page-look", name: p.name, key: FIRST_KEYS[p.name]!, standing: "waits" })),
            }),
          },
        ),
      ),
    },
  },
  shared: {
    path: SHARE_PATH,
    world: {
      files: FILES,
      live: live(
        snapshot([run("r2", HEAD, "done", looked(allPassed()))], {
          review: review("waits", "7 pictures wait for a look at 8f3c2a1", { pictures: waitingPictures() }),
        }),
      ),
    },
  },
};

const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`;

/**
 * A listed work with its standing, word and waits as `Listed::worded`
 * (crates/fearlezz-proof/src/events.rs) derives them from its newest run and counts, so the
 * stills show only words the host sends.
 */
function listed(n: number, branch: string, title: string | null, more: Partial<Listed> = {}): Listed {
  const work: Listed = {
    work: { repository: REPOSITORY, branch, pull: title ? { number: n, title } : null },
    path: title ? `/${REPOSITORY}/pull/${n}` : `/${REPOSITORY}/tree/${branch}`,
    seq: 10 + n,
    active: NOW - n * 1800,
    latest: { head: HEAD, source: ci("linux", 1200 + n), state: "done", failed: 0 },
    pictures: 0,
    notes: 0,
    weakened: false,
    standing: "none",
    word: "",
    waits: [],
    ...more,
  };
  const waits = [
    ...(work.pictures > 0 ? [`${plural(work.pictures, "picture")} to look at`] : []),
    ...(work.notes > 0 ? [plural(work.notes, "open note")] : []),
    ...(work.weakened ? ["a weakening to accept"] : []),
  ];
  const l = work.latest;
  const standing: Listed["standing"] = !l
    ? "none"
    : l.state === "running"
      ? "running"
      : l.state === "cancelled"
        ? "cancelled"
        : l.failed > 0
          ? "failed"
          : waits.length
            ? "waits"
            : "passed";
  const word = { none: "no run yet", running: "running", passed: "passed", waits: "waits for a person", failed: "failed", cancelled: "stopped" }[standing];
  return { ...work, standing, word, waits };
}

/** The repository page's states: the still's screen is `repository/<name>`. */
export const REPOSITORY_STATES: Record<string, World> = {
  works: {
    live: {
      kind: "snapshot",
      repository: REPOSITORY,
      works: [
        listed(1, "feat/profile-save", "feat: save the profile in one step", { pictures: 5 }),
        listed(2, "fix/queue-slot", "fix: a waiter names the check it waits behind", {
          latest: { head: HEAD, source: ci("linux", 1202), state: "running", failed: 1 },
        }),
        listed(3, "feat/tests-on-one-leg", "feat: tests run on one leg", {
          weakened: true,
          latest: { head: HEAD, source: ci("linux", 1203), state: "done", failed: 2 },
        }),
        listed(4, "spike/svg-thumbs", null, { latest: { head: HEAD, source: { kind: "local", login: "claude" }, state: "done", failed: 0 } }),
        listed(5, "chore/bump-wrangler", "chore: bump wrangler"),
        listed(6, "feat/notes-on-films", "feat: notes on a moment of a film", { notes: 1 }),
        listed(7, "docs/roadmap", "docs: what comes next", {
          latest: { head: HEAD, source: ci("linux", 1207), state: "cancelled", failed: 0 },
        }),
      ],
    },
  },
  empty: { live: { kind: "snapshot", repository: REPOSITORY, works: [] } },
};

// The strip of runs: a screen pushed three times, each run showing it differently. mara said
// the second push looks right, an agent's local run showed the same, and the newest push
// changed it again, so its still and its film open compared with that look. The pictures are
// real PNGs and VP9 films, since the page compares runs other than the base itself; the CLI's
// answers for the same files are beside them (crates/fearlezz/tests/fixtures/compare).
const FIRST_HEAD = "5d1c0e9a7b3f2e4d6c8a0b1f3e5d7c9a2b4f6e81";
const STRIP_STILL = "profile/desktop/light.png";
const STRIP_FILM = "checkout/desktop.webm";
/** What the strip's films say happened in them: a load whose timing may change, then two steps. */
const STRIP_MARKS: Marks = {
  marks: [
    { name: "opened", at: 0.3 },
    { name: "paid", at: 0.8 },
  ],
  spans: [{ name: "loading", start: 0, end: 0.3, timing: "free" }],
};
const STRIP_LOOK: Look = {
  check: "ee-page-look",
  base_commit: OLD_HEAD,
  base_result: "passed",
  items: [
    { name: STRIP_STILL, kind: "still", change: "changed", summary: "Changed in 1 place.", before: "looks/profile/before.png", after: "looks/profile/after.png", comparison: answerOf("base-still.json") },
    { name: STRIP_FILM, kind: "film", change: "changed", summary: "Changed in 1 state.", before: "looks/checkout/before.webm", after: "looks/checkout/after.webm", comparison: answerOf("base-film.json"), marks: STRIP_MARKS, before_marks: STRIP_MARKS },
  ],
};
/** What a run of the strip showed: `still` and `film` as the files it took. */
const stripRun = (id: string, head: string, shows: "base" | "look" | "now", source: Source, opened: number, key: string = shows): Run =>
  run(
    id,
    head,
    "done",
    allPassed().map((c) =>
      c.id === "ee-page-look"
        ? {
            ...c,
            look: { ...STRIP_LOOK, items: STRIP_LOOK.items.map((p) => ({ ...p, change: shows === "base" ? ("same" as const) : p.change })) },
            keys: { [STRIP_STILL]: `still-${key}`, [STRIP_FILM]: `film-${key}` },
          }
        : c,
    ),
    source,
    opened,
  );
/** A run's files: the base's before and what it showed as its after. */
const stripFiles = (shows: "base" | "look" | "now"): NonNullable<World["files"]> => ({
  "looks/profile/before.png": { type: PNG, body: fixture("base.png") },
  "looks/profile/after.png": { type: PNG, body: fixture(`${shows}.png`) },
  "looks/checkout/before.webm": { type: WEBM, body: fixture("base.webm") },
  "looks/checkout/after.webm": { type: WEBM, body: fixture(`${shows}.webm`) },
});
const lookedAt = (picture: string, run: string, head: string, at: number): PictureVerdict => ({
  check: "ee-page-look",
  picture,
  key: picture === STRIP_STILL ? "still-look" : "film-look",
  by: "mara",
  at,
  run,
  head,
});
export const STRIP: { world: World; still: string; film: string } = {
  still: `ee-page-look/${STRIP_STILL}`,
  film: `ee-page-look/${STRIP_FILM}`,
  world: {
    viewer: WRITER,
    runFiles: { r3: stripFiles("now"), l2: stripFiles("look"), r2: stripFiles("look"), r1: stripFiles("base") },
    live: live(
      snapshot(
        [
          stripRun("r3", HEAD, "now", ci("linux", 1201), NOW - 240),
          stripRun("l2", OLD_HEAD, "look", { kind: "local", login: "claude" }, NOW - 3000),
          stripRun("r2", OLD_HEAD, "look", ci("linux", 1188), NOW - 5400),
          stripRun("r1", FIRST_HEAD, "base", ci("linux", 1150), NOW - 9000),
        ],
        {
          review: review("waits", "2 pictures wait for your look at 8f3c2a1", {
            pictures: [STRIP_STILL, STRIP_FILM].map((name) => ({
              check: "ee-page-look",
              name,
              key: name === STRIP_STILL ? "still-now" : "film-now",
              standing: "waits" as const,
              changed_since: { verdict: lookedAt(name, "r2", OLD_HEAD, NOW - 5000) },
            })),
          }),
          verdicts: [lookedAt(STRIP_STILL, "r2", OLD_HEAD, NOW - 5000), lookedAt(STRIP_FILM, "r2", OLD_HEAD, NOW - 5000)],
        },
      ),
    ),
  },
};

// A long history: the same screen pushed often, each run showing it a little differently (each
// its own content), around the look and the newest push of STRIP. Most of it folds.
const stripLive = STRIP.world.live as Live & Snapshot;
const [stripNow, ...stripEarlier] = stripLive.runs;
const LONG_RUNS = [
  ...["y3", "y2", "y1"].map((id, k) => stripRun(id, OLD_HEAD, "look", ci("linux", 1199 - k), NOW - 600 - k * 600, id)),
  ...stripEarlier.slice(0, 2),
  ...["x4", "x3", "x2", "x1"].map((id, k) => stripRun(id, FIRST_HEAD, "base", ci("linux", 1180 - k), NOW - 6000 - k * 600, id)),
  ...stripEarlier.slice(2),
];
export const LONG_STRIP: World = {
  ...STRIP.world,
  runFiles: { ...STRIP.world.runFiles, ...Object.fromEntries(LONG_RUNS.filter((r) => /^[xy]/.test(r.id)).map((r) => [r.id, stripFiles(r.id.startsWith("x") ? "base" : "look")])) },
  live: live({ ...stripLive, runs: [stripNow!, ...LONG_RUNS] }),
};

// The strip's newest film, its step reached later than before: the before film pays at 0.6 s
// and the after film at 1.2 s, so the comparison aligned them there. Before the mark the before
// film has no footage for 0.6 s, after it the after film for 0.6 s.
const ALIGNED_TIMELINE = [
  { at: 0, length: 1.2, before: { from: 0, to: 0.6, delay: 0.6 }, after: { from: 0, to: 1.2, delay: 0 } },
  { at: 1.2, length: 1.4, before: { from: 0.6, to: 2, delay: 0 }, after: { from: 1.2, to: 2, delay: 0 } },
];
const alignedRun = (r: Run): Run => ({
  ...r,
  checks: r.checks.map((c) =>
    c.look
      ? {
          ...c,
          look: {
            ...c.look,
            items: c.look.items.map((p) =>
              p.name === STRIP_FILM
                ? {
                    ...p,
                    before_marks: { marks: [{ name: "paid", at: 0.6 }] },
                    marks: { marks: [{ name: "paid", at: 1.2 }] },
                    comparison: { ...p.comparison!, timeline: ALIGNED_TIMELINE },
                  }
                : p,
            ),
          },
        }
      : c,
  ),
});
export const ALIGNED: World = { ...STRIP.world, live: live({ ...stripLive, runs: [alignedRun(stripNow!), ...stripEarlier] }) };
