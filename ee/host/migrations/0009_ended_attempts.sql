-- Each attempt of a repository's proof workflow, as GitHub's `workflow_run` webhook says it
-- (requested, in progress, completed), with the commit it runs on. A proof workflow is one a CI
-- run of the repository came from (its `source.workflow`), or the attempt is of a workflow run
-- such a run names. While one is going the review of its head waits; once it completed, a leg
-- its plan made that opened no run never will, and a head no run of reached the host ended
-- without its proof.
CREATE TABLE attempts (
  repo TEXT NOT NULL,
  workflow_run INTEGER NOT NULL,
  run_attempt INTEGER NOT NULL,
  head TEXT NOT NULL,
  path TEXT NOT NULL,
  status TEXT NOT NULL,
  PRIMARY KEY (repo, workflow_run, run_attempt)
);
CREATE INDEX attempts_by_head ON attempts (repo, head);

-- A run of a work by the workflow file it came from, so a `workflow_run` webhook finds whether a
-- workflow is one of the repository's proof workflows among the runs of its works alone. The
-- expression must match the query's own `json_extract(source, '$.workflow')` for D1 to use it.
CREATE INDEX runs_by_workflow ON runs (work, json_extract(source, '$.workflow'));
