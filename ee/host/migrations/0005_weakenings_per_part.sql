-- A weakening became one per part of the config, with every way it weakened (`hows`) and the
-- part as it was and is now. Proofs, derived reviews and accepts stored in the old shape (one
-- `how` each) no longer read, and only this repository's own CI had stored any, so they go
-- once instead of being read in two shapes forever. A work's next run fills its page again,
-- and an admin accepts again where a weakening still waits. Works, verdicts (kept by a
-- picture's content), pull requests and jobs stay.
DELETE FROM notes;
DELETE FROM files;
DELETE FROM checks;
DELETE FROM accepts;
DELETE FROM runs;
UPDATE works SET review = NULL;
