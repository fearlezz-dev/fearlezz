-- Needs change is no longer a verdict: an open note on a picture says it. A verdict only says a
-- picture looks right, so the needs change verdicts go and so does the kind.
DELETE FROM verdicts WHERE kind = 'needs_change';
ALTER TABLE verdicts DROP COLUMN kind;
