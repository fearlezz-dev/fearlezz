// Where the run a work page shows stands, in one line: the case is decided once, in one order,
// and every part of the line (its tone, icon, word, detail and the sentence it shortens) comes
// from that one case, so they never disagree.

import type { IconName } from "./icons";
import { picturesOf } from "./pictures";
import type { Review, Run } from "./proof";
import { type Looker, canAccept, canSay, localBy, roleOf, toLookAt } from "./review";
import { short } from "./short";

/** The tone of the line: a local run, running, passed, failed, or waiting for a person. */
export type Tone = "local" | "run" | "ok" | "bad" | "look";

export type Headline = {
  tone: Tone;
  icon: IconName;
  /** The word on the pill. */
  word: string;
  /** The one fact that makes the word specific: what failed, how many wait, how far a run is. */
  detail: string;
  /** The sentence the word shortens, its title. */
  title: string;
};

/** Names in a list as a sentence says them: `a`, `a and b`, `a, b and c`. */
function list(names: string[]): string {
  if (names.length <= 1) return names.join("");
  return `${names.slice(0, -1).join(", ")} and ${names.at(-1)}`;
}

const pictures = (n: number) => `${n} ${n === 1 ? "picture" : "pictures"}`;

/** Where `run` stands for `viewer`. A local run is the agent's word, so it says so first and
 * waits for no one; once a CI run ended, the host's review of its head says what waits. */
export function headline(run: Run, review: Review | undefined, viewer: Looker | undefined): Headline {
  const head = short(run.head);
  const ended = run.checks.filter((c) => c.entry).length;
  const failedNames = run.checks.filter((c) => c.entry?.outcome === "failed").map((c) => c.id);
  const failed = failedNames.length;
  const toLook = picturesOf(run).filter(toLookAt).length;
  const checks = `${failed} ${failed === 1 ? "check" : "checks"}`;
  const by = localBy(run);

  if (by) {
    const what = `Unproven: ${by}'s local run of ${head}`;
    const line = (word: string, detail: string, title: string): Headline => ({ tone: "local", icon: "alert", word, detail, title });
    if (run.state === "running") return line("Running locally", `${ended} of ${run.checks.length} checks`, `${what}, ${ended} of ${run.checks.length} checks`);
    if (run.state === "cancelled") return line("Unproven", "Ended without its proof", `${what} stopped without its proof`);
    if (failed) return line("Unproven", `${list(failedNames)} failed`, `${what}: ${checks} failed`);
    if (toLook) return line("Unproven", `${by}'s local run, ${pictures(toLook)} changed`, `${what}: ${pictures(toLook)} changed`);
    return line("Unproven", `${by}'s local run, every check passed`, `${what}: its checks passed`);
  }

  if (run.state === "running") {
    const running = run.checks.filter((c) => c.started != null && !c.entry).map((c) => c.id);
    const detail = [running.length ? `${list(running)} running` : "Starting", failed ? `${list(failedNames)} failed` : "", toLook ? `${pictures(toLook)} so far` : ""];
    const looking = toLook ? `, ${pictures(toLook)} to look at already` : "";
    return {
      tone: "run",
      icon: "spinner",
      word: `Proving ${ended} of ${run.checks.length}`,
      detail: detail.filter(Boolean).join(", "),
      title: `Proving ${head}: ${ended} of ${run.checks.length} checks${looking}`,
    };
  }
  if (run.state === "cancelled") return { tone: "bad", icon: "stop", word: "Stopped", detail: "Ended without its proof", title: `The run of ${head} ended without its proof` };
  if (failed) return { tone: "bad", icon: "cross", word: `${failed} failed`, detail: list(failedNames), title: `${checks} failed at ${head}` };

  // Once the review of this head failed, needs change, waits or is approved, the host's line for
  // this person is the title.
  const said = review?.title[roleOf(viewer)] ?? "";
  const count = (standing: string) => review?.pictures.filter((p) => p.standing === standing).length ?? 0;
  // What changed since a person looked, said after what waits so it is looked at again.
  const changed = review?.pictures.filter((p) => p.changed_since).length ?? 0;
  const again = changed ? `, ${changed} changed since a look` : "";
  switch (review?.state) {
    case "failed":
      // A weakening that waits for an accept is a decision, not a failure: it reads as one.
      if (review.weakened.length && !review.accept) {
        return { tone: "look", icon: "eye", word: canAccept(viewer) ? "Waits for you" : "Waits for an admin", detail: "Weaker proof waits for an accept", title: said };
      }
      return { tone: "bad", icon: "cross", word: "Failed", detail: "Its proof failed", title: said };
    case "needs_change": {
      const n = count("needs_change");
      return { tone: "bad", icon: "cross", word: "Needs change", detail: `${pictures(n)} ${n === 1 ? "needs" : "need"} change${again}`, title: said };
    }
    case "waits":
      return { tone: "look", icon: "eye", word: canSay(viewer) ? "Waits for you" : "Waits for a look", detail: `${pictures(count("waits"))} to look at${again}`, title: said };
    case "approved": {
      const reviewed = review.pictures.length > 0 || review.weakened.length > 0;
      const detail = review.pictures.length ? "Every picture looks right" : "Every check passed, nothing to review";
      return { tone: "ok", icon: "check", word: reviewed ? "Looks right" : "Proved", detail, title: said };
    }
  }
  if (toLook) return { tone: "look", icon: "eye", word: "Pictures changed", detail: `${pictures(toLook)} changed`, title: `${pictures(toLook)} changed at ${head}` };
  return { tone: "ok", icon: "check", word: "Proved", detail: "Every check passed", title: `Proved ${head}` };
}
