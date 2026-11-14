-- A signed in person: the cookie holds only the random id; the GitHub token stays here.
CREATE TABLE sessions (
  id TEXT PRIMARY KEY,
  login TEXT NOT NULL,
  token TEXT NOT NULL,
  expires INTEGER NOT NULL
);

-- GitHub's answers, kept until they expire: who may read a repository, whether it is public,
-- and the Actions OIDC keys.
CREATE TABLE cache (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL,
  expires INTEGER NOT NULL
);
