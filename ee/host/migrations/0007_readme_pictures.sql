-- README pictures (`readme.rs`) come only from a CI run of a push to the default branch, and
-- the ref alone does not tell: `pull_request_target`, `workflow_run`, `issue_comment`,
-- `workflow_dispatch` and `schedule` runs carry the default branch's ref too, and the first two
-- often check out a fork's code. So a run keeps the event and the commit its OIDC token names
-- (`event_name`, `sha`); a local run keeps neither. Runs opened before this keep neither and
-- never give a README picture.
ALTER TABLE runs ADD COLUMN event TEXT;
ALTER TABLE runs ADD COLUMN sha TEXT;

-- The runs of a work's commit: when a README asks which push is newest, a commit counts from
-- its first run (the lowest rowid, as `opened` holds whole seconds), so a run of an older push
-- started again later does not make it newer.
CREATE INDEX runs_by_head ON runs (work, head, opened);
