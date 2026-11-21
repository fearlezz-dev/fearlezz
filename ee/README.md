# Fearlezz hosted parts

Hosted, paid parts of Fearlezz. The open core in `crates/` never depends on anything here; it
only speaks the protocol below. The host reads `proof.json` through the core's own bundle
model, the small crate `crates/fearlezz-proof` (serde types, the review of a head, and who may
say what), so both always read one shape.

## Proof host

A Cloudflare Worker written in Rust (`workers-rs`), with R2 for files, D1 for records and one
Durable Object per piece of work, and per repository, for its open pages. CI streams a run to it
while the run goes (`fearlezz run --ci`), and so does a signed in developer's `fearlezz run`; the
host draws the work's page itself, live, for the people who may read the repository, who
review it there, and for anyone a writer gave a share link.

A repository opts in by installing the Fearlezz GitHub App and letting its workflow request
GitHub's OIDC token (`permissions: id-token: write`) on its proof job. Both are needed:
without either, CI still proves the commit but sends nothing, and there is no page or review.
CI asks whether the repository is installed before it sends anything; a job without the
permission says so in its log and job summary, and `fearlezz setup login` says whether the app
is installed on the repository it runs in. The app needs these repository permissions:

| Permission | For |
| --- | --- |
| **Metadata** read | who may read a repository, and who may write or administer it |
| **Checks** write | the `Fearlezz review` check run |
| **Pull requests** write | the one review comment on each pull request |
| **Actions** write | the `workflow_job` webhook, and running a failed proof job again once an admin accepts what it weakens |

and subscribes to the webhook events **Pull request** and **Workflow job**, sent to
`<APP_ORIGIN>/api/github/webhook` with the webhook secret.

### Protocol, version 2

Every API call from CI carries `Authorization: Bearer <token>`: a GitHub Actions OIDC token
whose `aud` is the host's origin (for example `https://fearlezz.seamlezz.com`, the default),
fetched afresh for each call. The host checks its signature against GitHub's keys, its issuer,
audience and expiry, and takes the repository from its `repository` claim. A token only ever
acts for its own repository. A run for a pull request needs a token whose `head_ref` is the
run's branch (and whose `ref` is `refs/pull/<n>/merge`); a run for a push needs `ref` =
`refs/heads/<branch>`. Only a token for the ref that opened a run writes it.

A developer's machine speaks the same protocol for a **local run**, with the CLI's host token
(`fzh_…`, see The CLI) instead: `fearlezz run` streams every run once the developer signed in
with `fearlezz setup login` and the host says the app is installed; without a sign in nothing
leaves the machine. The token opens and writes only its person's local runs (source `local`,
with their login, no pull request, no job), of repositories GitHub gives them write access to
(cached 5 minutes like every role), and uploads their blobs. `installed` then names the
repository: `GET /api/v2/installed?repository=<owner/name>`. A local run is the agent's word: it
never moves the work's head or review, the check run or the comment, and takes no verdict or
accept (notes, yes). The CLI waits at most 2 seconds to connect and 5 without a byte moving, and
after one failure sends nothing more for that run.

The bodies are the serde types of `crates/fearlezz-proof/src/events.rs`.

