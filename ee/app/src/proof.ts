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

export type Listing = { "kind": "snapshot", 
/**
 * `owner/name` as GitHub spells it.
 */
repository: string, 
/**
 * Its works, newest activity first.
 */
works: Array<Listed>, } | { "kind": "work" } & Listed;

export type Listed = { 
/**
 * The work.
 */
work: Work, 
/**
 * Its page's path on the host: its pull request's, or its branch's.
 */
path: string, 
/**
 * Its last event: a listing of the work with a higher one replaces this.
 */
seq: number, 
/**
 * When it last changed, in seconds since the epoch.
 */
active: number, 
/**
 * Its newest run, none before its first.
 */
latest?: Latest | null, 
/**
 * How many pictures of its head wait for a verdict.
 */
pictures: number, 
/**
 * How many notes on it are open.
 */
notes: number, 
/**
 * Whether what its head weakens waits for an admin's accept.
 */
weakened: boolean, 
/**
 * Where its newest run stands, as [`Listed::worded`] derives it.
 */
standing: ListedStanding, 
/**
 * Where its newest run stands, in a few words.
 */
word: string, 
/**
 * What waits on it, each as a short phrase.
 */
waits: Array<string>, };

export type ListedStanding = "none" | "running" | "passed" | "waits" | "failed" | "cancelled";

export type Latest = { 
/**
 * The commit it proves.
 */
head: string, 
/**
 * Where it runs: a local run is unproven.
 */
source: Source, 
/**
 * Whether it still runs.
 */
state: RunState, 
/**
 * How many of its checks failed so far.
 */
failed: number, };

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
keys?: { [key in string]: string }, 
/**
 * What the run's CLI found comparing its pictures with those a person said something
 * about.
 */
since?: { [key in string]: { [key in string]: boolean } }, } | { "kind": "done", 
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
verdict: PictureVerdict, 
/**
 * The picture's notes the verdict resolved, as they are now.
 */
resolved?: Array<Note>, } | { "kind": "withdrawn", 
/**
 * The check that took the picture.
 */
check: string, 
/**
 * The picture's name.
 */
picture: string, 
/**
 * The content the verdict was on.
 */
key: string, } | { "kind": "edited", 
/**
 * The notes as they are now.
 */
notes: Array<Note>, } | { "kind": "review", 
/**
 * The review, none while the work has no head.
 */
review?: Review | null, } | { "kind": "note", 
/**
 * The note as it is now.
 */
note: Note, } | { "kind": "resolved", 
/**
 * The notes as they are now.
 */
notes: Array<Note>, } | { "kind": "waiting", 
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
 * What the head weakens, as its proofs show it: it fails the proof until an admin accepts
 * it, and the accept runs the proof job again.
 */
weakened: Array<Weakening>, 
/**
 * The accept of exactly those weakenings, once an admin gave it.
 */
accept?: Accept | null, 
/**
 * The work's open notes on pictures the head asks a person to look at, oldest first: they
 * follow the work, not the head, and one whose picture the head does not show lapses.
 */
notes?: Array<Note>, 
/**
 * Where the preview deploy of the head lives, as its proof names it.
 */
preview?: string | null, 
/**
 * Where it stands in one line, as [`Review::title_for`] says it to each role; empty in a
 * review stored before the host sent it, until [`Review::titled`].
 */
title: ReviewTitle, };

export type ReviewState = "ci" | "running" | "failed" | "cancelled" | "needs_change" | "waits" | "approved";

export type ReviewTitle = { 
/**
 * To someone who may only read: signed out, through a share link, or read access. GitHub's
 * check run and comment say this one.
 */
read: string, 
/**
 * To someone with write access, who gives verdicts.
 */
write: string, 
/**
 * To an admin, who also accepts weakenings.
 */
admin: string, };

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
 * Where it stands: an open note on it outweighs a looks right verdict.
 */
standing: PictureStanding, 
/**
 * The looks right verdict on it, when someone gave one on this content.
 */
verdict?: PictureVerdict | null, 
/**
 * What a person said about what it showed before it changed, so it shows for a look
 * again; none when nothing was said about other content.
 */
changed_since?: ChangedSince | null, };

export type ChangedSince = { 
/**
 * The newest looks right verdict on what it showed before, when none is on what it shows
 * now.
 */
verdict?: PictureVerdict | null, 
/**
 * The ids of its open notes made on what it showed before.
 */
notes?: Array<string>, };

export type PictureStanding = "waits" | "needs_change" | "looks_right";

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

export type Accept = { 
/**
 * The commit it accepts the weakenings of.
 */
head: string, 
/**
 * What it accepts, as the proof it was given on showed it.
 */
weakened: Array<Weakening>, 
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
resolved_by?: string | null, 
/**
 * The note it was made from, such as the same note on another variant of the screen: the
 * first of them, so a group's notes share one link and an edit can reach them all.
 */
link?: string | null, };

export type NoteEdit = { 
/**
 * What should change there now.
 */
words: string, 
/**
 * Whether the notes linked with it take the words too.
 */
linked: boolean, };

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
 * Each way it weakened, such as `it runs another command`, in the order found.
 */
