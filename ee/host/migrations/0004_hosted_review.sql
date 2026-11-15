-- Protocol version 2 and review on the host: a run streams in while it goes, the host draws the
-- work's page live and holds its review (verdicts, notes, accepts), and the CLI signs in.
-- Nothing real was stored by version 1, so its tables go.
DROP TABLE latest;
DROP TABLE files;
DROP TABLE proofs;
DROP TABLE shares;

-- A piece of work: one branch of one repository (`repo` lowercased, `repository` as GitHub
-- spells it), with the pull request a run named, if any. `seq` counts its events: every write
-- to the work, its runs or their checks bumps it in the same batch, and sets `active`, when the
-- work last changed, so the repository page lists works newest activity first. `head` is the
-- newest commit of the work (a CI run's head, or a pull request's head from its webhook), whose
-- review the page and GitHub show, and `review` that review as last derived
-- (`fearlezz_proof::review::Review` as JSON), so a page's snapshot reads it and an unchanged one
-- is not sent again.
CREATE TABLE works (
  id TEXT PRIMARY KEY,
  repo TEXT NOT NULL,
  repository TEXT NOT NULL,
  branch TEXT NOT NULL,
  pull INTEGER,
  pull_title TEXT,
  pull_url TEXT,
  seq INTEGER NOT NULL DEFAULT 0,
  active REAL,
  head TEXT,
  review TEXT,
  UNIQUE (repo, branch)
);
CREATE INDEX works_by_pull ON works (repo, pull);
CREATE INDEX works_by_activity ON works (repo, active);

