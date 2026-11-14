# Changelog

## [0.3.0](https://github.com/fearlezz-dev/fearlezz/compare/v0.2.0...v0.3.0) (2026-11-14)


### ⚠ BREAKING CHANGES

* the status `fearlezz/pictures` is now `fearlezz/approval`, and it also waits for an admin's `/fearlezz accept <words>` when a change weakens the proof. Rulesets that require `fearlezz/pictures` must require `fearlezz/approval` instead, and the pictures workflow's comment filter needs `/fearlezz accept`.

### Features

* hosted proof pages, with approve and accept on the page ([#18](https://github.com/fearlezz-dev/fearlezz/issues/18)) ([57c69af](https://github.com/fearlezz-dev/fearlezz/commit/57c69af4a3c327d5cbe4e439ef720ce771ed9bc0))
* park recorded windows off the developer's screens ([#16](https://github.com/fearlezz-dev/fearlezz/issues/16)) ([bd9174c](https://github.com/fearlezz-dev/fearlezz/commit/bd9174c0e96187689b67d201af7b1bbba1f7636f))


### Bug Fixes

* approve and accept on real GitHub: replies out of order, and the second job's weakening ([#21](https://github.com/fearlezz-dev/fearlezz/issues/21)) ([6182f90](https://github.com/fearlezz-dev/fearlezz/commit/6182f909b4aeba75135e55b20d932802956dee92))

## [0.2.0](https://github.com/fearlezz-dev/fearlezz/compare/v0.1.0...v0.2.0) (2026-11-12)


### Features

* make Fearlezz ready for real use in other projects ([#3](https://github.com/fearlezz-dev/fearlezz/issues/3)) ([3dcc000](https://github.com/fearlezz-dev/fearlezz/commit/3dcc00052fb7e9774f96b0c34c164ad004a9d427))
* pictures that prove UI work, locally and on the pull request ([#11](https://github.com/fearlezz-dev/fearlezz/issues/11)) ([fd72240](https://github.com/fearlezz-dev/fearlezz/commit/fd722404c6ee653ca6715f54f9b37bbcb3fdfbbe))
