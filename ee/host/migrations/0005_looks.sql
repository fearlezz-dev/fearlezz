-- A check's look: its pictures compared with the base commit (`fearlezz_proof::look::Look` as
-- JSON), from its end or, as the final word, from done.
ALTER TABLE checks ADD COLUMN look TEXT;
