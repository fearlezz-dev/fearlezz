# Fearlezz hosted parts

Hosted, paid parts of Fearlezz. The open core in `crates/` never depends on anything here; it
only speaks the protocol below. The host reads `proof.json` through the core's own bundle
model, the small crate `crates/fearlezz-proof` (serde types, what waits for a person, and who
may settle it), so both always read one shape.

## Proof host

A Cloudflare Worker written in Rust (`workers-rs`), with R2 for files, D1 for records and one
Durable Object per piece of work for its open pages. CI streams a run to it while the run
goes (`fearlezz run --ci`), and the host draws the work's page itself, live, for the people who
may read the repository.

A repository opts in by installing the Fearlezz GitHub App and letting its workflow request
GitHub's OIDC token (`permissions: id-token: write`). CI asks whether the repository is
installed before it sends anything. The app needs these repository permissions:

| Permission | For |
| --- | --- |
| **Metadata** read | who may read a repository, and who may write or administer it |
| **Checks** write | the `Fearlezz review` check run |
| **Pull requests** write | the one review comment on each pull request |
| **Actions** read | the `workflow_job` webhook |

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
| `POST /api/v2/runs` | Body `Open` (an OIDC token opens only a CI source, a CLI token only a local one with no `pull`, `400` otherwise): `{"repository", "branch", "pull"?: {"number", "title", "url"?}, "head", "base", "source": {"kind": "ci", "job", "run_url"?}, "checks": ["<id>", ...]}`, the checks it plans in order. Refused unless installed. Makes the work (one branch of one repository) if it is new. `201 {"run": "<id>", "url": "<page url>"}`. |
| `POST /api/v2/runs/<run>/checks` | Body `{"started": {"check": "<id>"}}` when a check starts, or `{"finished": {"check": <proof.json check entry>, "files": {"<path>": {"sha256", "size"}}, "look"?: <look>}}` when it ends, with the files it made (its output at `outputs/<id>.log`, its pictures at `looks/<folder>/...`) and, for a check that takes pictures, its look: the pictures compared with the base commit, as in `proof.json` (`400` when the look is of another check). A check the run did not plan joins at the end. `200 {"missing": ["<sha256>", ...]}`: the blobs the host does not hold yet. |
| `PUT /api/v2/blobs/<sha256>` | The bytes of one missing file, with `Content-Length`. Stored by content, per repository, so a file that did not change is sent once. `204`. |
| `POST /api/v2/runs/<run>/done` | Body `{"proof": <proof.json>, "files": {...}}`: every file of the bundle. `409 {"error", "missing": [...]}` while a blob is missing (send them and say done again); otherwise the run is done, the proof's entries and looks are the checks' final word, and it answers `200 {"url": "<page url>"}`. |

