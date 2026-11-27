-- Each workflow run attempt GitHub's `workflow_run` webhook said completed: the commit it ran on,
-- its workflow file and how it ended. A leg its plan made that opened no run by then never
-- will, so the review stops waiting for it.
CREATE TABLE attempts (
  repo TEXT NOT NULL,
  workflow_run INTEGER NOT NULL,
  run_attempt INTEGER NOT NULL,
  head TEXT NOT NULL,
  path TEXT NOT NULL,
  conclusion TEXT,
  PRIMARY KEY (repo, workflow_run, run_attempt)
);
CREATE INDEX attempts_by_head ON attempts (repo, head);

-- The workflow files a repository proves with: each one a run of which reached the host. One
-- of them ending for a head no run of reached the host ended without its proof.
CREATE TABLE proof_workflows (
  repo TEXT NOT NULL,
  path TEXT NOT NULL,
  PRIMARY KEY (repo, path)
);

-- What the host ran again on GitHub because it never reached the host (`leg <id>`, or
-- `workflow`), once per head, so a host that keeps failing never loops.
CREATE TABLE reruns (
  repo TEXT NOT NULL,
  head TEXT NOT NULL,
  what TEXT NOT NULL,
  at REAL NOT NULL,
  PRIMARY KEY (repo, head, what)
);
