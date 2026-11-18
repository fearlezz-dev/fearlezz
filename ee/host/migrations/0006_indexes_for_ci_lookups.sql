-- Indexes for the lookups that join a run to its GitHub job by the JSON in `runs.source`. Without
-- them each lookup read every run, or every job, of the repository, so the rows D1 reads grew
-- with the repository's history.
--
-- A run by its workflow run (webhook: the run a finished job belongs to). The expression must
-- match the queries' own `json_extract(source, '$.run_id')` for D1 to use it.
CREATE INDEX runs_by_workflow_run ON runs (json_extract(source, '$.run_id'));

-- A run by its job id (the jobs a work's page shows waiting for a runner).
CREATE INDEX runs_by_job ON runs (json_extract(source, '$.job'));

-- A job by the workflow run and attempt it belongs to (the leg of each run of a work).
CREATE INDEX jobs_by_run ON jobs (repo, workflow_run, run_attempt);