A refusal answers `{"error": "<what to do>"}` with `401` for a token that does not pass, `403`
for a repository without the app or a token for another repository or ref, `400` for a body
the host cannot read (an unknown `proof.json` schema, bytes that do not hash to their name, a
`Content-Length` other than the file's size), `404` for a run of another repository, and `409`
for a run that is done, or a blob no run of the repository names.

**A host error never fails the proof**, with one exception that fails safe. When the host took
the run (opened it and answered done), what the change weakens waits for an admin's accept on
the page (see Review) and does not fail the job; when the host cannot be reached or refuses,
the weakening fails the job as it does without a host. CI prints `proof page: <url>` in its log and links the
page in the job summary; when a call fails it prints `proof page not sent: <why>` and stops
sending for that run, and the proof goes on.

### Pages

The host builds every URL. A work's page is at its branch or, once a run named one, its pull
request:

| Path | Shows |
| --- | --- |
| `/<owner>/<repo>/work/<branch>` | The work's page (a branch with slashes keeps them, each segment escaped). |
| `/<owner>/<repo>/pull/<n>` | The same page, by its pull request. |
| `/live/<owner>/<repo>/...` | The page's live connection, at `/live` before the page's path. |
| `/login`, `/callback`, `/logout` | GitHub sign in. |

The page is a Solid app (`ee/app`) the worker serves from its static assets: the same
`index.html` for every work, which draws itself from its live connection. An unknown work is
`404`. It answers `no-store`, may not be framed, its `connect-src` is `'self'`, and its
`img-src` and `media-src` name the files origin, the only place pictures load from.

**Live.** D1 is the one record; the Durable Object of a work (`WorkRoom`) holds only the
WebSockets of its open pages, which hibernate with it. Every write to a work bumps its `seq`
in the same D1 batch, and then goes to the room as an event. The live connection's messages
are `events::Live`:

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

**Pictures.** The page shows every look of the run it shows as it arrives, so a person can
start looking before the run ends: counts that filter, grouping by check or by the parts of a
picture's name (screen, viewport, theme), changed pictures first, then new and gone, then
timing, and the same ones folded. The picture on the stage compares its before and after in
five ways (side by side, slider, onion skin, difference, blink) with outlines of where they
differ; films play on one clock with a lane per film and a jump to each change. Flaky pictures
show, marked. `j` and `k` step between pictures, `1` to `5` pick the way to compare. The page
gets a run's files with `GET /api/v2/runs/<run>/grant`, which answers
`{"files": "<files origin>/<grant>/", "expires"}` to someone who may see the work (the page's
rule: `401` without a session for a private repository, `404` otherwise) and is asked again
before it ends.

The room takes a socket before it reads the snapshot, so nothing written between the two is
missed; the page drops events its snapshot already holds by `seq`. A dropped connection opens
again with a fresh snapshot. Asked without upgrading, `/live/...` answers the snapshot as JSON.
A live connection without access gets `401`, an unknown work `404`, and a work that is gone
closes with `1008`.

**Types.** The page's TypeScript types (`ee/app/src/proof.ts`) are generated from
`fearlezz-proof` with ts-rs; a test of `ee-tests` fails when they drift, and
`FEARLEZZ_WRITE_TYPES=1 cargo test --manifest-path ee/Cargo.toml --test types` writes them.

### Review

The host is where review happens. Its one model is `crates/fearlezz-proof/src/review.rs`: the
review of a work's head is derived from the head's CI runs (the newest of each job), the work's
verdicts and the head's accept, and stored in `works.review` only to tell when it changed.
Every write that can move it (a run opening, a look arriving, done, a cancel, a verdict, an
accept, a push to the pull request) derives it again; when it changed, open pages get it live
and GitHub gets it as the check run and the comment.

- **Verdicts are per picture**, on a CI run only, by a signed in person with write access.
  A picture to look at is changed, new, gone, timing or unreadable, and not flaky. A verdict is
  kept by check, picture and content: a still by the SHA-256 of its before and after, a film by
  which of its stable states changed (its bytes differ each time it is recorded). So a picture
  that did not change keeps its verdict across runs and pushes; the page says "carried over".
  A verdict has a kind: `looks_right`, or `needs_change`, which a person can give only while
  the picture has an open note. A `needs_change` verdict holds for the content it was given on:
  once the picture changes, it waits for a look again.
- **Notes** say what should change, on a CI run or a local one (so feedback reaches the agent
  before a pull request exists): a region of a picture's before, its after or both (in
  fractions of the picture), at a moment of a film, with the person's words. A note is kept by
  check and picture, so an open note shows on every later run of the work until a person with
  write access resolves it. Resolving changes no verdict: whether the picture now looks right
  is said on its own.
- **An accept is per head**, by an admin of the repository, and covers every weakening of it.

| Call | Does |
| --- | --- |
| `POST /api/v2/runs/<run>/verdicts` | Body `{"check", "picture", "kind": "looks_right" \| "needs_change"}`. `201 {"verdict"}`. `403` without write access; `409` for a local run, a picture with nothing to look at, one whose files are not all in yet, or `needs_change` without an open note on the picture. |
| `POST /api/v2/runs/<run>/notes` | On a CI or a local run. Body `{"check", "picture", "side": "before" \| "after" \| "both", "region": {"x", "y", "w", "h"}, "time"?, "words"}`. `201 {"note"}`. `400` for empty words (at most 2000 characters), a region outside the picture, a side the picture lacks, or a time on a still; `403` without write access; `409` for a picture the run does not have. |
| `POST /api/v2/notes/<id>/resolve` | Body `{}`. `200 {"note"}`. `403` without write access. |
| `POST /api/v2/runs/<run>/accept` | Body `{}`. `201 {"accept"}`. `403` unless admin, `409` unless the run is done with weakenings that wait. |
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
| the proof failed | completed, `failure` | The proof of `<head>` failed |
| a picture needs change | completed, `failure` | "1 picture needs change at `<head>`" |
| every run ended without done | completed, `cancelled` | The CI run of `<head>` ended without its proof |
| a picture or a weakening waits | completed, `action_required` | what waits, for example "2 pictures wait for a look" |
| everything looks right and is accepted | completed, `success` | Everything looks right |

