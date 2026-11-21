Pictures of a made up settings screen in three versions (base, look, now) as stills and VP9
films, with what `fearlezz setup compare --json` answers for them. The CLI's tests check these
answers (`tests/media.rs`); the proof host's page compares the same files in the browser and
must give the same answers (`ee/app/e2e/compare.look.ts`), and its strip of runs shows them.

- `still.json`: `look.png` against `now.png`.
- `film.json`: `look.webm` against `now.webm`.
- `base-still.json`, `base-film.json`: `base` against `now`, as CI would compare them.
