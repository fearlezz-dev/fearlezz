// Generated from crates/fearlezz-proof by ee/host-tests/tests/types.rs; do not edit.
// FEARLEZZ_WRITE_TYPES=1 cargo test --manifest-path ee/Cargo.toml --test types

export type Live = { "kind": "snapshot" } & Snapshot | { "kind": "event", 
/**
 * Its place in the work's events; each is one more than the one before.
 */
seq: number, 
/**
 * The run it changes.
 */
run: string, 
/**
 * What changed.
 */
change: Change, };

export type Snapshot = { 
/**
 * The last event it includes; a page applies only later ones.
 */
seq: number, 
/**
 * The work.
 */
work: Work, 
/**
 * Its runs, newest first.
 */
runs: Array<Run>, };

export type Change = { "kind": "opened", 
/**
 * The work.
 */
work: Work, 
/**
 * The new run.
 */
run: Run, } | { "kind": "started", 
/**
 * Its id.
 */
check: string, 
/**
 * When, in seconds since the epoch.
 */
at: number, } | { "kind": "finished", 
/**
 * Its entry.
 */
check: Check, } | { "kind": "done", 
/**
 * Every check's entry.
 */
checks: Array<Check>, };

export type Work = { 
/**
 * `owner/name` as GitHub spells it.
 */
repository: string, 
/**
 * Its branch.
 */
branch: string, 
/**
 * Its pull request, once a run named one.
 */
pull?: Pull | null, };

export type Pull = { 
/**
 * Its number.
 */
number: number, 
/**
 * Its title, so a person knows which pull request a page belongs to.
 */
title: string, 
/**
 * Its page.
 */
url?: string | null, };

export type Run = { 
/**
 * Its id.
 */
id: string, 
/**
 * The commit it proves.
 */
head: string, 
/**
 * The base branch.
 */
base: string, 
/**
 * Where it runs.
 */
source: Source, 
/**
 * Whether it still runs.
 */
state: RunState, 
/**
 * When it opened, in seconds since the epoch.
 */
opened: number, 
/**
 * Its checks, planned ones first in their order, then any it ran besides.
 */
checks: Array<RunCheck>, };

export type RunState = "running" | "done";

export type Source = { "kind": "ci", 
/**
 * The job's id in its workflow.
 */
job: string, 
/**
 * The workflow run's page.
 */
run_url?: string | null, };

export type RunCheck = { 
/**
 * Its id.
 */
id: string, 
/**
 * When it started, in seconds since the epoch by the host's clock.
 */
started?: number | null, 
/**
 * Its entry in the proof, once it ended.
 */
entry?: Check | null, };

export type Check = { 
/**
 * Its id.
 */
id: string, 
/**
 * The whole command.
 */
run: string, 
/**
 * The stage it belongs to.
 */
stage: string, 
/**
 * How it came out.
 */
outcome: Ran, 
/**
 * How long it ran.
 */
seconds: number, 
/**
 * Its command's exit code, when it ended with one.
 */
exit_code?: number | null, 
/**
 * How many tests ran, for a check that counts them.
 */
tests?: number | null, 
/**
 * Each test case, for a check whose reader names them.
 */
cases?: Array<Case>, 
/**
 * The findings on lines the change touched, worst first.
 */
findings?: Array<Finding>, 
/**
 * How many findings are on lines it did not touch.
 */
findings_elsewhere: number, 
/**
 * Everything it printed: a file in the bundle, relative to it.
 */
output?: string | null, 
/**
 * Why it failed and what to do, for a failed check.
 */
failure?: Item | null, 
/**
 * Why it did not run.
 */
not_run?: string | null, 
/**
 * The same inputs gave both outcomes.
 */
unstable?: boolean, 
/**
 * What red before green showed.
 */
red?: Red | null, };

export type Ran = "passed" | "failed" | "not_run";

export type Case = { 
/**
 * `classname::name`, or the name alone when there is no class.
 */
name: string, 
/**
 * The `file` attribute, which some runners write.
 */
file?: string | null, 
/**
 * It failed.
 */
failed: boolean, };

export type Finding = { 
/**
 * Relative to the worktree root; empty when the tool named no file.
 */
file: string, 
/**
 * The line, or 0 when the tool named none.
 */
line: number, 
/**
 * The last line it spans, when it spans more than one.
 */
end_line?: number | null, 
/**
 * How much it matters.
 */
level: Level, 
/**
 * The rule the tool names it by.
 */
rule: string, 
/**
 * What the tool says.
 */
message: string, 
/**
 * What to do about it.
 */
fix?: string | null, 
/**
 * Why it matters.
 */
why?: string | null, 
/**
 * It counts wherever it is, not only on lines the change touched.
 */
anywhere?: boolean, };

export type Level = "hint" | "flag" | "gate";

export type Item = { 
/**
 * What is wrong.
 */
what: string, 
/**
 * Why.
 */
why: string, 
/**
 * What to do about it.
 */
fix: string, };

export type Red = { 
/**
 * What it comes to.
 */
verdict: Verdict, 
/**
 * The change's test files the check reads.
 */
files?: Array<string>, 
/**
 * The tests that failed on the base and pass here (red), or that pass on both (not red).
 */
tests?: Array<string>, 
/**
 * Why it did not run.
 */
why?: string | null, 
/**
 * The change is a fix this does not prove.
 */
blocks?: boolean, };

export type Verdict = "red" | "unbuilt" | "base_fails" | "not_red" | "new" | "did_not_run";