hows: Array<string>, 
/**
 * The part as the base branch's config has it, as TOML (`[check.tests]` with its fields,
 * or `no_proof = [...]`), absent where it has no such part.
 */
was?: string, 
/**
 * The part as the change's config has it, as TOML, absent where it is gone.
 */
now?: string, } & ({ "check": string } | { "setting": string });

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

export type Viewer = { 
/**
 * Their GitHub login.
 */
login: string, 
/**
 * Their role on the repository.
 */
role: Role, } | { 
/**
 * Always null.
 */
login: null, };

export type Role = "read" | "write" | "admin";

export type Share = { 
/**
 * Its id, which revokes it.
 */
id: string, 
/**
 * The GitHub login of who made it.
 */
made_by: string, 
/**
 * When, in seconds since the epoch.
 */
made_at: number, };

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
 * The job's id in its workflow (`GITHUB_JOB`), the same for every leg of a matrix.
 */
job: string, 
/**
 * The workflow run's page.
 */
run_url?: string | null, 
/**
 * The workflow run (`GITHUB_RUN_ID`). With the attempt and the runner it names the
 * one job a run is, a leg of a matrix among its siblings, as GitHub's `workflow_job`
 * webhook names it.
 */
run_id: number, 
/**
 * The workflow run's attempt (`GITHUB_RUN_ATTEMPT`), 1 until it is run again.
 */
run_attempt: number, 
/**
 * The runner that took the job (`RUNNER_NAME`).
 */
runner_name: string, 
/**
 * The leg it runs, for a run Fearlezz split over several jobs (`linux`, `macos/ui`):
 * the leg it counts as in review, without waiting for GitHub to name its job.
 */
leg?: string | null, 
/**
 * Every leg the run's plan made, this one among them: the review waits for each.
 */
planned?: Array<string>, } | { "kind": "local", 
/**
 * Who ran it: the login of the CLI's sign in, which the host sets from its token.
 */
login: string, };

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
keys?: { [key in string]: string }, 
/**
 * What its run's CLI found comparing its pictures with those a person said something
 * about.
 */
since?: { [key in string]: { [key in string]: boolean } }, };

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
 * failed). Then every picture counts as new, for a person to look at.
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
 * `still`, `film` or `text`.
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
 * Named by the check's `flaky`: compared and shown, but never waits for a person's look.
 */
flaky?: boolean, 
/**
 * A changed text's lines against its before, as the CLI compared them: kept lines folded
 * to a count past a few on each side of a change. None for a still or film, and in a
 * bundle from a CLI that did not write it.
 */
diff?: Array<DiffLine> | null, 
/**
 * The animated WebP copy of a film's after inside the look folder, relative to it: the one
 * form of a film a page that allows only images (a README on GitHub) plays. The CLI makes it
 * only on a CI run that a push to the default branch started, the only runs the host serves
 * pictures of at a stable address; None
 * otherwise, and for a still or text.
 */
webp?: string | null, 
/**
 * What the check said happened in its after film (marks and spans), timing it from them.
 */
marks?: Marks | null, 
/**
 * What the check said happened in its before film, on the base commit.
 */
before_marks?: Marks | null, };

export type PictureChange = "changed" | "timing" | "new" | "gone" | "unreadable" | "same";

export type DiffLine = { "kind": "same", 
/**
 * The line, without its line break.
 */
text: string, } | { "kind": "was", 
/**
 * The line, without its line break.
 */
text: string, } | { "kind": "now", 
/**
 * The line, without its line break.
 */
text: string, } | { "kind": "fold", 
/**
 * How many.
 */
count: number, };

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
pairs: Array<Pair>, 
/**
 * The films' marks differ: one has a mark or span the other does not, or in another order.
 */
marks_changed?: boolean, };

export type Likeness = "same" | "timing" | "changed";

export type Marks = { 
/**
 * Moments, in order.
 */
marks?: Array<Mark>, 
/**
 * Spans of time, in order.
 */
spans?: Array<Span>, };

export type Mark = { 
/**
 * What happened, as the check wrote it.
 */
name: string, 
/**
 * Seconds into the film.
 */
at: number, 
/**
 * Only for timing, not drawn on the film's timeline.
 */
hidden?: boolean, };

export type Span = { 
/**
 * What happens in it, as the check wrote it.
 */
name: string, 
/**
 * Seconds into the film it starts.
 */
start: number, 
/**
 * Seconds into the film it ends.
 */
end: number, 
/**
 * How its timing counts: the same by default.
 */
timing: Timing, };

export type Timing = "same" | "free";

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
 * The platform it ran on, in a project that names platforms; the same check can run on
 * several.
 */
platform?: string | null, 
/**
 * The hash of what it read, so CI on a later commit can tell a pass still holds.
 */
key?: string | null, 
/**
 * Where its pass was seen, when it did not run here: the base commit's CI, which ran it
 * on the same inputs (that run's page, or the commit without one).
 */
reused?: string | null, 
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
 * It failed: a failure, or a pass only on a rerun that the check's `flaky_tests` does not
 * name.
 */
failed: boolean, 
/**
 * It failed, then passed when the runner ran it again.
 */
flaky?: boolean, 
/**
 * What a failed test printed, from its runner's output, cut short: why it failed.
 */
output?: string | null, };

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
