# Changelog

## [0.4.0](https://github.com/fearlezz-dev/fearlezz/compare/v0.3.0...v0.4.0) (2026-11-16)


### ⚠ BREAKING CHANGES

* a check runs in every project folder with each, and the rust piece covers every Cargo project
* the tests of a TypeScript project come with a vitest or bun piece, which setup init picks ([#30](https://github.com/fearlezz-dev/fearlezz/issues/30))
* a framework's Biome rules come with its own piece, and Biome skips what git ignores ([#27](https://github.com/fearlezz-dev/fearlezz/issues/27))

### Features

* a check runs in every project folder with each, and the rust piece covers every Cargo project ([15f01d6](https://github.com/fearlezz-dev/fearlezz/commit/15f01d6ec5959ba599ff4ac07d49ef6ed09bba00))
* a framework's Biome rules come with its own piece, and Biome skips what git ignores ([#27](https://github.com/fearlezz-dev/fearlezz/issues/27)) ([c416483](https://github.com/fearlezz-dev/fearlezz/commit/c41648329fcb48f44084a0fdc91678204b7c4280))
* the tests of a TypeScript project come with a vitest or bun piece, which setup init picks ([#30](https://github.com/fearlezz-dev/fearlezz/issues/30)) ([7e643b8](https://github.com/fearlezz-dev/fearlezz/commit/7e643b81838931eb1637732ec36800b5cda62f7b))

## [0.3.0](https://github.com/fearlezz-dev/fearlezz/compare/v0.2.0...v0.3.0) (2026-11-16)


### ⚠ BREAKING CHANGES

* a weakening shows its change, one per part, on a calmer page ([#25](https://github.com/fearlezz-dev/fearlezz/issues/25))
* a live proof host that owns review, and a free core that only proves ([#23](https://github.com/fearlezz-dev/fearlezz/issues/23))
* the status `fearlezz/pictures` is now `fearlezz/approval`, and it also waits for an admin's `/fearlezz accept <words>` when a change weakens the proof. Rulesets that require `fearlezz/pictures` must require `fearlezz/approval` instead, and the pictures workflow's comment filter needs `/fearlezz accept`.

### Features

* a live proof host that owns review, and a free core that only proves ([#23](https://github.com/fearlezz-dev/fearlezz/issues/23)) ([09fe180](https://github.com/fearlezz-dev/fearlezz/commit/09fe18059e6eb94f728f8e2b014ebd465d34de95))
* a weakening shows its change, one per part, on a calmer page ([#25](https://github.com/fearlezz-dev/fearlezz/issues/25)) ([39b5747](https://github.com/fearlezz-dev/fearlezz/commit/39b574720f5903785ca88e702af62737cfd59a2e))
* hosted proof pages, with approve and accept on the page ([#18](https://github.com/fearlezz-dev/fearlezz/issues/18)) ([57c69af](https://github.com/fearlezz-dev/fearlezz/commit/57c69af4a3c327d5cbe4e439ef720ce771ed9bc0))
* park recorded windows off the developer's screens ([#16](https://github.com/fearlezz-dev/fearlezz/issues/16)) ([bd9174c](https://github.com/fearlezz-dev/fearlezz/commit/bd9174c0e96187689b67d201af7b1bbba1f7636f))


### Bug Fixes

* approve and accept on real GitHub: replies out of order, and the second job's weakening ([#21](https://github.com/fearlezz-dev/fearlezz/issues/21)) ([6182f90](https://github.com/fearlezz-dev/fearlezz/commit/6182f909b4aeba75135e55b20d932802956dee92))

## [0.2.0](https://github.com/fearlezz-dev/fearlezz/compare/v0.1.0...v0.2.0) (2026-11-12)


### Features

* make Fearlezz ready for real use in other projects ([#3](https://github.com/fearlezz-dev/fearlezz/issues/3)) ([3dcc000](https://github.com/fearlezz-dev/fearlezz/commit/3dcc00052fb7e9774f96b0c34c164ad004a9d427))
* pictures that prove UI work, locally and on the pull request ([#11](https://github.com/fearlezz-dev/fearlezz/issues/11)) ([fd72240](https://github.com/fearlezz-dev/fearlezz/commit/fd722404c6ee653ca6715f54f9b37bbcb3fdfbbe))
