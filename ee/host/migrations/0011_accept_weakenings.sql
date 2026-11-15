-- An accept covers exactly the weakenings it was given on (`fearlezz_proof::floor::Weakening`
-- as a JSON list), so a CI run of the head passes only when it weakens just those.
ALTER TABLE accepts ADD COLUMN weakened TEXT NOT NULL DEFAULT '[]';
