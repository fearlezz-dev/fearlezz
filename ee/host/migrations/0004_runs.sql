-- Protocol version 2: a run streams in while it goes. Nothing real was stored by version 1,
-- so its tables go.
DROP TABLE latest;
DROP TABLE files;
DROP TABLE proofs;
DROP TABLE shares;

-- A piece of work: one branch of one repository (`repo` lowercased, `repository` as GitHub
-- spells it), with the pull request a run named, if any. `seq` counts its events: every write
-- to the work, its runs or their checks bumps it in the same batch.
CREATE TABLE works (
  id TEXT PRIMARY KEY,
  repo TEXT NOT NULL,
  repository TEXT NOT NULL,
  branch TEXT NOT NULL,
  pull INTEGER,
  pull_title TEXT,
  pull_url TEXT,
  seq INTEGER NOT NULL DEFAULT 0,
  UNIQUE (repo, branch)
);
CREATE INDEX works_by_pull ON works (repo, pull);

-- One run of a work. `git_ref` is the ref of the OIDC token that opened it: only tokens for the
-- same ref write it. `source` is its JSON (`events::Source`); `state` is running or done (a
-- run that never says done stays running until runs are closed from GitHub's events);
-- `proof` is its proof.json once done.
CREATE TABLE runs (
  id TEXT PRIMARY KEY,
  work TEXT NOT NULL REFERENCES works (id),
  git_ref TEXT NOT NULL,
  head TEXT NOT NULL,
  base TEXT NOT NULL,
  source TEXT NOT NULL,
  state TEXT NOT NULL,
  opened REAL NOT NULL,
  proof TEXT
);
CREATE INDEX runs_by_work ON runs (work, opened);

-- Each check of a run, in its order: planned when the run opened, `started` once it starts,
-- `entry` (its `fearlezz_proof::Check` as JSON) once it ends.
CREATE TABLE checks (
  run TEXT NOT NULL REFERENCES runs (id),
  id TEXT NOT NULL,
  position INTEGER NOT NULL,
  started REAL,
  entry TEXT,
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
