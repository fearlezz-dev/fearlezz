# ee

Hosted, paid parts of Fearlezz live here, under their own license. Nothing is here yet.

The boundary:

- Everything that runs locally or in CI is the open core in `crates/` and never depends on `ee/`.
- The core talks to hosted parts only through an open, documented format (the proof bundle and its upload calls), so any server can implement the other side.
- Anything needed to make proof is free. Sharing proof with people who were not there, keeping it, and reading it across months is paid.

The design sketch is in `docs/spikes/hosted/README.md`.
