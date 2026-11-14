# Hosted redesign: model draft

Working draft for the drive record `record.yaml`. Not decided until the record says so.

## Things the host knows

- **Work**: one branch of one repository (`owner/repo` + branch). Its page lives at
  `/<owner>/<repo>/work/<branch>`; once a pull request exists, `/<owner>/<repo>/pull/<n>` is the
  same page. The host builds every URL; the CLI never does.
- **Run**: one `fearlezz run`. Source is `ci` (job, run page, OIDC) or `local` (the signed in
  developer, unproven). State: queued, running, done, cancelled. Head and base commit. Checks
  stream in as they start and finish; pictures upload as they are made; `proof.json` closes it.
- **Picture**: identified by check and name, so it is the same picture across runs.
- **Note**: on a picture, in a run. Side (before, after, both), region in fractions of the
  picture, film time, words, author, state (open, resolved). An open note shows again on every
  later run of the same work, beside the new picture, until a person resolves it.
- **Verdict**: per picture and head commit, by a person with write access, on a CI run only:
  looks right, or needs change (with at least one open note).
- **Accept**: of the weakenings at a head commit, by an admin, on a CI run only.

## What GitHub sees

- One check run per pull request head, written by the app: in progress while CI runs, action
  required while something waits for a person, failure while a picture needs change, success when
  every changed picture looks right and every weakening is accepted. It links to the work page.
- One comment, written and updated by the app: the state in a line and the link.
- The CI job itself stays the proof (free and hosted alike); the app's check is the review.

## How data reaches the host

- CI: OIDC, as today, but as events while the run goes (run started with its planned checks,
  check started, check finished with its result, blob, run done with `proof.json`).
- Local: the same events with the developer's CLI token from `fearlezz setup login`.
- GitHub webhooks (`workflow_job`) for queued runs, so a page shows "waiting for a runner"
  before Fearlezz itself starts.

## Protocol, version 2 (draft)

Writers authenticate with a GitHub OIDC token (CI) or a host token from `setup login` (local).

| Call | Does |
| --- | --- |
| `POST /api/v2/runs` | `{repository, branch, pull?, head, base, source, checks: [<planned ids>]}`. Opens a run, running. Answers `{run, url}`: the host's page of the work. |
| `POST /api/v2/runs/<run>/checks` | One check's entry (`fearlezz_proof::Check`) as it starts and as it ends, with its files `{path: {sha256, size}}`. Answers the files the host lacks. |
| `PUT /api/v2/blobs/<sha256>` | Bytes of one missing file, as in version 1. |
| `POST /api/v2/runs/<run>/done` | `proof.json`. Refused while a file is missing. Closes the run; the host settles the check run. |
| `GET /api/v2/work/<owner>/<repo>/<branch>` | Host token only: runs, verdicts and open notes of the work, for `fearlezz`. |
| `GET /api/v2/blobs/<sha256>` | Host token only: a noted picture, for `fearlezz --notes`. |

A run that never says done is closed as cancelled when its job ends (the `workflow_job`
webhook), or after an hour for a local run.

## What the agent does

- `fearlezz` shows what waits on the host for this branch: open notes, verdicts, the CI run.
- `fearlezz --notes` saves each open note's pictures with the region drawn, and prints their
  paths and words, so the agent can look at them.
- A CLI token reads and uploads local runs; it can never give a verdict or accept. Only a
  browser session can. The hooks hold the agent from stopping while notes are open.

Sketch of what the agent sees:

```
$ fearlezz
...
waits on the host  fix/card-button, PR #24
  card / dark     needs change   2 notes by gabber235
  card / light    looks right
  checkout film   not looked at yet
next: fearlezz --notes   saves the noted pictures with each region drawn

$ fearlezz --notes
note 1  demo-card  card/dark.png  after
  "The button text is cut off."
  .../notes/pr-24/1-after.png        the after picture, region 1 drawn
  .../notes/pr-24/1-before.png       the before picture, same region, for comparison
note 2  demo-card  checkout.mp4  both, at 3.2 s
  "Spacing grew between the rows, it should stay 16px."
  .../notes/pr-24/2-before.png       frame at 3.2 s, region drawn
  .../notes/pr-24/2-after.png
next: change the code, then fearlezz run; a person resolves notes on the next proof
```

- Region drawing and film frames happen in the CLI (`media.rs` already decodes films with
  ffmpeg); the host only stores the note. So the host never runs image code.
- `fearlezz setup login`: the device flow, run through the host, gives the CLI a host token
  that can read and upload local runs, nothing more.
