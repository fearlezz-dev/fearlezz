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

Pages:

| Path | Shows |
| --- | --- |
| `/<owner>/<repo>/proofs/<id>/` | The bundle's `index.html`; every other path below it is the bundle's file at that path, with its content type and `Range` support (films need it). |
| `/<owner>/<repo>/pull/<n>` | A redirect to the latest whole proof of that pull request. |
| `/login`, `/callback`, `/logout` | GitHub sign in. |

Who sees a page: anyone for a public repository; otherwise a signed in person whom GitHub gives
at least read access to the repository. Anyone else gets `404`, so a private repository's name
is not confirmed. A person without a session is sent to sign in first. Who may approve (write
access) and share links come later.