| Call | Does |
| --- | --- |
| `GET /api/v2/installed` | `200 {"installed": true}`, or `200 {"installed": false, "install": "<url>"}`. |
| `POST /api/v2/runs` | Body `Open` (an OIDC token opens only a CI source, a CLI token only a local one with no `pull`, `400` otherwise): `{"repository", "branch", "pull"?: {"number", "title", "url"?}, "head", "base", "source": {"kind": "ci", "job", "run_url"?, "run_id", "run_attempt", "runner_name", "leg"?, "planned"?}, "checks": ["<id>", ...]}`, the checks it plans in order; a CI source names its job exactly with `GITHUB_JOB`,
`GITHUB_RUN_ID`, `GITHUB_RUN_ATTEMPT` and `RUNNER_NAME`, which tell the legs of a matrix apart. A run of a proof Fearlezz split over several jobs (`fearlezz run --ci --leg`) also names its `leg` (`linux`, `macos/ui`), the leg it counts as in review whatever GitHub names its job, and `planned`, every leg of its plan: the review stays running until each has a run. Refused unless installed. Makes the work (one branch of one repository) if it is new. `201 {"run": "<id>", "url": "<page url>"}`. |
| `POST /api/v2/runs/<run>/checks` | Body `{"started": {"check": "<id>"}}` when a check starts, or `{"finished": {"check": <proof.json check entry>, "files": {"<path>": {"sha256", "size"}}, "look"?: <look>}}` when it ends, with the files it made (its output at `outputs/<id>.log`, its pictures at `looks/<folder>/...`) and, for a check that takes pictures, its look: the pictures compared with the base commit, as in `proof.json` (`400` when the look is of another check). A check the run did not plan joins at the end. `200 {"missing": ["<sha256>", ...]}`: the blobs the host does not hold yet. |
| `POST /api/v2/runs/<run>/weakenings` | A CI run asks this before it judges what its change weakens; a closed run answers too, so the gate of a split proof asks through a leg's run. Body `{"weakened": [<weakening>, ...]}`, one per part, each `{"check" or "setting", "hows", "was"?, "now"?}`: the part by name, every way it weakened, and the part as TOML in the base and the head config where it is there. `200 {"accepted": <accept> \| null}`: the admin's accept of the run's head (`{"head", "weakened", "by", "at"}`) when it covers exactly these weakenings (each part and its ways, in any order; never the TOML), nothing more or less, else `null`. `409` for a local run. |
| `PUT /api/v2/blobs/<sha256>` | The bytes of one missing file, with `Content-Length`. Stored by content, per repository, so a file that did not change is sent once. `204`. |
| `POST /api/v2/runs/<run>/done` | Body `{"proof": <proof.json>, "files": {...}}`: every file of the bundle. `409 {"error", "missing": [...]}` while a blob is missing (send them and say done again); otherwise the run is done, the proof's entries and looks are the checks' final word, and it answers `200 {"url": "<page url>"}`. |

A refusal answers `{"error": "<what to do>"}` with `401` for a token that does not pass, `403`
for a repository without the app or a token for another repository or ref, `400` for a body
the host cannot read (an unknown `proof.json` schema, bytes that do not hash to their name, a
`Content-Length` other than the file's size), `404` for a run of another repository, and `409`
for a run that is done, or a blob no run of the repository names.

**A host error never fails the proof**, and a host never loosens it. What the change weakens
fails the job exactly as without a host, unless the host answers `weakenings` with an admin's
accept of exactly those weakenings at this head; the proof then names the accept
(`proof.json`'s `accepted`) and passes. A host that cannot be reached, refuses or answers
anything else means not accepted. CI prints `proof page: <url>` in its log and links the
page in the job summary; when a call fails it prints `proof page not sent: <why>` and stops
sending for that run, and the proof goes on.

### Pages

The host builds every URL. A work's page is at its branch or, once a run named one, its pull
request:

| Path | Shows |
| --- | --- |
| `/<owner>/<repo>` | The repository page: every work of the repository (see below). |
| `/<owner>/<repo>/work/<branch>` | The work's page (a branch with slashes keeps them, each segment escaped). |
| `/<owner>/<repo>/pull/<n>` | The same page, by its pull request. |
| `/-/share/<token>` | The work's page through a share link: for anyone, view only (see Share links). A GitHub owner never starts with a dash, so `/-/` names no repository. |
| `/live/...` | A page's live connection, at `/live` before the page's path. |
| `/login`, `/callback`, `/logout` | GitHub sign in. |

The page is a Solid app (`ee/app`) the worker serves from its static assets: the same
`index.html` for every page, which draws itself from its live connection. An unknown work, or a
repository the host holds no work of, is `404`. It answers `no-store`, may not be framed, its `connect-src` is `'self'` and
the files origin (the page reads a text there to diff it), and its `img-src` and `media-src`
name the files origin, the only place pictures load from.

**Live.** D1 is the one record; the Durable Object of a work (`WorkRoom`) holds the WebSockets
of its open pages, which hibernate with it, and runs every write to the work, one at a time
(`blockConcurrencyWhile`): the write and the bump of the work's `seq` in one D1 batch, then the
event to its pages, then the review derived again when the write can move it. So pages get the
events in the order of their `seq`, each one more than the one before, and the review last
written is derived from the newest state. Nothing in a write waits for GitHub: the room's alarm
publishes the review right after (see Review), and every call to GitHub gives up after 10
seconds. The live connection's messages are `events::Live`:

- first `{"kind": "snapshot", "seq", "work", "runs"}`: the work and its newest 20 runs, each with
  its checks in order (planned, `started` at a time, or with their `entry`, and their `look`
  once compared);
- then `{"kind": "event", "seq", "run", "change"}` for each write, where `change` is `opened`
  (a new run), `started`, `finished` (with the check's look and its pictures' content keys, if
  any), `done` (with every look and key) or `cancelled` (its job ended without done);
- and `{"kind": "work", "seq", "change"}` for what belongs to the work, not one run: a
  `verdict`, the head's `review` as it now stands, or the CI jobs `waiting` for a runner.

The snapshot also carries the work's `review`, its `verdicts` and its `waiting` jobs.

A look reaches pages, in a snapshot or an event, only once every picture it names is held: CI
sends a check's end before the pictures the host lacks, so until then a page could only show
them broken. The blob upload that completes a look sends it as a `finished` event of its check.

**The repository page** lists every work of the repository, newest activity first (when the
work last changed: `works.active`, set by every write that bumps its `seq`): its branch, its pull
request's number and title when a run named one, where its newest run stands (running, passed,
failed, stopped, and whether it is a local run, unproven, with who ran it), and what waits on
it: pictures of its head without a verdict, open notes, and a weakening to accept. Each row
leads to the work's page; the work page's repository name leads back. Who sees it is the work
page's rule. Its live connection (`/live/<owner>/<repo>`, held by the repository's own Durable
Object) sends `events::Listing`: first `{"kind": "snapshot", "repository", "works": [...]}`,
then `{"kind": "work", ...}` with one work as it now stands each time that work changes; the
page keeps the newest of each work by its `seq`. This replaces the old local overview page;
`fearlezz --all` prints text.

