-- Repositories are kept as GitHub's `owner/name`, lowercased, since GitHub ignores case.

-- A proof CI sent: its proof.json, and whether every file of it arrived.
CREATE TABLE proofs (
  id TEXT PRIMARY KEY,
  repo TEXT NOT NULL,
  pull INTEGER,
  proof TEXT NOT NULL,
  done INTEGER NOT NULL DEFAULT 0,
  created INTEGER NOT NULL
);

-- The files of a proof's bundle, by path, and the blob holding each.
CREATE TABLE files (
  proof TEXT NOT NULL REFERENCES proofs (id),
  path TEXT NOT NULL,
  sha256 TEXT NOT NULL,
  size INTEGER NOT NULL,
  PRIMARY KEY (proof, path)
);
CREATE INDEX files_by_blob ON files (sha256);

-- The blobs a repository holds in R2, under blobs/<repo>/<sha256>.
CREATE TABLE blobs (
  repo TEXT NOT NULL,
  sha256 TEXT NOT NULL,
  PRIMARY KEY (repo, sha256)
);

-- The latest whole proof of each pull request.
CREATE TABLE latest (
  repo TEXT NOT NULL,
  pull INTEGER NOT NULL,
  proof TEXT NOT NULL REFERENCES proofs (id),
  PRIMARY KEY (repo, pull)
);
