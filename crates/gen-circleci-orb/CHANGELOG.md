<!-- LTex: Enabled=false -->
# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/) and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

Summary: 

## [0.1.2] - 2026-07-20

Summary: Chore[1]

## [0.1.1] - 2026-07-19

Summary: Changed[2], Chore[1], Fixed[4]

### Fixed

 - fix(deps): update rust crate thiserror to 2.0.19
 - fix(deps): update rust crate serde to 1.0.229
 - fix(deps): update rust crate anyhow to 1.0.104
 - fix: pin container version + gate on index lag

### Changed

 - refactor: emit apt-get install one pkg per line
 - refactor: drop orb-release-binary + verify-orb

## [0.1.0] - 2026-07-17

Summary: Chore[1], Documentation[2], Fixed[5]

### Fixed

 - fix(deps): update rust crate tokio to 1.53.0
 - fix(deps): update rust crate regex to 1.13.1
 - fix(deps): update rust crate toml to 1.1.3
 - fix(deps): update rust crate tokio to 1.52.4
 - fix(deps): update rust crate clap to 4.6.2

## [0.0.62] - 2026-07-09

Summary: Chore[1]

## [0.0.61] - 2026-07-08

Summary: Added[1], Chore[1]

### Added

 - feat: rust_image override for build binary jobs

## [0.0.60] - 2026-07-06

Summary: Chore[1]

## [0.0.59] - 2026-07-06

Summary: Chore[1]

## [0.0.58] - 2026-07-06

Summary: Added[1], Chore[1], Fixed[2]

### Added

 - feat: restore the validation wiring check

### Fixed

 - fix: correct orb existence check (exit 255 = not found)
 - fix: install circleci CLI intrinsically, not via a deletable flag

## [0.0.57] - 2026-07-03

Summary: Chore[1], Fixed[1]

### Fixed

 - fix: self-sufficient container builder stage

## [0.0.56] - 2026-07-03

Summary: Chore[1], Fixed[1]

### Fixed

 - fix: never run the wiring check at release

## [0.0.55] - 2026-07-02

Summary: Added[1], Chore[1], Fixed[7]

### Added

 - feat: interactive subcommand reservation

### Fixed

 - fix(deps): update rust crate pcu to 0.6.28
 - fix(deps): update rust crate console to 0.16.4
 - fix(deps): update rust crate config to 0.15.25
 - fix(deps): update rust crate anyhow to 1.0.103
 - fix: require [orb], warn on missing [record]
 - fix: config completeness + round-trip stability
 - fix: bump gen-orb-mcp orb default to 0.1.48

## [0.0.54] - 2026-06-30

Summary: Added[1], Chore[1]

### Added

 - feat: cede build_mcp_server to gen-orb-mcp orb

## [0.0.53] - 2026-06-25

Summary: Added[5], Chore[1], Documentation[1]

### Added

 - feat: content-matched managed-block strip
 - feat: orb generate job runs update --check (drift alert)
 - feat: add 'update' command to re-sync CI wiring
 - feat: ci_patcher resync mode (replace managed blocks)
 - feat: wrap orb-managed config blocks in markers

## [0.0.52] - 2026-06-24

Summary: Added[1], Changed[1], Chore[1], Fixed[1]

### Added

 - feat: generate --check + orb-release verify gate

### Fixed

 - fix: drop ci-skip marker from regenerate-orb commit

### Changed

 - refactor: push regen early, cancel redundant run

## [0.0.51] - 2026-06-23

Summary: Chore[2], Fixed[1]

### Fixed

 - fix: forward orb boolean flags via when+BASH_ENV

## [0.0.50] - 2026-06-23

Summary: Added[3], Changed[1], Chore[1], Fixed[2]

### Added

 - feat: end-of-workflow push job with optional SSH key
 - feat: Model B workspace carrying for auto-record
 - feat: ambient-auth record, drop write_token_env

### Fixed

 - fix(deps): update rust crate pcu to 0.6.27
 - fix: stage repo-relative orb pathspec for auto-record

### Changed

 - refactor: collapse own regenerate/push to one job; image gets ssh tooling

## [0.0.49] - 2026-06-19

Summary: Added[1], Chore[1], Fixed[1]

### Added

 - feat: config-driven builder image preserves digest

### Fixed

 - fix: persist workspace PATH via BASH_ENV

## [0.0.48] - 2026-06-18

Summary: Added[3], Chore[1], Fixed[14]

### Added

 - feat: config-driven auto-record
 - feat: generate --record auto-commits regenerated orb
 - feat: suppress config subcommands from orb job generation