**Pictures.** The page shows every look of the run it shows as it arrives, so a person can
start looking before the run ends: counts that filter, grouping by check or by the parts of a
picture's name (screen, viewport, theme), changed pictures first, then new and gone, then
timing, and the same ones folded. The picture on the stage compares its before and after in
five ways (side by side, slider, onion skin, difference, blink) with outlines of where they
differ; films play on one clock with a lane per film and a jump to each change. A text shows as
its diff, line by line with the words that differ marked, read from the files origin. Flaky
pictures show, marked. `j` and `k` step between pictures, `1` to `5` pick the way to compare. The page
gets a run's files with `GET /api/v2/runs/<run>/grant`, which answers
`{"files": "<files origin>/<grant>/", "expires"}` to someone who may see the work (the page's
rule: `401` without a session for a private repository, `404` otherwise), or with
`?share=<token>` to a share link of the run's work (`404` for any other run, or a link that does
not hold), and is asked again before it ends.

The room takes a socket before it reads the snapshot, so nothing written between the two is
missed; the page drops events its snapshot already holds by `seq`. A dropped connection opens
again with a fresh snapshot. Asked without upgrading, `/live/...` answers the snapshot as JSON.
A live connection without access gets `401`, an unknown work `404`, and a work that is gone
closes with `1008`, as does a share link's page when the link is revoked and a page whose
viewer lost access (Who sees a page).

**Types.** The page's TypeScript types (`ee/app/src/proof.ts`) are generated from
`fearlezz-proof` with ts-rs; a test of `tests/ee` fails when they drift, and
`FEARLEZZ_WRITE_TYPES=1 cargo test --manifest-path ee/Cargo.toml --test types` writes them.

### Review

The host is where review happens. Its one model is `crates/fearlezz-proof/src/review.rs`: the
review of a work's head is derived from the head's CI runs (the newest run of each leg: a leg is
the job with the display name GitHub gives it, `proof (macos-latest)`, so every leg of a matrix
counts, and a run again, whole or of failed jobs only, replaces just the legs it ran; until the
`workflow_job` webhook names a run's job, the run is a leg of its own, so its pictures never drop
out; a run that ended without done replaces nothing, so the leg's older run keeps its failure
and pictures in review, and a leg with no run that said done keeps the review from approving), the work's verdicts and the head's accept, and stored in `works.review` only to tell when it changed.
Every write that can move it (a run opening, a look arriving, done, a cancel, a verdict, an
accept, a push to the pull request) derives it again in the work's room, after the writes
before it; when it changed, open pages get it live. GitHub gets it as the check run and the
comment right after, from the room's alarm, which never runs twice at once and publishes the
review last stored, so runs that open at once still make one comment and one check run, GitHub
ends with the newest review, and a GitHub that hangs or fails never fails or holds a write.

