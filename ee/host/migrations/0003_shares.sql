-- A share link: anyone holding its random token sees the latest whole proof of one pull
-- request, until a person with write access revokes it.
CREATE TABLE shares (
  token TEXT PRIMARY KEY,
  repo TEXT NOT NULL,
  pull INTEGER NOT NULL,
  made_by TEXT NOT NULL,
  made_at INTEGER NOT NULL,
  revoked_at INTEGER
);
CREATE INDEX shares_by_pull ON shares (repo, pull);
