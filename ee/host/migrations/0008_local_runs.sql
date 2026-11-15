-- Local runs, and a CLI sign in that lasts.

-- When a run last said something (opened, a check's start or end, done): a local run silent
-- for an hour is closed as cancelled, as it has no CI job whose end would close it.
ALTER TABLE runs ADD COLUMN touched REAL;
UPDATE runs SET touched = opened;
CREATE INDEX runs_by_state ON runs (state, touched);

-- The CLI's tokens, from `fearlezz setup login`, now the host's own: kept as their HMAC with
-- the signing key, so the table alone opens nothing. Each speaks for a GitHub login, with
-- that person's GitHub token (`token`, until `token_expires` when GitHub sets one) deciding
-- what it may read and write, and the refresh token the host renews it with. A token lives
-- 30 days from its last use (`used`), or until `fearlezz setup logout`. Tokens from before
-- lived as long as their GitHub token, about eight hours, so none is worth keeping.
DROP TABLE cli_tokens;
CREATE TABLE cli_tokens (
  hash TEXT PRIMARY KEY,
  login TEXT NOT NULL,
  token TEXT NOT NULL,
  token_expires INTEGER,
  refresh TEXT,
  used INTEGER NOT NULL
);