- **Verdicts are per picture**, on a CI run only, by a signed in person with write access.
  A picture to look at is changed, new, gone, timing or unreadable, and not flaky. A verdict is
  kept by check, picture and content: a still by the SHA-256 of its before and after, a changed
  film by which of its stable states changed (its bytes differ each time it is recorded), a new
  or gone film by its bytes, so its verdict never carries over to another recording, and a text by
  the SHA-256 of its before and after like a still. So a picture
  that did not change keeps its verdict across runs and pushes; the page says "carried over".
  A verdict only says a picture looks right: what should change is a note.
- **A picture needs change while an open note is on it**, whatever verdict its content has;
  that is derived, never stored. Resolving its last note brings it back to looks right when a
  verdict holds for its current content, and to not looked at yet otherwise. Each picture of
  the review says where it stands (`standing`: `waits`, `needs_change` or `looks_right`).
- **Notes** say what should change, on a CI run or a local one (so feedback reaches the agent
  before a pull request exists): a region of a picture's before, its after or both (in
  fractions of the picture), at a moment of a film, with the person's words. A note is kept by
  check and picture, so an open note shows on every later run of the work until a person with
  write access resolves it.
- **An accept is per head**, by an admin of the repository, of exactly what a finished CI run
  of it weakens, which failed its proof job. The host keeps it and runs that failed job again
  as the app: it lists the jobs of the run's workflow run attempt (`GET /repos/{owner}/{repo}/
  actions/runs/{run_id}/attempts/{attempt}/jobs`) and re-runs the run's own job when it failed
  (`POST /repos/{owner}/{repo}/actions/jobs/{job_id}/rerun`): the job its runner took, see
  below. The job asks for
  the accept before it judges its weakenings (Protocol above) and passes, so the required job
  stays the one gate and the host's review honours only its own accept, never one a proof names.

| Call | Does |
| --- | --- |
| `POST /api/v2/runs/<run>/verdicts` | Body `{"check", "picture"}`: it looks right. `201 {"verdict"}`. `403` without write access; `409` for a local run, a picture with nothing to look at, or one whose files are not all in yet. |
| `POST /api/v2/runs/<run>/notes` | On a CI or a local run. Body `{"check", "picture", "side": "before" \| "after" \| "both", "region": {"x", "y", "w", "h"}, "time"?, "words"}`. `201 {"note"}`. `400` for empty words (at most 2000 characters), a region outside the picture, a side the picture lacks, or a time on a still; `403` without write access; `409` for a picture the run does not have. |
| `POST /api/v2/notes/<id>/resolve` | Body `{}`. `200 {"note"}`. `403` without write access. |
| `POST /api/v2/runs/<run>/accept` | Body `{}`. `201 {"accept"}`, once GitHub runs the failed proof job again. `403` unless admin, `409` unless the run is done with weakenings; `502` when the accept is kept but GitHub did not run the job again (run it again on GitHub). |
| `GET /api/v2/viewer/<owner>/<repo>` | `{"login", "role": "admin" \| "write" \| "read"}`, or `{"login": null}` signed out. |

Every write takes only a browser session: a request with an `Authorization` header is `403`
(a CI token or any bearer never gives a verdict), one whose `Origin` is not the app origin is
`403`, one whose body is not `application/json` is `415`, and one without a session is `401`.
A run the person may not see is `404`. Write and admin access come from GitHub's collaborator
permission, cached 5 minutes per login and repository.

**The check run** `Fearlezz review`, written as the app on the pull request's head:

| Review | Check run | Title |
| --- | --- | --- |
| no CI run of the head yet | `queued` | Waiting for CI to prove `<head>` |
| a CI run is running | `in_progress` | CI is proving `<head>` |
| the proof failed | completed, `failure` | The proof of `<head>` failed (and, when it weakens what no admin accepted, that it waits for an admin's accept) |
| an open note says a picture needs change | completed, `failure` | "1 picture needs change at `<head>`" |
| a job whose every run ended without done (a run again that ends so replaces nothing) | completed, `cancelled` | A CI job of `<head>` ended without its proof: run it again |
| a picture waits for a look | completed, `action_required` | for example "2 pictures wait for a look" |
| everything looks right | completed, `success` | Everything looks right |

Waiting is `action_required`, not `neutral`: it completes the check run, so a required check
blocks the merge, and GitHub shows it as asking for something; `neutral` would let a required
check pass. Its details link is the work's page. The summary lists the pictures that need
change by their open notes with their words, the ones that wait, the ones that look right and
by whom, and the weakenings with their accept. Make `Fearlezz review`
a required check in the branch's protection to hold merges to review. A check run that is
completed and goes back to running is created anew; otherwise it is edited, and an unchanged
one is not sent again.

**The comment.** One comment on the pull request, created on its first hosted run and edited
afterwards: where review stands and a link to the page. Nothing more is written on a closed
pull request.

**Webhooks** (`POST /api/github/webhook`), each checked against `X-Hub-Signature-256` with the
webhook secret before anything in it is read (`401` when the signature does not hold):

- `pull_request` opened, synchronize or reopened: the pull request's head becomes its work's
  head, so the check run says it waits for CI. A pull request from a fork is passed over: its
  CI gets no OIDC token, so no hosted run comes.
- A local run has no job: one that said nothing for an hour (no start, end, blob upload or
  done) is closed as cancelled by the host's schedule (a cron trigger every 10 minutes). A blob
  counts for each open run of its writer that names it, as its bytes start and once stored.
- `workflow_job`: a queued or waiting job whose name is the `GITHUB_JOB` of an earlier run of
  the repository, alone or with a matrix's values after it (`proof (macos-latest)`), shows on
  its branch's page as waiting for a runner. A completed one cancels its own run if that never
  said done: the run of its workflow run, attempt and runner (`run_id`, `run_attempt`,
  `runner_name`), so a matrix leg ending leaves its siblings running. A job is a run's own by
  workflow run, attempt and runner alone, whatever its workflow names it (`name: Proof`); only
  when one runner took several jobs of the attempt (a self hosted one) does the name choose,
  the one that is the run's `GITHUB_JOB` alone or with a matrix's values, and when that leaves
  it unclear no job is the run's.
  Once a runner took a job (in progress or completed), its name is the leg of that same run, and
  the review is derived again.

**Deploying review.** In the GitHub App's settings, add the permissions and events above, set
the webhook URL and a webhook secret. Put the secret in 1Password as `WEBHOOK_SECRET` (declared
in `secretspec.toml`), then run the devenv tasks `host-push-secrets` and `host-deploy`; the
deploy applies the D1 migration `0004_hosted_review.sql` (the whole schema of protocol 2 and
review, over the deployed `0001` to `0003`) and the schedule (`[triggers]` in `wrangler.toml`).
Existing installations
must approve the new permissions (Actions write among them) before the host can write their
check runs or run a proof job again. For
`fearlezz setup login`, also tick **Enable Device Flow** in the app's settings.

### Share links

A person with write access makes a link that shows anyone the work's live page, view only,
until someone with write access revokes it. A link names the work (its branch, and so its pull
request once it has one), not a run, so it follows new runs.

- **The token** is 128 random bits in the link's URL, `<APP_ORIGIN>/-/share/<token>`. The host
  keeps only its HMAC with `SIGNING_KEY` (`shares`: id, hash, work, who made it, when), like the
  CLI's token, so the table alone opens nothing. The list therefore shows who made each link
  and when, never the link again: it is shown once, when made. Revoking deletes the row and
  closes the pages open on the link (`1008`, "This link was revoked."). An unknown or revoked
  token gets a bare `404` that names no repository.
- **Only that work.** The link skips the repository's access check for its work's page, live
  connection and run grants alone. Everything else still needs what it needed before.
- **It only views.** Through a link the page says so, offers no verdict, note, resolve, accept
  or share, and asks no one to sign in, even a signed in writer. Every write still takes a
  browser session (Review above), so the link gives none; as a bearer it is refused like any
  other.
- **The files** come from the files origin under the same one hour grant of one run as any
  other view, asked afresh by each page; the token never reaches the files origin. The page
  answers `Referrer-Policy: no-referrer` and `X-Robots-Tag: noindex, nofollow`.
- **The Share control** shows beside the work's name for a signed in person with write access:
  it makes a link and shows it to copy, and lists the work's live links, each with Revoke.

Making and revoking take what a verdict takes (a session, `Origin` = `APP_ORIGIN`, a JSON body,
no `Authorization` header); listing takes the session. Each needs write access to the
repository (GitHub's collaborator permission, cached 5 minutes like every role): `403` without
it, `404` for a work or private repository the person may not see, `401` without a session.

| Call | Does |
| --- | --- |
| `POST /api/v2/shares` | Body `{"repository": "owner/name", "branch"}`. `201 {"url", "share": {"id", "made_by", "made_at"}}`. |
| `GET /api/v2/shares?repository=<owner/name>&branch=<branch>` | `200 {"shares": [{"id", "made_by", "made_at"}]}`, newest first. |
| `POST /api/v2/shares/<id>/revoke` | Body `{}`. `200 {"revoked": "<id>"}`; `404` for a link that is gone. |

### The CLI

The agent reads what people said through Fearlezz, never through the page. `fearlezz setup
login` runs GitHub's device flow through the host (which holds the app's client id) and ends in
a host token (`fzh_…`) that the CLI keeps in the user's config folder, readable by the user
alone. The token speaks for the person who entered the code and is the host's own: it lives 30
days from its last use, until `fearlezz setup logout` ends it, and is kept on the host only as
its HMAC with `SIGNING_KEY` (`cli_tokens`). The host keeps the person's GitHub user token and its
refresh token beside it and renews the user token when it is about to expire, so the person
signs in once. It reads, and streams its person's local runs (Protocol above); a verdict, note,
resolve or accept refuses any `Authorization` header.

| Call | Does |
| --- | --- |
| `POST /api/v2/login/device` | `{"device_code", "user_code", "verification_uri", "expires_in", "interval"}`. `502` when GitHub refuses (the app's device flow is off). |
| `POST /api/v2/login/token` | Body `{"device_code"}`. `{"pending": true, "interval"}` until the person entered the code, then `{"token", "login"}`; `400` when the flow ended without them. |
| `DELETE /api/v2/login` | Ends the bearer's token. `204`. |
| `GET /api/v2/work/<owner>/<repo>/<branch>` | With the token: the work, its page URL, the review of its head and its open notes with the paths of their pictures (`notes::Feedback`). `401` without a token that holds, `404` for a work or repository the person may not read. |
| `GET /api/v2/runs/<run>/files/<path>` | With the token: one picture, film or text of a run, as a download (`Content-Disposition: attachment`, a sandboxing CSP, `nosniff`); only the types the files origin serves. |

`fearlezz` shows per picture its verdict or "not looked at yet" and its open notes when the
CLI is signed in and the branch is on GitHub, waiting at most two seconds; `fearlezz --notes`
prints each open note and saves its pictures with the region drawn (ffmpeg, in the CLI) into
the user cache folder, and a noted text as it is (a note on a text is about the whole text); the stop hook holds the agent once while notes are open.

### Two origins

A run's files are what CI wrote, so they never load where people are signed in. The one worker
answers on two origins and tells them apart by the request's host:

- the **app origin** (`APP_ORIGIN`, `https://fearlezz.seamlezz.com` by default): sign in, the
  API, the pages and their live connections;
