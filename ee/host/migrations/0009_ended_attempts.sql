-- Each workflow run attempt GitHub's `workflow_run` webhook said completed, with the commit it
-- ran on: a leg its plan made that opened no run by then never will, so the review stops
-- waiting for it.
CREATE TABLE attempts (
  repo TEXT NOT NULL,
  workflow_run INTEGER NOT NULL,
  run_attempt INTEGER NOT NULL,
  head TEXT NOT NULL,
  PRIMARY KEY (repo, workflow_run, run_attempt)
);
CREATE INDEX attempts_by_head ON attempts (repo, head);