### Fixed

 - fix(deps): update rust crate pcu to 0.6.25
 - fix(deps): update rust crate pcu to 0.6.24
 - fix(deps): update rust crate config to 0.15.24
 - fix(deps): update rust crate pcu to 0.6.23
 - fix: prune orphaned orb files (#120)
 - fix: RC010 param keys + run sonarcloud on main
 - fix: enum orb param defaults to first value, not empty
 - fix(deps): update rust crate toml to v1
 - fix(deps): update rust crate dialoguer to 0.12.0
 - fix(deps): update rust crate console to 0.16.3
 - fix(deps): update rust crate tempfile to 3.27.0
 - fix(deps): update rust crate regex to 1.12.4
 - fix(deps): update rust crate indexmap to 2.14.0
 - fix(deps): update rust crate clap to 4.6.1

## [0.0.47] - 2026-06-12

Summary: Added[1], Chore[1]

### Added

 - feat: descriptive command run-step names

## [0.0.46] - 2026-06-11

Summary: Chore[1], Fixed[1]

### Fixed

 - fix: externalize job_group run scripts to includes

## [0.0.45] - 2026-06-05

Summary: Added[1], Chore[1]

### Added

 - feat: composite job_group builder

## [0.0.44] - 2026-06-04

Summary: Added[1], Chore[1]

### Added

 - feat(generate): MCP feature auto-provisions executor build toolchain

## [0.0.43] - 2026-06-04

Summary: Added[1], Chore[1]

### Added

 - feat(generate): persist apt_packages in [orb] config

## [0.0.42] - 2026-06-03

Summary: Chore[1]

## [0.0.41] - 2026-06-03

Summary: Chore[1]

## [0.0.40] - 2026-06-03

Summary: Chore[2]

## [0.0.39] - 2026-06-03

Summary: Added[1], Chore[1]

### Added

 - feat(generate): add InstallMethod::Local for pre-built binary in Docker context

## [0.0.38] - 2026-06-02

Summary: Added[5], Chore[1], Fixed[5]

### Added

 - feat(init): pre-populate dialogue from existing config on re-run
 - feat(generate): read install_method and base_image from [orb] config
 - feat(init): auto-populate orb_path defaults in bootstrap config (closes #83)
 - feat(init): auto-detect push-capable subcommands (closes #81)
 - feat(generate): read binary, namespaces, orb_dir from [orb] config

### Fixed

 - fix(init): skip dialogue prompt when field is explicitly set via CLI flag
 - fix(clippy): use or_default() instead of or_insert_with(Default::default)
 - fix(test): make TTY test conditional on actual terminal state
 - fix(init): skip dialogue when stderr is not a TTY (closes #82)
 - fix(ci): support multiple contexts for build_mcp_server

## [0.0.37] - 2026-05-29

Summary: Added[1], Chore[1], Fixed[2]

### Added

 - feat(config): add [ci] section to persist CI patching values

### Fixed

 - fix(config): persist git_push_subcommands in [orb] section
 - fix(render): generate add-workspace-to-path.sh alongside every orb

## [0.0.36] - 2026-05-29

Summary: Added[1], Chore[1], Fixed[1]

### Added

 - feat(init): interactive dialogue for CI context names

### Fixed

 - fix: orb generation gaps found during regeneration test

## [0.0.35] - 2026-05-29

Summary: Chore[1], Fixed[1]

### Fixed

 - fix: parser, validation workflow, and init flags

## [0.0.34] - 2026-05-28

Summary: Added[7], Changed[1], Chore[1], Fixed[2]

### Added

 - feat(config): add config subcommand for TOML file management
 - feat(init): write bootstrap gen-circleci-orb.toml after generation
 - feat(generate): add --config flag and auto-discover config file
 - feat(generator): add extra_job verbatim YAML generation
 - feat(generator): add job_group composed job generation
 - feat(generator): add config-driven suppression, param overrides, orbs
 - feat(config): add orb_config module with load/save and TOML types

### Fixed

 - fix(config): remove unused SubcommandConfig import
 - fix(config): use or_default() instead of or_insert_with(T::default)

### Changed

 - refactor(render): extract helpers to reduce cognitive complexity

## [0.0.33] - 2026-05-28

Summary: Chore[1]

## [0.0.32] - 2026-05-28

Summary: Added[1], Chore[1]

### Added

 - feat: add attach_workspace/workspace_root to generated jobs

## [0.0.31] - 2026-05-27

Summary: Added[1], Chore[1]

### Added

 - feat: add --apt-packages flag to generate

## [0.0.30] - 2026-05-27

Summary: Added[1], Chore[1], Fixed[6]

### Added

 - feat(generate): add --circleci-cli-version flag

### Fixed

 - fix(orb): snake_case all component filenames (RC010)
 - fix(orb): include required params in generated example
 - fix: enforce HTTPS-only for curl in cli-installer
 - fix(generator): use [[ ]] and regenerate orb scripts
 - fix: use orb-tools executor for ensure-orb-registered job
 - fix: replace circleci CLI with GraphQL API in ensure-orb-registered

## [0.0.29] - 2026-05-26

Summary: Added[1], Chore[1]

### Added

 - feat: add ensure-orb-registered CLI subcommand

## [0.0.28] - 2026-05-26

Summary: Chore[1], Fixed[1]

### Fixed

 - fix(ci_patcher): step2 idempotency + replace mcp with build_mcp_server

## [0.0.27] - 2026-05-25

Summary: Chore[1]

## [0.0.26] - 2026-05-25

Summary: Chore[1]

## [0.0.25] - 2026-05-23

Summary: Chore[1]

## [0.0.24] - 2026-05-22

Summary: Added[1], Chore[1]

### Added

 - feat: add git-push-subcommand flag to generate

## [0.0.23] - 2026-05-15

Summary: Added[3], Changed[2], Chore[2], Documentation[1], Fixed[1]

### Added

 - feat: move orb-release jobs into the orb (#51)
 - feat: implement --mcp flag in init command
 - feat: tag-triggered orb-release workflow in config.yml

### Fixed

 - fix: use set +e pattern for circleci CLI calls

### Changed

 - refactor: reduce patch_build cognitive complexity
 - refactor: extract push_tag_filters helper

## [0.0.22] - 2026-05-13

Summary: Chore[1], Fixed[1]

### Fixed

 - fix: handle circleci setup exit 255 without burying genuine errors

## [0.0.21] - 2026-05-13

Summary: Chore[1], Fixed[1]

### Fixed

 - fix: correct default docker orb version to 3.0.1

## [0.0.19] - 2026-05-13

Summary: Added[1], Changed[1], Chore[1]

### Added

 - feat: make --release-after-job required in init

### Changed

 - refactor: remove hardcoded release_crate rewire from patch_release

## [0.0.18] - 2026-05-12

Summary: Chore[1], Fixed[1]

### Fixed

 - fix: make ensure-orb-registered idempotent on already-exists error

## [0.0.17] - 2026-05-12

Summary: Chore[1], Fixed[2]

### Fixed

 - fix: normalise binary path to stem in CliDefinition
 - fix(generator): emit env var patterns in generated scripts

## [0.0.16] - 2026-05-07

Summary: Chore[1]

## [0.0.15] - 2026-05-07

Summary: Chore[1]

## [0.0.14] - 2026-05-07

Summary: Chore[1]

## [0.0.13] - 2026-05-06

Summary: Chore[1]

## [0.0.12] - 2026-05-06

Summary: Chore[1], Fixed[2]

### Fixed

 - fix: rename restricted command params with subcommand prefix
 - fix: skip restricted command params; sort apt packages

## [0.0.11] - 2026-05-05

Summary: Added[1], Chore[1], Fixed[2]

### Added

 - feat(generate): auto-detect source_url from git remote origin

### Fixed

 - fix(dockerfile): add circleci user and project workdir
 - fix(dockerfile): use multi-stage build to eliminate curl|bash and GLIBC mismatch

## [0.0.10] - 2026-05-05

Summary: Added[4], Changed[2], Chore[1], Fixed[1]

### Added

 - feat(init): per-namespace orb visibility control
 - feat(init): add --private flag for orb visibility
 - feat(ci_patcher): support multiple orb namespaces
 - feat: rewire release_crate to require orb publish

### Fixed

 - fix(ci): update regenerate-orb to use --orb-namespace; genericise docs

### Changed

 - refactor(generate): rename --namespace to --orb-namespace
 - refactor(init): replace --namespace/--private-namespace with explicit --public-orb-namespace / --private-orb-namespace flags

## [0.0.9] - 2026-05-05

Summary: Chore[1], Fixed[12]

### Fixed

 - fix: run circleci setup before orb info/create
 - fix: use CIRCLE_TOKEN not CIRCLECI_TOKEN for CLI auth
 - fix: export CIRCLECI_CLI_TOKEN from CIRCLECI_TOKEN
 - fix: drop CIRCLECI_API_TOKEN export in ensure-orb job
 - fix: use separate job for orb registration
 - fix: ensure orb is registered before publishing
 - fix: add v prefix to CIRCLE_TAG for orb-tools/publish
 - fix(ci_patcher): correct release chain ordering
 - fix: use docker context (not docker-hub)
 - fix: use docker-hub context for Docker push
 - fix: use DOCKERHUB_USERNAME/PASSWORD with --password-stdin
 - fix: derive Docker version from git tags, not CIRCLE_TAG

## [0.0.8] - 2026-05-01

Summary: Added[1], Chore[1], Fixed[7], Testing[1]

### Added

 - feat: self-generate orb and fix help parser bugs

### Fixed

 - fix: add vcs_type: github to orb-tools/publish
 - fix: correct circleci/docker orb version to 3.0.1
 - fix: orb-tools review compliance (RC001/003/006/009)
 - fix: add git to Dockerfile; use rust:latest for bootstrap
 - fix: create jobs section in release.yml when missing; wire own CI
 - fix: use rust:latest and gen-circleci-orb image in generated CI
 - fix: bootstrap cargo-binstall before installing gen-circleci-orb

## [0.0.7] - 2026-05-01

Summary: Chore[1], Fixed[1]

### Fixed

 - fix: use cargo-binstall binary in regenerate-orb job

## [0.0.6] - 2026-05-01

Summary: Changed[1], Chore[1], Fixed[4]

### Fixed

 - fix: skip reserved CircleCI job parameter names
 - fix: use debian:12-slim for regenerate-orb job
 - fix: export cargo bin to PATH after binstall bootstrap
 - fix: use cimg/base:stable + binstall bootstrap in regenerate-orb

### Changed

 - refactor: build binary from source for orb regeneration

## [0.0.5] - 2026-04-30

Summary: Chore[1], Documentation[1], Fixed[1]

### Fixed

 - fix(parser): don't exclude --version flags with a <VALUE> metavar

## [0.0.4] - 2026-04-29

Summary: Chore[1], Fixed[2]

### Fixed

 - fix(render): supply defaults for optional params without CLI default
 - fix(parser): use Usage line to detect required params

## [0.0.3] - 2026-04-29

Summary: Changed[1], Chore[1], Fixed[1]

### Fixed

 - fix(dockerfile): use debian:12-slim and bootstrap cargo-binstall correctly

### Changed

 - refactor: extract DEFAULT_BASE_IMAGE constant to single definition

## [0.0.2] - 2026-04-28

Summary: Added[1], Chore[1], Documentation[1]

### Added

 - feat(init): add --docker-namespace for container image registry org

## [0.0.1] - 2026-04-28

Summary: Added[2], Chore[1], Fixed[2], Testing[1]

### Added

 - feat: isolate orb source in dedicated subdirectory
 - feat: implement gen-circleci-orb MVP (generate + init)

### Fixed

 - fix(ci_patcher): use correct orb-tools@12 API (snake_case params, review job)
 - fix(test): update integration test paths for orb subdirectory

## [0.0.0] - 2026-04-16

Summary: Chore[2]

[Unreleased]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.1.2...HEAD
[0.1.2]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.62...v0.1.0
[0.0.62]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.61...v0.0.62
[0.0.61]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.60...v0.0.61
[0.0.60]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.59...v0.0.60
[0.0.59]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.58...v0.0.59
[0.0.58]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.57...v0.0.58
[0.0.57]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.56...v0.0.57
[0.0.56]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.55...v0.0.56
[0.0.55]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.54...v0.0.55
[0.0.54]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.53...v0.0.54
[0.0.53]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.52...v0.0.53
[0.0.52]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.51...v0.0.52
[0.0.51]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.50...v0.0.51
[0.0.50]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.49...v0.0.50
[0.0.49]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.48...v0.0.49
[0.0.48]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.47...v0.0.48
[0.0.47]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.46...v0.0.47
[0.0.46]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.45...v0.0.46
[0.0.45]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.44...v0.0.45
[0.0.44]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.43...v0.0.44
[0.0.43]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.42...v0.0.43
[0.0.42]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.41...v0.0.42
[0.0.41]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.40...v0.0.41
[0.0.40]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.39...v0.0.40
[0.0.39]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.38...v0.0.39
[0.0.38]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.37...v0.0.38
[0.0.37]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.36...v0.0.37
[0.0.36]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.35...v0.0.36
[0.0.35]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.34...v0.0.35
[0.0.34]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.33...v0.0.34
[0.0.33]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.32...v0.0.33
[0.0.32]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.31...v0.0.32
[0.0.31]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.30...v0.0.31
[0.0.30]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.29...v0.0.30
[0.0.29]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.28...v0.0.29
[0.0.28]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.27...v0.0.28
[0.0.27]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.26...v0.0.27
[0.0.26]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.25...v0.0.26
[0.0.25]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.24...v0.0.25
[0.0.24]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.23...v0.0.24
[0.0.23]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.22...v0.0.23
[0.0.22]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.21...v0.0.22
[0.0.21]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.19...v0.0.21
[0.0.19]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.18...v0.0.19
[0.0.18]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.17...v0.0.18
[0.0.17]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.16...v0.0.17
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
[0.0.1]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.0...v0.0.1
[0.0.0]: https://github.com/jerus-org/gen-circleci-orb/releases/tag/v0.0.0

