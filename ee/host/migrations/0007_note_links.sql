-- A note made from another (the same note on another variant of a screen) keeps the first
-- note's id, so an edit of its words can reach every note of the group.
ALTER TABLE notes ADD COLUMN link TEXT;