- the **files origin** (`FILES_ORIGIN`, `https://fearlezz.seamlezz.net` by default): the runs'
  files under a grant, and showcase media (below), and nothing else. It reads no cookies and sets none.

Files origin: `/<grant>/<path>` is the run's picture, film or text at that path, with `Range`
support (films need it). Only png, jpeg, webp, gif, avif, mp4, webm and txt are served, each with
the content type its name pins (a text of a look is kept as `.txt`, served as
`text/plain; charset=utf-8`) and `X-Content-Type-Options: nosniff`; anything else (svg, html, a
log) is `404`, so nothing a pull request wrote ever runs as a page. Each answer allows the app
origin to read it (`Access-Control-Allow-Origin`), so the page can diff a text; the grant in the
URL is what opens it. The grant is
`base64url(json).base64url(mac)`: the JSON names the repository, the run and an expiry one hour
out, and the mac is HMAC-SHA256 with `SIGNING_KEY`. A missing, expired or altered grant, or one
for another run, gets `404`. Files answer with `Content-Security-Policy: frame-ancestors
<APP_ORIGIN>`. Each file's `ETag` is its SHA-256: `If-None-Match` with it gets `304`, and a
`Range` whose `If-Range` names another tag (or a date, since no file says when it changed) gets
the whole file with `200`, so a cache never joins the bytes of two versions.

