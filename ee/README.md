# Fearlezz hosted parts

Hosted, paid parts of Fearlezz. The open core in `crates/` never depends on anything here; it
only speaks the protocol below.

## Proof host

A Cloudflare Worker written in Rust (`workers-rs`), with R2 for files and D1 for records. It
takes the proof bundle CI writes (`fearlezz run --ci --proof <dir>`: `proof.json`, the page
`index.html`, outputs and media) and shows its page to the people who may read the repository,
without the machine that ran it.

A repository opts in by installing the Fearlezz GitHub App and letting its workflow request
GitHub's OIDC token (`permissions: id-token: write`). CI asks whether the repository is
installed before it sends anything.

The app needs these repository permissions: **Metadata** read (who may read a repository, a
person's permission, a pull request's head) and **Pull requests** write (GitHub takes it, or
Issues write, for a comment on a pull request), so the user tokens of people signed in through
it may post their approve and accept comments. A user token acts with both the app's and the
person's permissions, so it posts only where the person could comment anyway.

### Protocol, version 1

Every API call from CI carries `Authorization: Bearer <token>`: a GitHub Actions OIDC token
whose `aud` is the host's origin (for example `https://fearlezz.seamlezz.com`, the default). The host checks
its signature against GitHub's keys, its issuer, audience and expiry, and takes the
repository from its `repository` claim. A token only ever acts for its own repository. For a
pull request, its `ref` (`refs/pull/<n>/merge`) must name the pull request the proof is for.

| Call | Does |
| --- | --- |
| `GET /api/v1/installed` | `200 {"installed": true}`, or `200 {"installed": false, "install": "<url>"}`. Asks GitHub whether the app is installed on the token's repository. |
| `POST /api/v1/proofs` | Body `{"proof": <proof.json>, "files": {"<path>": {"sha256": "<hex>", "size": <bytes>}}}`, every file of the bundle but `proof.json` (so `index.html` too). Refused unless installed. `201 {"id": "<id>", "url": "<page url>", "missing": ["<sha256>", ...]}`: the files the host does not hold yet. |
| `PUT /api/v1/blobs/<sha256>` | The bytes of one missing file, with `Content-Length`. Stored by content, per repository, so a picture that did not change is sent once. Refused when the bytes do not hash to the name. `204`. |
| `POST /api/v1/proofs/<id>/done` | Refused while a file is missing. Marks the proof whole and makes it the latest of its pull request. `200 {"url": "<page url>"}`. |

