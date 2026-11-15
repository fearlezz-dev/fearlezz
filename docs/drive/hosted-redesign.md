# Hosted redesign

Supersedes the approvals by comment, the shell and the CI written page of `hosted-proof.md`.

## Goal

"The hosted version should be more hosted." The proof page is drawn by the host, not generated
by CI, and updates live: waiting, running, done. A person can say "no, this doesn't look right
yet", draw on the old side, the new side or both, and the agent gets that feedback through
Fearlezz commands, with the pictures. The free version may be limited a bit more for it.

## What was built

- **Free core only proves.** `fearlezz run --ci` proves the commit and writes the job summary,
  listing changed pictures without blocking on them. A weakening fails the job; an admin can
  bypass branch protection. Gone from the core: approve and accept comments, the status comment,
  the `fearlezz/approval` status, the pictures branch and its workflow, `say approve`, the local
  report and overview pages.
- **The host owns review.** CI (OIDC) and signed in local runs stream to the host by protocol
  version 2 (`ee/README.md`). One Durable Object per work fans out live updates and serializes
  its writes; D1 is the one record. The page is TypeScript with Solid in `ee/app`, its types
  generated from `crates/fearlezz-proof`.
- **Review.** A verdict per picture ("looks right"), following the picture's content across
  pushes. Notes with a region on before, after or both, and a film time; an open note means the
  picture needs change. An admin accepts weakenings on the page, which re-runs the proof job;
  the job stays the one gate for weakenings. The app writes one `Fearlezz review` check run and
  one comment per pull request, counting the newest run of every matrix leg.
- **The agent's side.** `fearlezz setup login` (device flow through the host, a read and local
  upload token that can never review), `fearlezz` shows what waits, `fearlezz --notes` saves the
  noted pictures with regions drawn, and the stop hook holds the agent while notes are open.
- **Local runs** show on the host marked unproven; never approvable, notes allowed and carried
  over to the pull request.
- **Also:** share links that only view, a repository page of every work, and base checkouts
  prepared and run with the base commit's own `[env]` and shell.

## Decisions

- **Free CI only proves** (option A of three): keeping comment approvals in free would keep two
  authorities and all of the old machinery; blocking free CI on pictures would give free users a
  red check they cannot clear.
- **The host is the authority** for review, and the app sets the check. Follows from the above:
  no comment parser is left in the core.
- **Only the host has a page**, including the overview: free users get text and picture files.
- **Local runs show on the host, unproven**, so feedback works before a pull request exists.
- **Approving is per picture**, with no words; words belong in notes.
- **A verdict follows the picture's content** (a new or gone film by its bytes, a changed film by
  which states changed), so unchanged pictures are not reviewed again.
- **No local approval**: it only ever counted the agent's word.
- **Stack**: Rust worker, Durable Objects with hibernating WebSockets, Solid page, generated
  types; the core gets no JS build.
- **A look reaches pages only once its pictures have arrived**, rather than the page retrying.
- **Accept re-runs the proof**: installing the app must never loosen what free mode blocks.
- **Needs change comes from open notes**, so a verdict and notes cannot drift apart.
- **The newest run of every leg** counts, a leg being the job plus GitHub's display name, matched
  by run, attempt and runner, so a partial re-run never drops another leg's pictures.
- **One branch, one pull request**, so the hosted parts never break on main.

## Tries

- **Live transport spike** (picked): a throwaway Rust worker showed workers-rs 0.9 supports
  Durable Object WebSocket hibernation with no workaround; an object was evicted after 90 s idle
  and kept its socket. 4 ms from event to page locally. Deleted after recording.
- **One page app shared by local and hosted** (dropped): the developer chose only the host has a
  page.
- **A separate hosted app beside the local page** (dropped): two renderers of the same pictures.
- **Verdict per check in the person's own words** (dropped): cannot say one picture is fine and
  another is not, which notes need.
- **Verdicts reset on every push** (dropped): re-reviewing unchanged pictures breeds rubber
  stamping.
- **A weakening waiting in the optional review check** (dropped after review): installing the app
  loosened the gate unless the check was made required.
- **A stored needs change verdict** (dropped after review): it drifted from the notes.
- **Newest attempt per job** (dropped after review): a partial re-run dropped other legs'
  pictures from review.
- **The page retrying missing pictures** (dropped): a missing picture would look the same as one
  still uploading.