### Showcase media

`<FILES_ORIGIN>/showcase/<owner>/<repo>/<check>/<picture>` is a stable address for a README, a
docs site or a website to embed: the picture `<picture>` (its name in the check's look, its after) of the newest push to
the repository's default branch whose done CI run ran the check and whose proof did not fail. It
holds no grant, and a grant always holds a dot, so `showcase` never names one.

- **Which run.** Only a run a push started: its OIDC token's `event_name` was `push`, its ref
  `refs/heads/<default branch>`, and the commit it proves is the token's `sha`. The ref alone is
  not enough: `pull_request_target`, `workflow_run`, `issue_comment`, `workflow_dispatch` and
  `schedule` runs carry it too, and the first two often check out a fork's code. So a pull
  request, another branch, another event, a run proving another commit and a local run never
  count. Each run keeps its token's `event_name` and `sha` (`runs.event`, `runs.sha`); runs
  opened before they were kept never count.
- **What it sends.** On a push to the default branch, `fearlezz run --ci` compares nothing:
  the pull request already showed and approved what changed, so each check's look holds every
  picture as taken, with no before and no run on the base. A push whose run takes no picture of
  a check (its pass reused from the base commit's proof, say) leaves the older push's picture,
  which is the same.
- **Did not fail.** Its proof must not have failed as review judges it
  (`fearlezz_proof::review::failed`: nothing to fix, no failed check, and what it weakens
  covered by the host's own accept of its head, so an accepted weakening counts and one not
  accepted does not). A newer push that failed, or whose run is not done, leaves the older
  picture, and a picture the newest proven push no longer takes is `404`. The bar is the run's
  proof, not the check alone: a showcase shows what the default branch proved. Of a split proof,
  each leg's run counts on its own, since the gate does not stream to the host.
- **Newest push.** Pushes are ordered as the host first saw their commit (its first run's
  place in `runs`, since `opened` holds whole seconds), not by when a run opened: a run of an older push started again (after a flake, or by an admin's accept)
  never brings its picture back over a newer push's. Of one push, its newest run that did not
  fail counts. There is no cap on how far back: the runs of pushes that ran the check are read
  newest push first, 20 at a time, until one whose proof did not fail, so a picture outlasts
  any number of newer pushes that failed or reused the check's earlier result.
- **Check and picture.** Both may hold slashes; the path names the longest check of the run its
  leading segments spell, and the rest is the picture.
- **Only stills and films.** png, jpeg, webp, gif, avif, mp4 and webm; a text of a look is
  `404`.
- **A film as an animated WebP.** GitHub strips `<video>` from a README and plays no film from
  another origin; only an animated image in `<img>` plays. So on a CI run a push to the default
  branch started (and no other run, so a pull request pays nothing), the CLI makes each film's
  animated WebP copy: 720 pixels wide at most and never upscaled, 12 frames a second, looping
  forever, its last frame held 1.5 s, starting at its first frame that is not one flat colour,
  quality 75. A film longer than 30 s from there gets none, and the CLI says so; without ffmpeg
  it says so and sends none, and the proof never fails over it. The copy is a file of the run
  beside the film's after, its extension swapped for `.anim.webp`, and the look item's `webp`
  names it, so it travels with the check's other files.
  A picture name that no item of the look has names the copy of the film whose name, with
  `.webp` in place of an extension of four characters or fewer (or added when it has none), is
  that name: `home.webp` is the copy of `home.webm`, `home.mp4` or `home`. One rule
  (`fearlezz_proof::look::webp_name`) names it here and in `fearlezz setup showcase`. A still
  the check took under that name wins.
- **Any repository with the app, private ones included.** GitHub's image proxy (camo) loads a
  README's pictures without signing in, even for a private repository, so the address holds no
  grant: anyone who knows it can load the picture, whatever the repository's visibility. The
  app asks GitHub for the repository (`GET /repos/{owner}/{repo}`) and its default branch, kept
  5 minutes like every answer of GitHub; a repository without the app, and anything not there,
  is `404`. An owner or name holding anything but
  letters, digits, `.`, `_` and `-`, or that is `.` or `..`, is `404` without asking GitHub.
- **Caching.** The host keeps which file an address names in the worker's cache
  (`caches.default`, keyed by the address without its query) for five minutes, so a view looks
  up neither GitHub nor the runs. The answer says `Cache-Control: public, max-age=300` and
  carries the `ETag`, `If-Range` and `Range` handling of every file of this origin, the type its
  name pins and `nosniff`. A push shows within five minutes, and a repository that removes the app
  can stay visible through these caches for those few minutes.

- **Any website.** Unlike a run's files, a showcase answer says `Access-Control-Allow-Origin: *`
  and `Cross-Origin-Resource-Policy: cross-origin`, so any site may embed it, also under
  cross-origin isolation, and read it from a script or a canvas. It holds only what the address
  already gives anyone.

A light and a dark variant are two pictures, chosen in the README with
`<picture><source media="(prefers-color-scheme: dark)" srcset="...">` (the root README shows it).
`fearlezz setup showcase` prints that markup for every picture of the work's last run.

### Who sees a page

Anyone for a public repository; otherwise a signed in person whom GitHub gives at least read
access to the repository. Anyone else gets `404`, so a private repository's name is not
confirmed. A person without a session is sent to sign in first. A work's page, and the repository page, follow
this rule alike; a share link opens one work's page to anyone holding it (Share links).

The host decides who asks and what they may do in one place, `access.rs`. Each request
resolves one viewer from its one door: someone anonymous, a share link, or a person signed in
by session or by CLI token (a page only ever reads the session cookie, the CLI's API only its
token). Every endpoint then asks one policy, `can(viewer, action, repository)`, which reads
GitHub's word through one cached lookup: whether a repository is public, whether a person may
read it, and their role on it.

- **Sessions** live in D1 (`sessions`): the person's login, GitHub user token and expiry (eight
  hours, as long as the token). The token stays on the server; it reads who may see a
  repository. The `session` cookie (HttpOnly, Secure, SameSite=Lax) holds
  only the row's random 128 bit id. Logging out deletes the row, so an old cookie signs in
  no one.
- **GitHub's answers are cached** in D1 (`cache`): who may read a repository and their role
  there, per login and repository, and whether a repository is public and the default branch its
  showcase pictures come from, each for 5 minutes; GitHub's OIDC keys for
  an hour, fetched again once when a token names a key id the cached set lacks. So a change
  of access on GitHub shows here within 5 minutes.
- **A live page follows the same rule while it stays open.** Its room asks again for each page
  a person opened before every event it sends, from the cache only (a write never waits for
  GitHub), and asks GitHub on an alarm right after an event the cache could not answer for, and
  at least every 5 minutes while one is open, and closes with `1008` ("Your access to this page ended; sign in again to see
  it.") a page whose session ended (logged out or expired) or whose access GitHub took away.
  A share link's pages stay until the link is revoked.

### Code

`ee/` is a Cargo workspace of its own, apart from the core's: `host` is the worker
(`wrangler.toml` holds its bindings and settings, `migrations/` the D1 schema), `app` the page
(Solid, TypeScript and Vite, run with bun; the worker's build builds it into its static
assets), and `host-tests` drives the worker over HTTP under `wrangler dev --local` against a
fake GitHub, and its pages in headless Chrome (Google Chrome or Chromium, from `/Applications`
or `PATH`) over the DevTools protocol on a pipe. Inside `devenv shell`,
`cargo test --manifest-path ee/Cargo.toml` builds the worker and runs them, and
`bun test ee/app` runs the page's own tests. Fearlezz proves it with the `fmt/ee`,
`clippy/ee`, `tests/ee`, `ee-page-types` and `ee-page-tests` checks.

The worker is built for size: `opt-level = "z"` with fat LTO and one codegen unit
(`ee/Cargo.toml`), and wasm-opt `-Oz` (`ee/host/Cargo.toml`). Its wasm is about 410 KB gzip
(1.0 MB raw); most of it is the host's own code and serde's code for the proof model's types.
