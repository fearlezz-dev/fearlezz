-- What a run's CLI found comparing each picture of a check with the pictures a person said
-- something about: by picture name, each one's key and whether it looks the same now.
ALTER TABLE checks ADD COLUMN since TEXT;

-- A verdict follows the picture a person saw, its own file's SHA-256, no longer its before and
-- after (`still:<before>:<after>`) or which of a film's states changed: each verdict's key
-- becomes the file of its picture in the run it was given on. Without that run, a still or
-- text keeps its after, a gone picture becomes `gone:<before>`, and a film keeps its old key,
-- which no picture has, so it waits for a look again.
UPDATE OR REPLACE verdicts SET key = COALESCE(
  (
    SELECT f.sha256
    FROM checks c, json_each(c.look, '$.items') i
    JOIN files f ON f.run = c.run AND f.path = json_extract(i.value, '$.after')
    WHERE c.run = verdicts.run
      AND c.id = verdicts.check_id
      AND json_extract(i.value, '$.name') = verdicts.picture
  ),
  CASE
    WHEN key LIKE '%:-' THEN 'gone:' || substr(key, instr(key, ':') + 1, 64)
    WHEN key LIKE 'still:%' OR key LIKE 'text:%' THEN substr(key, -64)
    ELSE key
  END
);
