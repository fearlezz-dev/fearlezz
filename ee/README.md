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

### Who sees a page

Anyone for a public repository; otherwise a signed in person whom GitHub gives at least read
access to the repository. Anyone else gets `404`, so a private repository's name is not
confirmed. A person without a session is sent to sign in first. Who may approve (write access)
and share links come later.

- **Sessions** live in D1 (`sessions`): the person's login, GitHub token and expiry (eight
  hours, as long as the token). The `session` cookie (HttpOnly, Secure, SameSite=Lax) holds
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
