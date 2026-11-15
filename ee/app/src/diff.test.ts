import { describe, expect, test } from "bun:test";
import { diff } from "./diff";

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
});
