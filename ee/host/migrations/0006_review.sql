-- Review: verdicts on pictures, accepts of weakenings, what the app wrote on GitHub, and the CI
-- jobs GitHub said are queued.

-- The newest commit of a work (a run's head, or a pull request's head from its webhook), whose
-- review the page and GitHub show, and that review as last derived (`fearlezz_proof::review::
-- Review` as JSON), so a page's snapshot reads it and an unchanged one is not sent again.
ALTER TABLE works ADD COLUMN head TEXT;
ALTER TABLE works ADD COLUMN review TEXT;

-- A person's verdict on a picture, kept by check, picture and content (`review::key`): a picture
-- that did not change keeps it across runs of the work. A later verdict on the same content
-- replaces it. `run` and `head` say where it was given.
CREATE TABLE verdicts (
  work TEXT NOT NULL REFERENCES works (id),
  check_id TEXT NOT NULL,
  picture TEXT NOT NULL,
  key TEXT NOT NULL,
  kind TEXT NOT NULL,
  login TEXT NOT NULL,
  at REAL NOT NULL,
  run TEXT NOT NULL,
  head TEXT NOT NULL,
  PRIMARY KEY (work, check_id, picture, key)
);

-- An admin's accept of every weakening of one head of a work.
CREATE TABLE accepts (
  work TEXT NOT NULL REFERENCES works (id),
  head TEXT NOT NULL,
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
