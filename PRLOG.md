# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/)
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- generator: emit env var patterns in generated scripts(pr [#41])
- normalise binary path to stem in CliDefinition(pr [#43])

## [0.0.16] - 2026-05-07

### Fixed

- orb: use env vars in generate and init scripts(pr [#40])

## [0.0.15] - 2026-05-07

### Fixed

- orb: pass parameters to scripts via environment vars(pr [#39])

## [0.0.14] - 2026-05-07

### Added

- add build_rust_binary job and workspace support for generate job(pr [#38])

## [0.0.13] - 2026-05-06

### Added

- add build_mcp_server to release pipeline(pr [#37])

## [0.0.12] - 2026-05-06

### Fixed

- skip restricted command params; sort apt packages(pr [#36])

## [0.0.11] - 2026-05-05

### Fixed

- dockerfile: multi-stage build eliminates curl|bash and GLIBC mismatch(pr [#35])

## [0.0.10] - 2026-05-05

### Added

- rewire release_crate to require orb publish(pr [#33])
- add --private flag for orb visibility(pr [#34])

## [0.0.9] - 2026-05-05

### Fixed

- derive Docker version from git tags, not CIRCLE_TAG(pr [#21])
- Docker image and orb before crates.io(pr [#22])
- use DOCKERHUB_USERNAME/PASSWORD with --password-stdin(pr [#23])
- use docker-hub context for Docker push(pr [#24])
- inject CIRCLE_TAG for orb-tools/publish in merge pipeline(pr [#25])
- ci_patcher: correct release chain ordering(pr [#27])
- add v prefix to CIRCLE_TAG for orb-tools/publish(pr [#28])
- ensure orb is registered before publishing(pr [#29])
- use separate job for orb registration(pr [#30])
- drop CIRCLECI_API_TOKEN export in ensure-orb job(pr [#31])
- export CIRCLECI_CLI_TOKEN from CIRCLECI_TOKEN(pr [#32])

## [0.0.8] - 2026-05-01

### Fixed

- bootstrap cargo-binstall before installing gen-circleci-orb(pr [#18])
- correct circleci/docker orb version to 3.0.1(pr [#19])
- add vcs_type: github to orb-tools/publish(pr [#20])

## [0.0.7] - 2026-05-01

### Fixed

- use cargo-binstall binary in regenerate-orb job(pr [#17])

## [0.0.6] - 2026-05-01

### Fixed

- use cimg/base:stable + binstall bootstrap in regenerate-orb(pr [#13])
- export ~/.cargo/bin to PATH after binstall bootstrap(pr [#14])
- use ubuntu:24.04 for regenerate-orb (GLIBC 2.39)(pr [#15])
- skip reserved CircleCI job parameter names(pr [#16])

## [0.0.5] - 2026-04-30

### Fixed

- parser: don't exclude --version flags with a <VALUE> metavar(pr [#11])

## [0.0.4] - 2026-04-29

### Fixed

- parser: use Usage line to detect required params(pr [#10])

## [0.0.3] - 2026-04-29

### Fixed

- dockerfile: use debian:12-slim and bootstrap cargo-binstall correctly(pr [#9])

## [0.0.2] - 2026-04-28

### Added

- add --docker-namespace for container image registry org(pr [#8])

## [0.0.1] - 2026-04-28

### Added

- implement gen-circleci-orb MVP (generate + init)(pr [#7])

### Changed

- chore-init standard Rust workspace framework(pr [#2])
- chore-set initial crate version to 0.0.0(pr [#5])
- docs-add initial design document(pr [#4])

[#2]: https://github.com/jerus-org/gen-circleci-orb/pull/2
[#5]: https://github.com/jerus-org/gen-circleci-orb/pull/5
[#4]: https://github.com/jerus-org/gen-circleci-orb/pull/4
[#7]: https://github.com/jerus-org/gen-circleci-orb/pull/7
[#8]: https://github.com/jerus-org/gen-circleci-orb/pull/8
[#9]: https://github.com/jerus-org/gen-circleci-orb/pull/9
[#10]: https://github.com/jerus-org/gen-circleci-orb/pull/10
[#11]: https://github.com/jerus-org/gen-circleci-orb/pull/11
[#13]: https://github.com/jerus-org/gen-circleci-orb/pull/13
[#14]: https://github.com/jerus-org/gen-circleci-orb/pull/14
[#15]: https://github.com/jerus-org/gen-circleci-orb/pull/15
[#16]: https://github.com/jerus-org/gen-circleci-orb/pull/16
[#17]: https://github.com/jerus-org/gen-circleci-orb/pull/17
[#18]: https://github.com/jerus-org/gen-circleci-orb/pull/18
[#19]: https://github.com/jerus-org/gen-circleci-orb/pull/19
[#20]: https://github.com/jerus-org/gen-circleci-orb/pull/20
[#21]: https://github.com/jerus-org/gen-circleci-orb/pull/21
[#22]: https://github.com/jerus-org/gen-circleci-orb/pull/22
[#23]: https://github.com/jerus-org/gen-circleci-orb/pull/23
[#24]: https://github.com/jerus-org/gen-circleci-orb/pull/24
[#25]: https://github.com/jerus-org/gen-circleci-orb/pull/25
[#27]: https://github.com/jerus-org/gen-circleci-orb/pull/27
[#28]: https://github.com/jerus-org/gen-circleci-orb/pull/28
[#29]: https://github.com/jerus-org/gen-circleci-orb/pull/29
[#30]: https://github.com/jerus-org/gen-circleci-orb/pull/30
[#31]: https://github.com/jerus-org/gen-circleci-orb/pull/31
[#32]: https://github.com/jerus-org/gen-circleci-orb/pull/32
[#33]: https://github.com/jerus-org/gen-circleci-orb/pull/33
[#34]: https://github.com/jerus-org/gen-circleci-orb/pull/34
[#35]: https://github.com/jerus-org/gen-circleci-orb/pull/35
[#36]: https://github.com/jerus-org/gen-circleci-orb/pull/36
[#37]: https://github.com/jerus-org/gen-circleci-orb/pull/37
[#38]: https://github.com/jerus-org/gen-circleci-orb/pull/38
[#39]: https://github.com/jerus-org/gen-circleci-orb/pull/39
[#40]: https://github.com/jerus-org/gen-circleci-orb/pull/40
[#41]: https://github.com/jerus-org/gen-circleci-orb/pull/41
[#43]: https://github.com/jerus-org/gen-circleci-orb/pull/43
[Unreleased]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.16...HEAD
[0.0.16]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.15...v0.0.16
[0.0.15]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.14...v0.0.15
[0.0.14]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.13...v0.0.14
[0.0.13]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.12...v0.0.13
[0.0.12]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.11...v0.0.12
[0.0.11]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.10...v0.0.11
[0.0.10]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.9...v0.0.10
[0.0.9]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.8...v0.0.9
[0.0.8]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.7...v0.0.8
[0.0.7]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.6...v0.0.7
[0.0.6]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.5...v0.0.6
[0.0.5]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.4...v0.0.5
[0.0.4]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.3...v0.0.4
[0.0.3]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.2...v0.0.3
[0.0.2]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.1...v0.0.2
[0.0.1]: https://github.com/jerus-org/gen-circleci-orb/releases/tag/v0.0.1
