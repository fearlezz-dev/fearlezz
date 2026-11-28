# Changelog

## [0.5.4](https://github.com/fearlezz-dev/fearlezz/compare/v0.5.3...v0.5.4) (2026-11-28)


### Features

* fearlezz setup show prints what each piece and check sets ([#77](https://github.com/fearlezz-dev/fearlezz/issues/77)) ([9152f4d](https://github.com/fearlezz-dev/fearlezz/commit/9152f4d08943e0b18a5f30139f4e814d47843127))
* the CLI says when a newer Fearlezz is out, and the host refuses ones too old ([#81](https://github.com/fearlezz-dev/fearlezz/issues/81)) ([b3a8d1a](https://github.com/fearlezz-dev/fearlezz/commit/b3a8d1a459b813defc31fec3eba3c704d8b7fff4))


### Bug Fixes

* a helper in an integration test crate may break the lints clippy allows in tests ([#84](https://github.com/fearlezz-dev/fearlezz/issues/84)) ([f62241d](https://github.com/fearlezz-dev/fearlezz/commit/f62241dd861e53d4c60d89491ffd8de1e288c67f))
* ffmpeg refused the film reread's select expression past 100 states ([#82](https://github.com/fearlezz-dev/fearlezz/issues/82)) ([c32c366](https://github.com/fearlezz-dev/fearlezz/commit/c32c366a6460b0aa142d1d0e111e88d72fec276e))
* setup init sees a Flutter app in a folder as Flutter, not as an Xcode app ([#78](https://github.com/fearlezz-dev/fearlezz/issues/78)) ([8324b69](https://github.com/fearlezz-dev/fearlezz/commit/8324b699fdd2acf795bc8f049442ed601a9f47b8))
* the Dart piece turns on no conflicting lints, and dead_code_pub_in_binary spares library test builds ([#79](https://github.com/fearlezz-dev/fearlezz/issues/79)) ([31a0128](https://github.com/fearlezz-dev/fearlezz/commit/31a0128c788ec3e2a266f6411c27f1a09837596f))
* the review check never hangs on a leg that never reached the host ([#85](https://github.com/fearlezz-dev/fearlezz/issues/85)) ([ae6cf53](https://github.com/fearlezz-dev/fearlezz/commit/ae6cf5315474427202a747f8342e42651156cca2))
* the setup guide offers every opportunity, asks before loosening a lint, and signs in itself ([#76](https://github.com/fearlezz-dev/fearlezz/issues/76)) ([b65f222](https://github.com/fearlezz-dev/fearlezz/commit/b65f222e82e79791998866d2ce9f27b447af0959))

## [0.5.3](https://github.com/fearlezz-dev/fearlezz/compare/v0.5.2...v0.5.3) (2026-11-25)


### Bug Fixes

* every film gets an animated WebP copy, however long ([#74](https://github.com/fearlezz-dev/fearlezz/issues/74)) ([ea2bbbf](https://github.com/fearlezz-dev/fearlezz/commit/ea2bbbf8afcccd17a339b793551f8bf70712bbee))

## [0.5.2](https://github.com/fearlezz-dev/fearlezz/compare/v0.5.1...v0.5.2) (2026-11-23)


### Features

* a film is timed from what its check said happened in it ([33db68e](https://github.com/fearlezz-dev/fearlezz/commit/33db68efe4e5e4100ccb728923860fb2581b2198))
* a looks right follows a picture that looks the same as the one approved ([a54d74f](https://github.com/fearlezz-dev/fearlezz/commit/a54d74f1fd4d79f0de05e054d365971c9343490a))
* a minimum Fearlezz version, setup init writes the proof workflow, and opportunities to prove more ([#62](https://github.com/fearlezz-dev/fearlezz/issues/62)) ([4640289](https://github.com/fearlezz-dev/fearlezz/commit/46402894fc9f6610ff1b6d3764a827c3909214c6))
* a setup guide agents follow ([#68](https://github.com/fearlezz-dev/fearlezz/issues/68)) ([0c226c1](https://github.com/fearlezz-dev/fearlezz/commit/0c226c1961453870754a829d979b7f42c6c80792))
* compare pictures in one Rust crate that also builds for wasm ([8c08078](https://github.com/fearlezz-dev/fearlezz/commit/8c08078811d4049d3f9e5360dba893560a8ced43))
* **ee:** compare a picture with any earlier run on a strip of runs ([24e3fdd](https://github.com/fearlezz-dev/fearlezz/commit/24e3fdd9cf4c392e45bcce6d3e7a43cd07b876ad))
* films play aligned at their marks, matched only between the same marks ([7442fdc](https://github.com/fearlezz-dev/fearlezz/commit/7442fdc22af442b97fa00e40f72d17d6d6a106a8))
* setup names an older Fearlezz on PATH, says why no proof page was sent, and hints steps over && ([#61](https://github.com/fearlezz-dev/fearlezz/issues/61)) ([7a691b0](https://github.com/fearlezz-dev/fearlezz/commit/7a691b0ad6811aaa59859a53e6edd455d3a21efa))
* showcase films play as animated WebP ([#70](https://github.com/fearlezz-dev/fearlezz/issues/70)) ([c6a3c4a](https://github.com/fearlezz-dev/fearlezz/commit/c6a3c4a206dbf92eb365bf9845022fb46e1c1bb3))
* showcase pictures from the proof host at /showcase/, for any repository with the app, and setup showcase markup ([#66](https://github.com/fearlezz-dev/fearlezz/issues/66)) ([48073f1](https://github.com/fearlezz-dev/fearlezz/commit/48073f12c480c347a3aea234b94fbc12a17949c9))
* the proof host's review page: notes and looks answer at once, films play and take notes, pictures load ahead, and stills of every state ([2bfb644](https://github.com/fearlezz-dev/fearlezz/commit/2bfb6440933b5ea88d0e8fdbb445f7387f6057ce))


### Bug Fixes

* a film's name keeps its extension, so a still and a film of one screen never share verdicts or notes, and two pictures of one name fail the check ([65a2a8c](https://github.com/fearlezz-dev/fearlezz/commit/65a2a8c6c178ce9ac6868864bfbbfcbaa0ffede1))
* adding a tool with its settings no longer weakens the proof ([#63](https://github.com/fearlezz-dev/fearlezz/issues/63)) ([70aba19](https://github.com/fearlezz-dev/fearlezz/commit/70aba196256b93478d83adfab981601c153be743))
* **ee:** the page's comparison keeps a large film's frames scaled and capped, recovers from a trap, runs only the newest ask and refuses hostile films; notes stay on now's side, and the page needs no wasm-unsafe-eval ([01bb4c0](https://github.com/fearlezz-dev/fearlezz/commit/01bb4c0d7667450de3f296d7f7e62b19082525c8))
* Fearlezz runs the ffmpeg of the project's shell for pictures ([#64](https://github.com/fearlezz-dev/fearlezz/issues/64)) ([d0de8a3](https://github.com/fearlezz-dev/fearlezz/commit/d0de8a3f0b65657d879335d046903f2eaa5417a9))
* key the built in checks on what the binary ships, not its version ([#67](https://github.com/fearlezz-dev/fearlezz/issues/67)) ([e8b2be9](https://github.com/fearlezz-dev/fearlezz/commit/e8b2be9b3d4f3243cc488d4a2f0333baa05108be))
* the pull request title decides red before green and the commits check locally as in CI ([#65](https://github.com/fearlezz-dev/fearlezz/issues/65)) ([db9ac80](https://github.com/fearlezz-dev/fearlezz/commit/db9ac808ae6d99ff63ac9739709ffa39b5ffbf1a))

## [0.5.1](https://github.com/fearlezz-dev/fearlezz/compare/v0.5.0...v0.5.1) (2026-11-19)


### Features

* one command shape, secrets from the environment first and for release places ([#58](https://github.com/fearlezz-dev/fearlezz/issues/58)) ([18d13c1](https://github.com/fearlezz-dev/fearlezz/commit/18d13c13175a374869ca487a58454d0cd5420ac8))


### Bug Fixes

* a failed test's output shows on CI, and a film on a new iOS simulator gets 60 s to start ([#55](https://github.com/fearlezz-dev/fearlezz/issues/55)) ([953d4f3](https://github.com/fearlezz-dev/fearlezz/commit/953d4f3d8f636d7bdba09e927145fac175c2b6c7))

## [0.5.0](https://github.com/fearlezz-dev/fearlezz/compare/v0.4.0...v0.5.0) (2026-11-18)


### ⚠ BREAKING CHANGES

* language pieces for Svelte, Prettier, oxlint, Node tests, Lua, Dart, envtest, go race and commit messages ([#34](https://github.com/fearlezz-dev/fearlezz/issues/34))

### Features

* a check names the secrets it needs, fetched from SecretSpec and masked in its files ([#35](https://github.com/fearlezz-dev/fearlezz/issues/35)) ([e115198](https://github.com/fearlezz-dev/fearlezz/commit/e11519843ac860074edc2fbaf88096fcb9863cc3))
* a check too slow for each change runs on a schedule, and the action can open an issue when it fails ([#39](https://github.com/fearlezz-dev/fearlezz/issues/39)) ([d7a2438](https://github.com/fearlezz-dev/fearlezz/commit/d7a2438682ebe945254a62884a32ac7e102a12be))
* a failed test check names its failed tests, and a test that passes only on a rerun fails unless flaky_tests lists it ([#41](https://github.com/fearlezz-dev/fearlezz/issues/41)) ([a696978](https://github.com/fearlezz-dev/fearlezz/commit/a696978554e002d083410ad500979592c64a879e))
* a machine wide heavy queue, opt in with FEARLEZZ_QUEUE, so many agents on one machine take turns ([#45](https://github.com/fearlezz-dev/fearlezz/issues/45)) ([c79424f](https://github.com/fearlezz-dev/fearlezz/commit/c79424f738fc2470a3aaa6706fe50d40ed02a829))
* a preview deploy becomes proof ([#42](https://github.com/fearlezz-dev/fearlezz/issues/42)) ([9a0452d](https://github.com/fearlezz-dev/fearlezz/commit/9a0452d849e729392c7114e3dc9af2ed9a5555b0))
* a proof splits over several CI jobs, one leg per platform and job, combined by a gate ([#40](https://github.com/fearlezz-dev/fearlezz/issues/40)) ([a2fad9c](https://github.com/fearlezz-dev/fearlezz/commit/a2fad9c166e3fd6d0b35c3bf338c20261b0f2c8a))
* a test check finds each changed line no test ran, as strict as the project's coverage asks ([#32](https://github.com/fearlezz-dev/fearlezz/issues/32)) ([e1b0ee9](https://github.com/fearlezz-dev/fearlezz/commit/e1b0ee9b2e8db1d28503f1f625354007d1b674ec))
* infra pieces (kustomize, helm, flux, terraform, opentofu, nix, yamllint, sops) that setup init adds, and steps in a check's run ([#33](https://github.com/fearlezz-dev/fearlezz/issues/33)) ([1b1c6fe](https://github.com/fearlezz-dev/fearlezz/commit/1b1c6fe1f251e7c6277d109f9d199f4cdf858eeb))
* language pieces for Svelte, Prettier, oxlint, Node tests, Lua, Dart, envtest, go race and commit messages ([#34](https://github.com/fearlezz-dev/fearlezz/issues/34)) ([1ea928a](https://github.com/fearlezz-dev/fearlezz/commit/1ea928aa7e2e0e0f122a0baa17872e59937f53f4))
* record the iOS simulator and a Linux display, not only macOS windows ([#38](https://github.com/fearlezz-dev/fearlezz/issues/38)) ([37e327e](https://github.com/fearlezz-dev/fearlezz/commit/37e327eac78203d3ea6aab7ff9701980e7d3ae7a))
* releases as a layer: places with a dry run, publish and landed, and fearlezz run --release ([#36](https://github.com/fearlezz-dev/fearlezz/issues/36)) ([075b121](https://github.com/fearlezz-dev/fearlezz/commit/075b121722248e49bb02f335b23558f724b5c798))
* texts compared with the base like pictures, and generates proves generated code is up to date ([#37](https://github.com/fearlezz-dev/fearlezz/issues/37)) ([cb953d7](https://github.com/fearlezz-dev/fearlezz/commit/cb953d72ec4c1459f468a8aa9f8ffd2f6e0cd510))

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
