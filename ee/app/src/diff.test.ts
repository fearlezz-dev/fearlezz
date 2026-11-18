import { describe, expect, test } from "bun:test";
import { type Fold, type Line, diff, folded, sent } from "./diff";

/** A line as a person reads it: its sign, and the words that differ in brackets. */
const show = (was: string | undefined, now: string | undefined) =>
  diff(was, now).map((l) => {
    const sign = l.kind === "same" ? " " : l.kind === "was" ? "-" : "+";
    return sign + l.pieces.map((p) => (p.differs ? `[${p.text}]` : p.text)).join("");
  });

describe("diff", () => {
  test("a replaced line marks only the words that differ", () => {
    expect(
      show(
        '[check.ee-clippy]\nrun = "cargo clippy --all-targets --quiet"',
        '[check.ee-clippy]\nrun = "cargo clippy --all-targets --all-features --quiet"',
      ),
    ).toEqual([
      " [check.ee-clippy]",
      '-run = "cargo clippy --all-targets --quiet"',
      '+run = "cargo clippy --all-targets [--all-features ]--quiet"',
    ]);
  });

  test("a quote next to a word never marks the word", () => {
    expect(show('run = "cargo test"', 'run = "cargo test --lib"')).toEqual([
      '-run = "cargo test"',
      '+run = "cargo test[ --lib]"',
    ]);
  });

  test("a line taken out or put in shows whole, unmarked", () => {
    expect(show('[check.tests]\nrun = "cargo test"\ntests = "junit"', '[check.tests]\nrun = "cargo test"')).toEqual([
      " [check.tests]",
      ' run = "cargo test"',
      '-tests = "junit"',
    ]);
    expect(show("a\nc", "a\nb\nc")).toEqual([" a", "+b", " c"]);
  });

  test("a part that is gone or new has only one side", () => {
    expect(show('[check.tests]\nrun = "cargo test"', undefined)).toEqual(["-[check.tests]", '-run = "cargo test"']);
    expect(show(undefined, 'no_proof = ["docs/**"]')).toEqual(['+no_proof = ["docs/**"]']);
  });

  test("a large text with one change diffs quickly", () => {
    const many = (n: number, at: string) => Array.from({ length: n }, (_, i) => (i === 10000 ? at : `line ${i}`)).join("\n");
    const started = performance.now();
    const lines = diff(many(20000, "color: red"), many(20000, "color: blue"));
    expect(performance.now() - started).toBeLessThan(2000);
    expect(lines.filter((l) => l.kind !== "same").length).toBe(2);
  });

  test("two large texts that share nothing still diff, as one replaced block", () => {
    const text = (tag: string) => Array.from({ length: 5000 }, (_, i) => `${tag} ${i}`).join("\n");
    const lines = diff(text("a"), text("b"));
    expect(lines.filter((l) => l.kind === "was").length).toBe(5000);
    expect(lines.filter((l) => l.kind === "now").length).toBe(5000);
  });
});

describe("folded", () => {
  test("long runs of kept lines fold to a count, three lines of context kept", () => {
    const was = Array.from({ length: 20 }, (_, i) => `line ${i}`);
    const now = [...was];
    now[10] = "changed";
    const shown = folded(diff(was.join("\n"), now.join("\n")), 3).map((l) => ("count" in l ? `(${l.count})` : l.kind));
    expect(shown).toEqual(["(7)", "same", "same", "same", "was", "now", "same", "same", "same", "(6)"]);
  });
});

describe("a diff the bundle carries", () => {
  const read = (lines: (Line | Fold)[]) =>
    lines.map((l) =>
      "kind" in l
        ? (l.kind === "same" ? " " : l.kind === "was" ? "-" : "+") + l.pieces.map((p) => (p.differs ? `[${p.text}]` : p.text)).join("")
        : `(${l.count})`,
    );

  test("shows as the CLI folded it, with the words that differ marked", () => {
    expect(
      read(
        sent([
          { kind: "fold", count: 4 },
          { kind: "same", text: "kind: ConfigMap" },
          { kind: "was", text: "color: red" },
          { kind: "now", text: "color: blue" },
          { kind: "now", text: "size: 2" },
          { kind: "same", text: "name: app" },
        ]),
      ),
    ).toEqual(["(4)", " kind: ConfigMap", "-color: [red]", "+color: [blue]", "+size: 2", " name: app"]);
  });

  test("a line put in before one taken out starts a new run", () => {
    expect(read(sent([{ kind: "now", text: "a" }, { kind: "was", text: "b" }]))).toEqual(["+a", "-b"]);
  });
});
