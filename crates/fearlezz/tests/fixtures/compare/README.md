Pictures of a made up settings screen in three versions (base, look, now) as stills and VP9
films, with what `fearlezz setup compare --json` answers for them. The CLI's tests check these
answers (`tests/media.rs`); the proof host's page compares the same files in the browser and
must give the same answers (`ee/app/e2e/compare.look.ts`), and its strip of runs shows them.

- `still.json`: `look.png` against `now.png`.
- `film.json`: `look.webm` against `now.webm`.
- `base-still.json`, `base-film.json`: `base` against `now`, as CI would compare them.

Stills of the proof host's own page from its CI runs, which `tests/media.rs` checks:

- `skeleton-before.png`, `skeleton-after.png`: the page while it connects, its faint loading
  skeleton moved 12px down: changed.
- `jitter-before.png`, `jitter-after.png`: one phone still of the same code from two runs,
  an icon drawn a fraction of a pixel apart: the same.