-- One run of a work. `git_ref` is the ref of the OIDC token that opened it, or `local:<login>`
-- for a person's local run: only the same writer writes it again. `source` is its JSON
-- (`events::Source`); `state` is running, done or cancelled; `proof` is its proof.json once
-- done. `touched` is when it last said something (opened, a check's start or end, a blob, done):
-- a local run silent for an hour is closed as cancelled, as it has no CI job whose end would
-- close it.
CREATE TABLE runs (
  id TEXT PRIMARY KEY,
  work TEXT NOT NULL REFERENCES works (id),
  git_ref TEXT NOT NULL,
  head TEXT NOT NULL,
  base TEXT NOT NULL,
  source TEXT NOT NULL,
  state TEXT NOT NULL,
  opened REAL NOT NULL,
  touched REAL NOT NULL,
  proof TEXT
);
CREATE INDEX runs_by_work ON runs (work, opened);
CREATE INDEX runs_by_state ON runs (state, touched);

-- Each check of a run, in its order: planned when the run opened, `started` once it starts,
-- `entry` (its `fearlezz_proof::Check` as JSON) once it ends, and `look`, its pictures compared
-- with the base commit (`fearlezz_proof::look::Look` as JSON), from its end or, as the final
-- word, from done.
CREATE TABLE checks (
  run TEXT NOT NULL REFERENCES runs (id),
  id TEXT NOT NULL,
  position INTEGER NOT NULL,
  started REAL,
  entry TEXT,
  look TEXT,
  PRIMARY KEY (run, id)
);

-- The files of a run, by their path in its proof bundle, and the blob holding each.
CREATE TABLE files (
  run TEXT NOT NULL REFERENCES runs (id),
  path TEXT NOT NULL,
  sha256 TEXT NOT NULL,
  size INTEGER NOT NULL,
  PRIMARY KEY (run, path)
);
CREATE INDEX files_by_blob ON files (sha256);

-- A person's verdict that a picture looks right, kept by check, picture and content
-- (`review::key`): a picture that did not change keeps it across runs of the work. A later
-- verdict on the same content replaces it. `run` and `head` say where it was given.
CREATE TABLE verdicts (
  work TEXT NOT NULL REFERENCES works (id),
  check_id TEXT NOT NULL,
  picture TEXT NOT NULL,
  key TEXT NOT NULL,
  login TEXT NOT NULL,
  at REAL NOT NULL,
  run TEXT NOT NULL,
  head TEXT NOT NULL,
  PRIMARY KEY (work, check_id, picture, key)
);

-- An admin's accept of one head of a work: exactly the weakenings it was given on
-- (`fearlezz_proof::floor::Weakening` as a JSON list), so a CI run of the head passes only when
-- it weakens just those.
CREATE TABLE accepts (
  work TEXT NOT NULL REFERENCES works (id),
  head TEXT NOT NULL,
  weakened TEXT NOT NULL,
  login TEXT NOT NULL,
  at REAL NOT NULL,
  PRIMARY KEY (work, head)
);

-- What the app wrote on a pull request: its check run of `head` and what it last said
-- (`sent`, the body as JSON, so an unchanged one is not written again), and its one comment
-- with its body. A closed pull request gets nothing more.
CREATE TABLE pulls (
  repo TEXT NOT NULL,
  number INTEGER NOT NULL,
  head TEXT,
  check_run INTEGER,
  sent TEXT,
  comment INTEGER,
  comment_body TEXT,
  closed INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (repo, number)
);

-- The CI jobs GitHub's workflow_job webhook named: queued, waiting, in_progress or completed.
CREATE TABLE jobs (
  repo TEXT NOT NULL,
  id INTEGER NOT NULL,
  workflow_run INTEGER NOT NULL,
  name TEXT NOT NULL,
  head TEXT NOT NULL,
  branch TEXT NOT NULL,
  state TEXT NOT NULL,
  url TEXT,
  since REAL NOT NULL,
  PRIMARY KEY (repo, id)
);
CREATE INDEX jobs_by_branch ON jobs (repo, branch, state);

-- A person's note on a picture (`fearlezz_proof::notes::Note`), by check and picture so it is
-- the same picture across the work's runs, made on run `run` at `head`. `side` is before,
-- after or both; the region is in fractions of the picture; `time` is the moment of a film.
-- An open note shows on every run of the work until a person resolves it.
CREATE TABLE notes (
  id TEXT PRIMARY KEY,
  work TEXT NOT NULL REFERENCES works (id),
  check_id TEXT NOT NULL,
  picture TEXT NOT NULL,
  run TEXT NOT NULL REFERENCES runs (id),
  head TEXT NOT NULL,
  side TEXT NOT NULL,
  x REAL NOT NULL,
  y REAL NOT NULL,
  w REAL NOT NULL,
  h REAL NOT NULL,
  time REAL,
  words TEXT NOT NULL,
  login TEXT NOT NULL,
  at REAL NOT NULL,
  state TEXT NOT NULL,
  resolved_by TEXT
);
CREATE INDEX notes_by_work ON notes (work, state, at);

-- The CLI's tokens, from `fearlezz setup login`, the host's own: kept as their HMAC with the
-- signing key, so the table alone opens nothing. Each speaks for a GitHub login, with that
-- person's GitHub token (`token`, until `token_expires` when GitHub sets one) deciding what it
-- may read and write, and the refresh token the host renews it with. A token lives 30 days
-- from its last use (`used`), or until `fearlezz setup logout`.
CREATE TABLE cli_tokens (
  hash TEXT PRIMARY KEY,
  login TEXT NOT NULL,
  token TEXT NOT NULL,
  token_expires INTEGER,
  refresh TEXT,
  used INTEGER NOT NULL
);

-- A share link: anyone holding its token sees the work's live page, view only, until a person
-- with write access revokes it (which deletes the row). The token is kept only as its HMAC with
-- the signing key, so the table alone opens nothing; `id` names the link to revoke it and opens
-- nothing either.
CREATE TABLE shares (
  id TEXT PRIMARY KEY,
  hash TEXT NOT NULL UNIQUE,
  work TEXT NOT NULL REFERENCES works (id),
  made_by TEXT NOT NULL,
  made_at REAL NOT NULL
);
CREATE INDEX shares_by_work ON shares (work, made_at);