Waiting is `action_required`, not `neutral`: it completes the check run, so a required check
blocks the merge, and GitHub shows it as asking for something; `neutral` would let a required
check pass. Its details link is the work's page. The summary lists the pictures that need
change, the ones that wait, the ones that look right and by whom, the open notes with their
words, and the weakenings with their accept. Make `Fearlezz review`
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
- A local run has no job: one that said nothing for an hour (no start, end or done) is
  closed as cancelled by the host's schedule (a cron trigger every 10 minutes).
- `workflow_job`: a job runs Fearlezz when its name is the `GITHUB_JOB` of an earlier run of
  the repository, alone or with a matrix's values after it (`proof (macos-latest)`). A queued
  or waiting one shows on its branch's page as waiting for a runner; a completed one cancels
  the runs of its workflow run and job that never said done. A job its workflow renames with
  `name:` is not matched.

**Deploying review.** In the GitHub App's settings, add the permissions and events above, set
the webhook URL and a webhook secret. Put the secret in 1Password as `WEBHOOK_SECRET` (declared
in `secretspec.toml`), then run the devenv tasks `host-push-secrets` and `host-deploy`; the
deploy applies the D1 migrations (`0006_review.sql`, `0007_notes.sql`, `0008_local_runs.sql`,
which ends the sign ins made before it) and the schedule (`[triggers]` in `wrangler.toml`). Existing installations
must approve the new permissions before the host can write their check runs. For
`fearlezz setup login`, also tick **Enable Device Flow** in the app's settings.

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
| `GET /api/v2/runs/<run>/files/<path>` | With the token: one picture or film of a run, as a download (`Content-Disposition: attachment`, a sandboxing CSP, `nosniff`); only picture and film types. |

`fearlezz` shows per picture its verdict or "not looked at yet" and its open notes when the
CLI is signed in and the branch is on GitHub, waiting at most two seconds; `fearlezz --notes`
prints each open note and saves its pictures with the region drawn (ffmpeg, in the CLI) into
the user cache folder; the stop hook holds the agent once while notes are open.

### Two origins

A run's files are what CI wrote, so they never load where people are signed in. The one worker
answers on two origins and tells them apart by the request's host:

- the **app origin** (`APP_ORIGIN`, `https://fearlezz.seamlezz.com` by default): sign in, the
  API, the pages and their live connections;
- the **files origin** (`FILES_ORIGIN`, `https://fearlezz.seamlezz.net` by default): the runs'
  files, and nothing else. It reads no cookies and sets none.

Files origin: `/<grant>/<path>` is the run's picture or film at that path, with `Range`
support (films need it). Only png, jpeg, webp, gif, avif, mp4 and webm are served, each with the
content type its name pins and `X-Content-Type-Options: nosniff`; anything else (svg, html, a
log) is `404`, so nothing a pull request wrote ever runs as a page. The grant is
`base64url(json).base64url(mac)`: the JSON names the repository, the run and an expiry one hour
out, and the mac is HMAC-SHA256 with `SIGNING_KEY`. A missing, expired or altered grant, or one
for another run, gets `404`. Files answer with `Content-Security-Policy: frame-ancestors
<APP_ORIGIN>`.

### Who sees a page

Anyone for a public repository; otherwise a signed in person whom GitHub gives at least read
access to the repository. Anyone else gets `404`, so a private repository's name is not
confirmed. A person without a session is sent to sign in first.

- **Sessions** live in D1 (`sessions`): the person's login, GitHub user token and expiry (eight
  hours, as long as the token). The token stays on the server; it reads who may see a
  repository. The `session` cookie (HttpOnly, Secure, SameSite=Lax) holds
  only the row's random 128 bit id. Logging out deletes the row, so an old cookie signs in
  no one.
- **GitHub's answers are cached** in D1 (`cache`): who may read a repository, per login and
  repository, and whether a repository is public, each for 5 minutes; GitHub's OIDC keys for
  an hour, fetched again once when a token names a key id the cached set lacks. So a change
  of access on GitHub shows here within 5 minutes.

### Code

`ee/` is a Cargo workspace of its own, apart from the core's: `host` is the worker
(`wrangler.toml` holds its bindings and settings, `migrations/` the D1 schema), `app` the page
(Solid, TypeScript and Vite, run with bun; the worker's build builds it into its static
assets), and `host-tests` drives the worker over HTTP under `wrangler dev --local` against a
fake GitHub, and its pages in headless Chrome (Google Chrome or Chromium, from `/Applications`
or `PATH`) over the DevTools protocol on a pipe. Inside `devenv shell`,
`cargo test --manifest-path ee/Cargo.toml` builds the worker and runs them, and
`bun test ee/app` runs the page's own tests. Fearlezz proves it with the `ee-fmt`,
`ee-clippy`, `ee-tests`, `ee-page-types` and `ee-page-tests` checks.
