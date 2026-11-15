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
change: Change, } | { "kind": "work", 
/**
 * Its place in the work's events.
 */
seq: number, 
/**
 * What changed.
 */
change: WorkChange, };

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
runs: Array<Run>, 
/**
 * Where review of its newest head stands.
 */
review?: Review | null, 
/**
 * Every verdict given on its pictures.
 */
verdicts?: Array<PictureVerdict>, 
/**
 * Its CI jobs that wait for a runner.
 */
waiting?: Array<Job>, 
/**
 * Its open notes, oldest first, on whichever run they were made.
 */
notes?: Array<Note>, };

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
check: Check, 
/**
 * Its pictures compared with the base commit, when it takes pictures.
 */
look?: Look | null, 
/**
 * What each picture of the look shows, as a verdict follows it.
 */
keys?: { [key in string]: string }, } | { "kind": "done", 
/**
 * Every check's entry.
 */
checks: Array<Check>, 
/**
 * Every look, each of the check named by its `check`.
 */
looks: Array<Look>, 
/**
 * What each picture of each look shows, by check.
 */
keys?: { [key in string]: { [key in string]: string } }, } | { "kind": "cancelled", };

export type WorkChange = { "kind": "verdict", 
/**
 * The verdict.
 */
verdict: PictureVerdict, } | { "kind": "review", 
/**
 * The review, none while the work has no head.
 */
review?: Review | null, } | { "kind": "note", 
/**
 * The note as it is now.
 */
note: Note, } | { "kind": "waiting", 
/**
 * Every one.
 */
waiting: Array<Job>, };

export type Review = { 
/**
 * The commit it is of.
 */
head: string, 
/**
 * Where it stands.
 */
state: ReviewState, 
/**
 * Every picture to look at, in the order of its runs and looks.
 */
pictures: Array<ReviewPicture>, 
/**
 * What the head weakens, waiting for an admin's accept.
 */
weakened: Array<Weakening>, 
/**
 * The accept of those weakenings, once an admin gave it.
 */
accept?: Accept | null, 
/**
 * The work's open notes, oldest first: they follow the work, not the head.
 */
notes?: Array<Note>, };

export type ReviewState = "ci" | "running" | "failed" | "cancelled" | "needs_change" | "waits" | "approved";

export type ReviewPicture = { 
/**
 * The check that took it.
 */
check: string, 
/**
 * Its name in its look.
 */
name: string, 
/**
 * What it shows, as [`key`] names it.
 */
key: string, 
/**
 * The verdict on it, when someone gave one on this content.
 */
verdict?: PictureVerdict | null, };

export type PictureVerdict = { 
/**
 * The check that took the picture.
 */
check: string, 
/**
 * The picture's name in its look.
 */
picture: string, 
/**
 * What the picture showed, as [`key`] names it.
 */
key: string, 
/**
 * What they said.
 */
kind: VerdictKind, 
/**
 * Their GitHub login.
 */
by: string, 
/**
 * When, in seconds since the epoch.
 */
at: number, 
/**
 * The run they gave it on.
 */
run: string, 
/**
 * The commit that run proved.
 */
head: string, };

export type VerdictKind = "looks_right" | "needs_change";

export type Accept = { 
/**
 * The commit it accepts the weakenings of.
 */
head: string, 
/**
 * Their GitHub login.
 */
by: string, 
/**
 * When, in seconds since the epoch.
 */
at: number, };

export type Note = { 
/**
 * Its id on the host.
 */
id: string, 
/**
 * The check that took the picture.
 */
check: string, 
/**
 * The picture's name in its look.
 */
picture: string, 
/**
 * The CI run it was made on, whose pictures it describes.
 */
run: string, 
/**
 * The commit that run proved.
 */
head: string, 
/**
 * Which picture of the pair it marks.
 */
side: NoteSide, 
/**
 * Where on the picture.
 */
region: Region, 
/**
 * For a film, the moment it marks, in seconds from its start.
 */
time?: number | null, 
/**
 * What should change there.
 */
words: string, 
/**
 * The GitHub login of who made it.
 */
by: string, 
/**
 * When, in seconds since the epoch.
 */
at: number, 
/**
 * Whether it still asks for a change.
 */
state: NoteState, 
/**
 * Who resolved it, once someone did.
 */
resolved_by?: string | null, };

export type NoteSide = "before" | "after" | "both";

