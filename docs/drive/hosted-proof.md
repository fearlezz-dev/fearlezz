# Hosted proof pages

## Goal

The hosted parts in `ee/`: proof links and pages a team can open without the developer's machine. Looking at a page, a person should see two things: did anything break, and did it implement what they asked for. "Correctly is just my judgment; that is why the page exists."

## What was built

- **One proof model.** `proof.rs` holds everything a run showed (every check, test case, finding, weakening, red before green, picture and film). `fearlezz run --ci --proof <dir>` writes it as `proof.json` with outputs, media and the page. The status comment, the check page and the proof page all come from it.
- **The proof page** (`report.rs`): a verdict with a strip of named pills, then what waits for you (approvals and weakenings), "Did anything break?" (a ledger of checks, failed ones first and open) and "Does it look right to you?" (every look with its compare modes and film lanes). A look on its own gets the same page.
- **Sending it from CI** (`host.rs`): with the Fearlezz GitHub App installed and `id-token: write`, CI asks the host whether the repository is installed, then sends only the files the host lacks, by sha256. `[host] url` overrides the default `https://fearlezz.seamlezz.com`.
- **The host** (`ee/host`): a Rust Cloudflare Worker with R2 and D1, protocol v1 in `ee/README.md`. It checks GitHub's OIDC token, stores bundles by content per repository, and shows a page to people GitHub lets read the repository (anyone for a public one). Bundle files come only from a separate files origin (`fearlezz.seamlezz.net`) through a signed grant for one proof that expires in an hour, framed by a shell on the app origin. Sessions live in D1, with only a random id in the cookie. Who may read, whether public, and GitHub's keys are cached.
- **Accept a weakening.** On a pull request from the repository itself, a weakened check no longer fails the proof: the status, renamed `fearlezz/approval`, waits until an admin comments `/fearlezz accept <words>`. A fork's weakening still fails, since its run cannot record what waits.
- **Approve and Accept on the page.** The page asks its shell by message; the host checks the session, the latest proof and its head, and write access (approve) or admin rights (accept), then posts the same comment as the person.
- **Share links.** A person with write access makes a link that opens the pull request's latest proof without signing in, view only, until revoked.
- **Red before green for a new check** says there is nothing on the base to compare, instead of reading a base that cannot have the check.

## Decisions

- **Full page first.** "A full page with every check, all of the findings, all of the pictures, all of the video analysis, the before and after, the comparison, approvals at the top, things that need to be weakened." Not half done.
- **The PR title, not the body,** "so I know which page belongs to which PR".
- **Density.** "Keep all of the information, but direct my attention to where it matters most, and let me still find what matters less."
- **The core draws the page;** the host stores, gates and approves. The same page shows locally.
- **Cloudflare, Rust.** Workers, R2 and D1, with `workers-rs`.
- **Access mirrors GitHub, plus share links.**
- **Opt in by installing the app.** CI asks before it sends anything.
- **Default host `fearlezz.seamlezz.com`, files on `fearlezz.seamlezz.net`:** a bad proof page cannot use the sign in of the app origin.
- **Sessions in D1;** the GitHub token stays on the server.
- **Approve posts the comment as the person.** "No new trust in CI and the pull request keeps the record"; a signed approval from the host would be a second way in.
- **Weakenings are accepted by an admin's comment** and wait like pictures, under one status renamed `fearlezz/approval`. This is breaking for rulesets that required `fearlezz/pictures`; Fearlezz is still at v0.
- **One Accept below the list,** since one accept comment covers every weakening.
- **A share link opens the pull request,** always its latest proof; viewers never approve or accept.
- **Red before green for checks a change adds:** fix the wording in this change, rather than leave it to fix itself once the check is on the base branch.

## Dropped tries

- **A spike of the stack** (throwaway Rust worker under `wrangler dev --local`). Kept as the direction, deleted as code. It showed that R2 with ranges, D1, the OIDC check through SubtleCrypto and GitHub sign in all work in Rust wasm, at 211 KB gzip; `Response::redirect()` cannot carry a cookie, and JS interop errors only show at runtime, so the host is tested over HTTP under `wrangler dev`.
- **Pages from the app origin.** Replaced by the separate files origin before anything was deployed: a proof page is content from a pull request and must not share an origin with the sign in.
- **The session in a signed cookie.** Replaced by sessions in D1, so the GitHub token never leaves the server and logout ends the session.
- **One Accept per weakening.** Pressing one marked every row as posted, which misled.

## Not proven yet

- Nothing is deployed: no Cloudflare account, GitHub App, domains or D1 database id yet. The page to shell messages are not driven in a real browser.
- Revoking a share link stops new loads at once; a page already open keeps loading files until its grant expires, up to an hour.
