-- Notes and the CLI's sign in.

-- A person's note on a picture (`fearlezz_proof::notes::Note`), by check and picture so it is
-- the same picture across the work's runs, made on CI run `run` at `head`. `side` is before,
-- after or both; the region is in fractions of the picture; `time` is the moment of a film.
-- An open note shows on every run of the work until a person resolves it.
CREATE TABLE notes (
  id TEXT PRIMARY KEY,
  work TEXT NOT NULL REFERENCES works (id),
  check_id TEXT NOT NULL,
  picture TEXT NOT NULL,
  run TEXT NOT NULL REFERENCES runs (id),
  head TEXT NOT NULL,
  side TEXT NOT NULL,
  x REAL NOT NULL,
  y REAL NOT NULL,
  w REAL NOT NULL,
  h REAL NOT NULL,
  time REAL,
  words TEXT NOT NULL,
  login TEXT NOT NULL,
  at REAL NOT NULL,
  state TEXT NOT NULL,
  resolved_by TEXT
);
CREATE INDEX notes_by_work ON notes (work, state, at);

-- The CLI's tokens, from `fearlezz setup login`: kept as their HMAC with the signing key, so
-- the table alone opens nothing. Each speaks for a GitHub login with that person's GitHub
-- token, which decides what it may read, until either expires. They read and never write.
CREATE TABLE cli_tokens (
  hash TEXT PRIMARY KEY,
  login TEXT NOT NULL,
  token TEXT NOT NULL,
  expires INTEGER NOT NULL
);
