# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/)
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- use cimg/base:stable + binstall bootstrap in regenerate-orb(pr [#13])

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
[Unreleased]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.5...HEAD
[0.0.5]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.4...v0.0.5
[0.0.4]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.3...v0.0.4
[0.0.3]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.2...v0.0.3
[0.0.2]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.1...v0.0.2
[0.0.1]: https://github.com/jerus-org/gen-circleci-orb/releases/tag/v0.0.1