A refusal answers `{"error": "<what to do>"}` with `401` for a token that does not pass, `403`
for a repository without the app or a proof for another repository or pull request, `400` for
a body the host cannot read (an unknown `proof.json` schema, bytes that do not hash to their
name, a `Content-Length` other than the file's size), `404` for a proof of another
repository, and `409` for a blob no proof of the repository names or `done` while a file is
missing.

### Two origins

A proof's `index.html` is HTML that CI wrote, so it never runs where people are signed in. The
one worker answers on two origins and tells them apart by the request's host:

- the **app origin** (`APP_ORIGIN`, `https://fearlezz.seamlezz.com` by default): sign in, the
  API, and a shell page per proof;
- the **files origin** (`FILES_ORIGIN`, `https://fearlezz.seamlezz.net` by default): the
  bundles' files, and nothing else. It reads no cookies and sets none.

App origin pages:

| Path | Shows |
| --- | --- |
| `/<owner>/<repo>/proofs/<id>/` | The shell: a tiny title above a full height frame of `<FILES_ORIGIN>/<grant>/index.html`. The host writes it, never CI. Nothing below this path is served here. |
| `/<owner>/<repo>/pull/<n>` | A redirect to the latest whole proof of that pull request. |
| `/login`, `/callback`, `/logout` | GitHub sign in. |
| `POST /<owner>/<repo>/proofs/<id>/approve`, `.../accept` | Posts `/fearlezz approve <words>` or `/fearlezz accept <words>` on the pull request as the signed in person. Only the shell calls these; see below. |

Files origin: `/<grant>/<path>` is the bundle's file at that path, with its content type and
`Range` support (films need it), so the page's relative links keep working.

- **The grant** is `base64url(json).base64url(mac)`: the JSON names the repository, the proof
  id and an expiry one hour out, and the mac is HMAC-SHA256 with `SIGNING_KEY` over
  `grant.<payload>`. A missing, expired or altered grant, or one whose proof is not a whole
  proof of its repository, gets `404`. Reloading the shell hands out a fresh one.
- **The frame** has `sandbox="allow-scripts allow-same-origin"`: the page runs its scripts on
  the files origin but cannot navigate the window. Files answer with
  `Content-Security-Policy: frame-ancestors <APP_ORIGIN>` (and `X-Frame-Options: SAMEORIGIN`
  for browsers too old for it), so only the shell may frame them. The shell itself may not be
  framed and is never cached, since it holds a grant.

### Approving on the page

The page offers an Approve button per check whose pictures wait and an Accept button beside a
weakening that waits for an admin, each with a field for the person's own words. The host never
approves anything itself: it posts the same comment a person would write on the pull request,
as that person, with their GitHub token, and CI judges it exactly as any other (who may, when
it was written, what it covers). A fork's approval names its head:
`/fearlezz approve <short head> <words>`.

The page runs on the files origin, which never holds the sign in, so it cannot call the host.
Framed (`window.parent !== window`), its button sends
`postMessage({kind: "fearlezz:approve" | "fearlezz:accept", check, words})` to its parent. The
shell takes messages only from its frame's window and from `FILES_ORIGIN`, posts them to its own
origin, shows what came of it in a bar above the frame (the comment's link, or why it was
refused), and answers the frame with `{kind: "fearlezz:posted", of, check, ok, text}`. Opened
locally, not framed, the page shows the command or comment to copy instead. The shell's one
script runs by a nonce that changes per view, and its `connect-src` is `'self'`.

`POST /<owner>/<repo>/proofs/<id>/approve` (or `accept`) takes `{"check": "<check>", "words":
"<words>"}` (`check` only for approve) and answers `201 {"url": "<comment url>", "pull": <n>,
"comment": "<what was posted>", "login": "<who>"}`, or `{"error": "<why>"}` with:

| Status | When |
| --- | --- |
| `403` | The `Origin` header is not `APP_ORIGIN` (the session cookie is `SameSite=Lax` too), or the person lacks write access (approve) or admin rights (accept), as GitHub's collaborator permission says when asked with their token. |
| `415` | The body is not JSON, so no form on another site can post it. |
| `401` | No session, or GitHub no longer takes its token: sign in again. |
| `400` | Words empty, over 500 characters, or on more than one line. |
| `404` | No such whole proof, or a private repository the person cannot read. |
| `409` | The proof is not the latest whole proof of its pull request (the error names the newer one), the pull request's head moved past the proof's, or nothing of the kind waits in it (no changed pictures of that check, no weakening that waits). |
| `502` | GitHub refused the comment, such as when the app lacks the permission below. |

These are early refusals with clear reasons; CI still judges the comment.

### Who sees a page

Anyone for a public repository; otherwise a signed in person whom GitHub gives at least read
access to the repository. Anyone else gets `404`, so a private repository's name is not
confirmed. A person without a session is sent to sign in first. Approving needs write access
and accepting admin rights (above); share links come later.

- **Sessions** live in D1 (`sessions`): the person's login, GitHub user token and expiry (eight
  hours, as long as the token). The token stays on the server; it reads who may see a
  repository and posts the person's approve and accept comments. The `session` cookie (HttpOnly, Secure, SameSite=Lax) holds
  only the row's random 128 bit id. Logging out deletes the row, so an old cookie signs in
  no one.
- **GitHub's answers are cached** in D1 (`cache`): who may read a repository, per login and
  repository, and whether a repository is public, each for 5 minutes; GitHub's OIDC keys for
  an hour, fetched again once when a token names a key id the cached set lacks. So a change
  of access on GitHub shows here within 5 minutes.

### Code

`ee/` is a Cargo workspace of its own, apart from the core's: `host` is the worker
(`wrangler.toml` holds its bindings and settings, `migrations/` the D1 schema), and
`host-tests` drives it over HTTP under `wrangler dev --local` against a fake GitHub. Inside
`devenv shell`, `cargo test --manifest-path ee/Cargo.toml` builds the worker and runs them;
Fearlezz proves it with the `ee-fmt`, `ee-clippy` and `ee-tests` checks.
