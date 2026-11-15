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
installed before it sends anything. The app needs the repository permission **Metadata** read
(who may read a repository).

### Protocol, version 2

Every API call from CI carries `Authorization: Bearer <token>`: a GitHub Actions OIDC token
whose `aud` is the host's origin (for example `https://fearlezz.seamlezz.com`, the default),
fetched afresh for each call. The host checks its signature against GitHub's keys, its issuer,
audience and expiry, and takes the repository from its `repository` claim. A token only ever
acts for its own repository. A run for a pull request needs a token whose `head_ref` is the
run's branch (and whose `ref` is `refs/pull/<n>/merge`); a run for a push needs `ref` =
`refs/heads/<branch>`. Only a token for the ref that opened a run writes it.

The bodies are the serde types of `crates/fearlezz-proof/src/events.rs`.

| Call | Does |
| --- | --- |
| `GET /api/v2/installed` | `200 {"installed": true}`, or `200 {"installed": false, "install": "<url>"}`. |
| `POST /api/v2/runs` | Body `Open`: `{"repository", "branch", "pull"?: {"number", "title", "url"?}, "head", "base", "source": {"kind": "ci", "job", "run_url"?}, "checks": ["<id>", ...]}`, the checks it plans in order. Refused unless installed. Makes the work (one branch of one repository) if it is new. `201 {"run": "<id>", "url": "<page url>"}`. |
| `POST /api/v2/runs/<run>/checks` | Body `{"started": {"check": "<id>"}}` when a check starts, or `{"finished": {"check": <proof.json check entry>, "files": {"<path>": {"sha256", "size"}}, "look"?: <look>}}` when it ends, with the files it made (its output at `outputs/<id>.log`, its pictures at `looks/<folder>/...`) and, for a check that takes pictures, its look: the pictures compared with the base commit, as in `proof.json` (`400` when the look is of another check). A check the run did not plan joins at the end. `200 {"missing": ["<sha256>", ...]}`: the blobs the host does not hold yet. |
| `PUT /api/v2/blobs/<sha256>` | The bytes of one missing file, with `Content-Length`. Stored by content, per repository, so a file that did not change is sent once. `204`. |
| `POST /api/v2/runs/<run>/done` | Body `{"proof": <proof.json>, "files": {...}}`: every file of the bundle. `409 {"error", "missing": [...]}` while a blob is missing (send them and say done again); otherwise the run is done, the proof's entries and looks are the checks' final word, and it answers `200 {"url": "<page url>"}`. |

A refusal answers `{"error": "<what to do>"}` with `401` for a token that does not pass, `403`
for a repository without the app or a token for another repository or ref, `400` for a body
the host cannot read (an unknown `proof.json` schema, bytes that do not hash to their name, a
`Content-Length` other than the file's size), `404` for a run of another repository, and `409`
for a run that is done, or a blob no run of the repository names.

**A host error never fails the proof.** CI prints `proof page: <url>` in its log and links the
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
  (a new run), `started`, `finished` (with the check's look, if any) or `done` (with every
  look).

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