export type Region = { 
/**
 * Its left edge.
 */
x: number, 
/**
 * Its top edge.
 */
y: number, 
/**
 * Its width.
 */
w: number, 
/**
 * Its height.
 */
h: number, };

export type NoteState = "open" | "resolved";

export type Weakening = { 
/**
 * How it weakened, such as `it runs another command`.
 */
how: string, } & ({ "check": string } | { "setting": string });

export type Job = { 
/**
 * Its name, as its workflow names it.
 */
name: string, 
/**
 * The commit it will prove.
 */
head: string, 
/**
 * Its page on GitHub.
 */
url?: string | null, 
/**
 * Since when it waits, in seconds since the epoch.
 */
since: number, };

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

export type RunState = "running" | "done" | "cancelled";

export type Source = { "kind": "ci", 
/**
 * The job's id in its workflow.
 */
job: string, 
/**
 * The workflow run's page.
 */
run_url?: string | null, } | { "kind": "local", };

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
entry?: Check | null, 
/**
 * Its pictures compared with the base commit, once compared.
 */
look?: Look | null, 
/**
 * What each picture of its look shows, by name, as a verdict follows it.
 */
keys?: { [key in string]: string }, };

export type Look = { 
/**
 * The check that took the pictures.
 */
check: string, 
/**
 * The base commit it was compared with.
 */
base_commit: string, 
/**
 * How the check did on the base commit.
 */
base_result: string, 
/**
 * Why there is no before (no base branch, the check fails on the base, the look
 * failed). Then every picture counts as new and needs approval.
 */
no_before?: string | null, 
/**
 * Something about the comparison a person should know, such as a base run that passed
 * without taking any pictures.
 */
note?: string | null, 
/**
 * Every picture.
 */
items: Array<Picture>, };

export type Picture = { 
/**
 * Its path below the check's media folder.
 */
name: string, 
/**
 * `still` or `film`.
 */
kind: string, 
/**
 * How it differs from its before.
 */
change: PictureChange, 
/**
 * The comparison in one line.
 */
summary: string, 
/**
 * The copy of the before inside the look folder, relative to it.
 */
before: string | null, 
/**
 * The copy of the after inside the look folder, relative to it.
 */
after: string | null, 
/**
 * What comparing them found.
 */
comparison: Comparison | null, 
/**
 * Named by the check's `flaky`: compared and shown, but never blocks or waits for
 * approval.
 */
flaky?: boolean, };

export type PictureChange = "changed" | "timing" | "new" | "gone" | "unreadable" | "same";

export type Comparison = { 
/**
 * What it comes to.
 */
verdict: Likeness, 
/**
 * A still rather than a film.
 */
still: boolean, 
/**
 * Width and height before.
 */
size_before: [number, number], 
/**
 * Width and height after.
 */
size_after: [number, number], 
/**
 * Length of the before film, in seconds.
 */
total_before: number, 
/**
 * Length of the after film, in seconds.
 */
total_after: number, 
/**
 * The largest shift of a held state, in seconds, and where it is in the before film.
 */
max_shift: number, 
/**
 * Where the largest shift is in the before film, in seconds.
 */
max_shift_at: number | null, 
/**
 * How many stable states the before film holds.
 */
states_before: number, 
/**
 * How many stable states the after film holds.
 */
states_after: number, 
/**
 * Where the pictures differ.
 */
moments: Array<Moment>, 
/**
 * The states matched between the films.
 */
pairs: Array<Pair>, };

export type Likeness = "same" | "timing" | "changed";

export type Moment = { 
/**
 * Start and end in the before film, in seconds.
 */
before: [number, number], 
/**
 * Start and end in the after film, in seconds.
 */
after: [number, number], 
/**
 * Where they differ.
 */
box: [number, number, number, number], 
/**
 * How many pixels differ.
 */
px: number, };

export type Pair = { 
/**
 * The state's index in the before film.
 */
before: number, 
/**
 * The state's index in the after film.
 */
after: number, 
/**
 * Frame index of the state in the before film.
 */
before_frame: number, 
/**
 * Frame index of the state in the after film.
 */
after_frame: number, 
/**
 * Start time of the state in the before film, in seconds.
 */
before_at: number, 
/**
 * Start time of the state in the after film, in seconds.
 */
after_at: number, 
/**
 * How many pixels differ.
 */
px: number, 
/**
 * Where they differ.
 */
boxes: Array<[number, number, number, number]>, };

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
