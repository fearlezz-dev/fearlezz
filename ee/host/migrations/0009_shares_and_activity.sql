-- Share links, and when each work last changed for the repository page.

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

-- When the work last changed: every write that bumps its `seq` sets it, so the repository page
-- lists works newest activity first.
ALTER TABLE works ADD COLUMN active REAL;
UPDATE works SET active = (SELECT MAX(COALESCE(touched, opened)) FROM runs WHERE runs.work = works.id);
CREATE INDEX works_by_activity ON works (repo, active);
