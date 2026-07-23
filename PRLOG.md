# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/)
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- ci-re-sync orb self-pin to 0.1.2(pr [#212])
- docs-add contributing, governance & templates(pr [#211])
- docs-add security policy & assurance case(pr [#213])
- docs-add CHANGELOG.md and activate generation(pr [#214])
- docs-add roadmap, architecture & root readme(pr [#215])
- chore-strengthen clippy, llvm-cov, cargo-deny(pr [#216])
- chore-add third-party license notices(pr [#217])
- docs-document release signing & fix binstall pubkey(pr [#218])
- docs-add LICENSES dir for badge license detection(pr [#220])
- docs-add OpenSSF criterion-to-evidence answer sheet(pr [#219])

## [0.1.2] - 2026-07-20

### Fixed

- remove orphaned cp + bump self orb pin to 0.1.1(pr [#210])

## [0.1.1] - 2026-07-19

### Changed

- refactor-drop redundant orb-release-binary + verify-orb jobs(pr [#203])
- refactor-emit apt-get install list one package per line (S7020)(pr [#204])

### Fixed

- deps: update rust crate anyhow to 1.0.104(pr [#205])
- deps: update dependency toolkit to v6.6.2(pr [#209])
- deps: update rust crate serde to 1.0.229(pr [#206])
- deps: update rust crate thiserror to 2.0.19(pr [#207])
- deps: update dependency gen-orb-mcp to v0.2.0(pr [#208])
- pin container version + gate on crates.io index lag(pr [#202])

## [0.1.0] - 2026-07-17

### Added

- explicit resource_class on custom orb build jobs(pr [#188])

### Changed

- chore-inherit renovate pin managers from shared config(pr [#197])
- docs-user documentation for v0.1.0(pr [#199])

### Fixed

- unblock CI — yanked spin + stale orb self-pin(pr [#190])
- deps: update dependency toolkit to v6.6.1(pr [#191])
- deps: lock file maintenance(pr [#189])
- deps: update rust crate clap to 4.6.2(pr [#193])
- deps: update rust crate regex to 1.13.1(pr [#196])
- deps: update rust crate tokio to 1.52.4(pr [#194])
- deps: update rust crate toml to 1.1.3(pr [#195])
- deps: update pinned containers(pr [#192])
- deps: update rust crate tokio to 1.53.0(pr [#198])

## [0.0.62] - 2026-07-09

### Fixed

- deps: update dependency gen-circleci-orb to v0.0.61(pr [#185])
- deps: lock file maintenance(pr [#186])
- deps: update dependency gen-orb-mcp to v0.1.51(pr [#187])

## [0.0.61] - 2026-07-08

### Added

- rust_image override for build binary jobs(pr [#184])

### Changed

- chore-bump self-orb pin to 0.0.60 (activates CLI smoke gate)(pr [#180])

### Fixed

- deps: bump crossbeam-epoch to 0.9.20 (RUSTSEC-2026-0204)(pr [#181])
- deps: update dependency toolkit to v6.5.1(pr [#183])
- deps: lock file maintenance(pr [#182])

## [0.0.60] - 2026-07-06

### Fixed

- bump self-orb pin to 0.0.59 (CLI-equipped container)(pr [#178])
- fail release if built container's circleci CLI is broken(pr [#179])

## [0.0.58] - 2026-07-06

### Added

- restore the validation wiring check(pr [#173])

### Fixed

- deps: pin rust docker tag to 31ee7fc(pr [#169])
- deps: update dependency gen-circleci-orb to v0.0.57(pr [#170])
- deps: pin rust docker tag to 31ee7fc(pr [#171])
- pin runtime base image digest in config(pr [#174])
- install circleci CLI intrinsically, not via a deletable flag(pr [#175])
- correct orb existence check (exit 255 = not found)(pr [#176])

## [0.0.57] - 2026-07-03

### Fixed

- self-sufficient container builder stage (deps + --locked)(pr [#168])

## [0.0.56] - 2026-07-03

### Fixed

- never run the wiring check at release (unblock orb publish)(pr [#167])

## [0.0.55] - 2026-07-02

### Added

- interactive subcommand reservation (full exclusion)(pr [#159])

### Changed

- chore-retire build_mcp_server wrapper(pr [#152])
- chore-self-host CI via orb-release workflow(pr [#154])
- chore-suppress the interactive init CI job(pr [#158])

### Fixed

- bump gen-orb-mcp orb default to 0.1.48(pr [#151])
- gen-circleci-orb.toml completeness + orb-release round-trip stability (#155)(pr [#156])
- require [orb] section, warn on missing [record] (config-completeness, #155)(pr [#157])
- deps: update dependency toolkit to v6.5.0(pr [#160])
- deps: update rust:1-slim-trixie docker digest to 31ee7fc(pr [#161])
- deps: update dependency gen-orb-mcp to v0.1.49(pr [#162])
- deps: update rust crate anyhow to 1.0.103(pr [#163])
- deps: update rust crate config to 0.15.25(pr [#164])
- deps: update rust crate console to 0.16.4(pr [#165])
- deps: update rust crate pcu to 0.6.28(pr [#166])

## [0.0.54] - 2026-06-30

### Added

- cede build_mcp_server to gen-orb-mcp orb (Stage 3)(pr [#150])

### Fixed

- deps: bump anyhow to 1.0.103 (RUSTSEC-2026-0190)(pr [#149])

## [0.0.53] - 2026-06-25

### Added

- add 'update' command to re-sync consumer CI wiring(pr [#146])

## [0.0.52] - 2026-06-24

### Added

- generate --check + orb-release verify gate(pr [#145])

### Fixed

- deps: update dependency gen-circleci-orb to v0.0.51(pr [#144])

## [0.0.51] - 2026-06-23

### Changed

- revert-roll back partial 0.0.51 release(pr [#143])

### Fixed

- forward orb boolean flags via when+BASH_ENV(pr [#142])

## [0.0.50] - 2026-06-23

### Added

- ambient-auth record, drop write_token_env(pr [#135])
- Model B workspace carrying for auto-record(pr [#136])
- end-of-workflow push job with optional SSH key(pr [#137])

### Fixed

- stage repo-relative orb pathspec for auto-record(pr [#134])
- deps: bump quinn-proto for RUSTSEC-2026-0185(pr [#141])
- deps: update pinned containers(pr [#138])
- deps: update dependency gen-circleci-orb to v0.0.49(pr [#139])
- deps: update rust crate pcu to 0.6.27(pr [#140])

## [0.0.49] - 2026-06-19

### Added

- config-driven builder image preserves digest(pr [#133])

### Fixed

- persist workspace PATH via BASH_ENV(pr [#132])

## [0.0.48] - 2026-06-18

### Added

- config-driven auto-record ([record] section)(pr [#121])

### Changed

- chore-regenerate orb to released CLI baseline(pr [#119])
- docs-backfill missing PRLOG entries(pr [#129])

### Fixed

- deps: pin dependencies(pr [#107])
- deps: update dependency gen-circleci-orb to v0.0.47(pr [#108])
- deps: update rust crate clap to 4.6.1(pr [#109])
- deps: update rust crate indexmap to 2.14.0(pr [#110])
- deps: update rust crate regex to 1.12.4(pr [#111])
- deps: update rust crate tempfile to 3.27.0(pr [#112])
- deps: update dependency orb-tools to v12.4.0(pr [#113])
- deps: update rust crate console to 0.16.3(pr [#114])
- deps: update rust crate dialoguer to 0.12.0(pr [#115])
- deps: update rust crate toml to v1(pr [#118])
- deps: update dependency docker to v4(pr [#117])
- prune orphaned orb files (#120)(pr [#122])
- deps: pin dependencies(pr [#124])
- deps: update dependency toolkit to v6.4.1(pr [#128])
- deps: update rust crate pcu to 0.6.24(pr [#127])
- sonar UTF-8 encoding + bump toolkit to 6.4.0(pr [#123])
- deps: update rust crate config to 0.15.24(pr [#125])
- deps: update rust crate pcu to 0.6.23(pr [#126])
- deps: update dependency toolkit to v6.4.2(pr [#130])
- deps: update rust crate pcu to 0.6.25(pr [#131])

## [0.0.47] - 2026-06-12

### Added

- descriptive command run-step names(pr [#106])

## [0.0.46] - 2026-06-11

### Fixed

- deps: update dependency toolkit to v6.3.0(pr [#105])
- externalize job_group run scripts to includes(pr [#104])

## [0.0.45] - 2026-06-05

### Added

- composite job_group builder(pr [#103])

## [0.0.44] - 2026-06-04

### Added

- MCP feature auto-provisions executor build toolchain(pr [#102])

## [0.0.43] - 2026-06-04

### Added

- persist apt_packages in [orb] config(pr [#101])

## [0.0.42] - 2026-06-03

### Fixed

- orb: run set_https_remote before git fetch in build_mcp_server(pr [#100])

## [0.0.41] - 2026-06-03

### Changed

- chore-revert errored release prlog marking(pr [#99])

### Fixed

- release: persist binary for build-mcp-server via build_rust_binary(pr [#98])

## [0.0.40] - 2026-06-03

### Fixed

- release: publish crate before building Docker image(pr [#97])

## [0.0.39] - 2026-06-03

### Fixed

- docker: use pre-built binary instead of cargo install in executor image(pr [#95])

## [0.0.38] - 2026-06-02

### Added

- read binary, namespaces, orb_dir from [orb] config(pr [#84])
- auto-detect push-capable subcommands(pr [#85])
- auto-populate orb_path defaults in bootstrap config(pr [#90])
- read install_method and base_image from [orb] config(pr [#92])
- pre-populate dialogue from existing config on re-run(pr [#93])

### Fixed

- ci: support multiple contexts for build_mcp_server(pr [#79])
- init: skip dialogue when stderr is not a TTY(pr [#86])
- init: skip dialogue prompt when field is explicitly set via CLI flag(pr [#91])

## [0.0.37] - 2026-05-29

### Added

- add [ci] section to persist CI patching values(pr [#78])

### Fixed

- render: generate add-workspace-to-path.sh alongside every orb(pr [#76])
- config: persist git_push_subcommands in [orb] section(pr [#77])

## [0.0.36] - 2026-05-29

### Added

- interactive dialogue for CI context names(pr [#75])

### Fixed

- orb generation gaps from regeneration test(pr [#74])

## [0.0.35] - 2026-05-29

### Changed

- chore(ci)-remove redundant toolkit/label from post-merge workflow(pr [#72])

### Fixed

- parser, validation workflow, and init flags(pr [#71])

## [0.0.34] - 2026-05-28

### Added

- add gen-circleci-orb.toml config module (closes #42, #12)(pr [#70])

## [0.0.33] - 2026-05-28

### Changed

- chore(ci)-bump gen-circleci-orb orb pin to 0.0.32 in release.yml(pr [#69])

## [0.0.32] - 2026-05-28

### Added

- add attach_workspace/workspace_root to generated jobs(pr [#68])

## [0.0.31] - 2026-05-27

### Added

- add --apt-packages flag to generate(pr [#67])

## [0.0.30] - 2026-05-27

### Fixed

- replace circleci CLI with GraphQL API in ensure-orb-registered(pr [#65])

## [0.0.29] - 2026-05-26

### Fixed

- orb: authenticate circleci CLI via CIRCLECI_CLI_TOKEN(pr [#64])

## [0.0.28] - 2026-05-26

### Fixed

- ci_patcher: step2 idempotency + replace mcp with build_mcp_server(pr [#63])

## [0.0.27] - 2026-05-25

### Changed

- chore(ci)-bump self-referencing orb pin to 0.0.26(pr [#62])

## [0.0.26] - 2026-05-25

### Fixed

- orb: use --crate-version in build_mcp_server_generate.sh(pr [#61])

## [0.0.25] - 2026-05-23

### Fixed

- use gen-circleci-orb orb for MCP server build(pr [#60])

## [0.0.24] - 2026-05-22

### Added

- add set_https_remote command(pr [#59])

## [0.0.23] - 2026-05-15

### Added

- tag-triggered orb-release workflow in config.yml(pr [#53])
- tag-triggered orb release + --mcp flag(pr [#54])
- move orb-release jobs into the orb (closes #51, #48, #49)(pr [#56])

### Changed

- chore-bump DEFAULT_GEN_ORB_MCP_ORB_VERSION to 0.1.14(pr [#55])
- docs-add version management philosophy(pr [#57])
- docs-update README and getting-started for current architecture(pr [#58])

### Fixed

- use set +e pattern for circleci CLI calls(pr [#52])

## [0.0.22] - 2026-05-13

### Fixed

- handle circleci setup exit 255 without burying genuine errors(pr [#50])

## [0.0.21] - 2026-05-13

### Fixed

- correct default docker orb version to 3.0.1(pr [#46])

## [0.0.19] - 2026-05-13

### Changed

- refactor-remove hardcoded release_crate rewire from patch_release(pr [#45])

## [0.0.18] - 2026-05-12

### Fixed

- make ensure-orb-registered idempotent on already-exists error(pr [#44])

## [0.0.17] - 2026-05-12

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
[#44]: https://github.com/jerus-org/gen-circleci-orb/pull/44
[#45]: https://github.com/jerus-org/gen-circleci-orb/pull/45
[#46]: https://github.com/jerus-org/gen-circleci-orb/pull/46
[#50]: https://github.com/jerus-org/gen-circleci-orb/pull/50
[#52]: https://github.com/jerus-org/gen-circleci-orb/pull/52
[#53]: https://github.com/jerus-org/gen-circleci-orb/pull/53
[#54]: https://github.com/jerus-org/gen-circleci-orb/pull/54
[#55]: https://github.com/jerus-org/gen-circleci-orb/pull/55
[#56]: https://github.com/jerus-org/gen-circleci-orb/pull/56
[#57]: https://github.com/jerus-org/gen-circleci-orb/pull/57
[#58]: https://github.com/jerus-org/gen-circleci-orb/pull/58
[#59]: https://github.com/jerus-org/gen-circleci-orb/pull/59
[#60]: https://github.com/jerus-org/gen-circleci-orb/pull/60
[#61]: https://github.com/jerus-org/gen-circleci-orb/pull/61
[#62]: https://github.com/jerus-org/gen-circleci-orb/pull/62
[#63]: https://github.com/jerus-org/gen-circleci-orb/pull/63
[#64]: https://github.com/jerus-org/gen-circleci-orb/pull/64
[#65]: https://github.com/jerus-org/gen-circleci-orb/pull/65
[#67]: https://github.com/jerus-org/gen-circleci-orb/pull/67
[#68]: https://github.com/jerus-org/gen-circleci-orb/pull/68
[#69]: https://github.com/jerus-org/gen-circleci-orb/pull/69
[#70]: https://github.com/jerus-org/gen-circleci-orb/pull/70
[#71]: https://github.com/jerus-org/gen-circleci-orb/pull/71
[#72]: https://github.com/jerus-org/gen-circleci-orb/pull/72
[#74]: https://github.com/jerus-org/gen-circleci-orb/pull/74
[#75]: https://github.com/jerus-org/gen-circleci-orb/pull/75
[#76]: https://github.com/jerus-org/gen-circleci-orb/pull/76
[#77]: https://github.com/jerus-org/gen-circleci-orb/pull/77
[#78]: https://github.com/jerus-org/gen-circleci-orb/pull/78
[#79]: https://github.com/jerus-org/gen-circleci-orb/pull/79
[#84]: https://github.com/jerus-org/gen-circleci-orb/pull/84
[#85]: https://github.com/jerus-org/gen-circleci-orb/pull/85
[#86]: https://github.com/jerus-org/gen-circleci-orb/pull/86
[#90]: https://github.com/jerus-org/gen-circleci-orb/pull/90
[#91]: https://github.com/jerus-org/gen-circleci-orb/pull/91
[#92]: https://github.com/jerus-org/gen-circleci-orb/pull/92
[#93]: https://github.com/jerus-org/gen-circleci-orb/pull/93
[#95]: https://github.com/jerus-org/gen-circleci-orb/pull/95
[#97]: https://github.com/jerus-org/gen-circleci-orb/pull/97
[#98]: https://github.com/jerus-org/gen-circleci-orb/pull/98
[#99]: https://github.com/jerus-org/gen-circleci-orb/pull/99
[#100]: https://github.com/jerus-org/gen-circleci-orb/pull/100
[#101]: https://github.com/jerus-org/gen-circleci-orb/pull/101
[#102]: https://github.com/jerus-org/gen-circleci-orb/pull/102
[#103]: https://github.com/jerus-org/gen-circleci-orb/pull/103
[#105]: https://github.com/jerus-org/gen-circleci-orb/pull/105
[#104]: https://github.com/jerus-org/gen-circleci-orb/pull/104
[#106]: https://github.com/jerus-org/gen-circleci-orb/pull/106
[#107]: https://github.com/jerus-org/gen-circleci-orb/pull/107
[#108]: https://github.com/jerus-org/gen-circleci-orb/pull/108
[#109]: https://github.com/jerus-org/gen-circleci-orb/pull/109
[#110]: https://github.com/jerus-org/gen-circleci-orb/pull/110
[#111]: https://github.com/jerus-org/gen-circleci-orb/pull/111
[#112]: https://github.com/jerus-org/gen-circleci-orb/pull/112
[#113]: https://github.com/jerus-org/gen-circleci-orb/pull/113
[#114]: https://github.com/jerus-org/gen-circleci-orb/pull/114
[#115]: https://github.com/jerus-org/gen-circleci-orb/pull/115
[#118]: https://github.com/jerus-org/gen-circleci-orb/pull/118
[#117]: https://github.com/jerus-org/gen-circleci-orb/pull/117
[#119]: https://github.com/jerus-org/gen-circleci-orb/pull/119
[#121]: https://github.com/jerus-org/gen-circleci-orb/pull/121
[#122]: https://github.com/jerus-org/gen-circleci-orb/pull/122
[#124]: https://github.com/jerus-org/gen-circleci-orb/pull/124
[#128]: https://github.com/jerus-org/gen-circleci-orb/pull/128
[#127]: https://github.com/jerus-org/gen-circleci-orb/pull/127
[#123]: https://github.com/jerus-org/gen-circleci-orb/pull/123
[#125]: https://github.com/jerus-org/gen-circleci-orb/pull/125
[#126]: https://github.com/jerus-org/gen-circleci-orb/pull/126
[#129]: https://github.com/jerus-org/gen-circleci-orb/pull/129
[#130]: https://github.com/jerus-org/gen-circleci-orb/pull/130
[#131]: https://github.com/jerus-org/gen-circleci-orb/pull/131
[#132]: https://github.com/jerus-org/gen-circleci-orb/pull/132
[#133]: https://github.com/jerus-org/gen-circleci-orb/pull/133
[#134]: https://github.com/jerus-org/gen-circleci-orb/pull/134
[#135]: https://github.com/jerus-org/gen-circleci-orb/pull/135
[#136]: https://github.com/jerus-org/gen-circleci-orb/pull/136
[#137]: https://github.com/jerus-org/gen-circleci-orb/pull/137
[#141]: https://github.com/jerus-org/gen-circleci-orb/pull/141
[#138]: https://github.com/jerus-org/gen-circleci-orb/pull/138
[#139]: https://github.com/jerus-org/gen-circleci-orb/pull/139
[#140]: https://github.com/jerus-org/gen-circleci-orb/pull/140
[#142]: https://github.com/jerus-org/gen-circleci-orb/pull/142
[#143]: https://github.com/jerus-org/gen-circleci-orb/pull/143
[#144]: https://github.com/jerus-org/gen-circleci-orb/pull/144
[#145]: https://github.com/jerus-org/gen-circleci-orb/pull/145
[#146]: https://github.com/jerus-org/gen-circleci-orb/pull/146
[#149]: https://github.com/jerus-org/gen-circleci-orb/pull/149
[#150]: https://github.com/jerus-org/gen-circleci-orb/pull/150
[#151]: https://github.com/jerus-org/gen-circleci-orb/pull/151
[#152]: https://github.com/jerus-org/gen-circleci-orb/pull/152
[#154]: https://github.com/jerus-org/gen-circleci-orb/pull/154
[#156]: https://github.com/jerus-org/gen-circleci-orb/pull/156
[#157]: https://github.com/jerus-org/gen-circleci-orb/pull/157
[#158]: https://github.com/jerus-org/gen-circleci-orb/pull/158
[#159]: https://github.com/jerus-org/gen-circleci-orb/pull/159
[#160]: https://github.com/jerus-org/gen-circleci-orb/pull/160
[#161]: https://github.com/jerus-org/gen-circleci-orb/pull/161
[#162]: https://github.com/jerus-org/gen-circleci-orb/pull/162
[#163]: https://github.com/jerus-org/gen-circleci-orb/pull/163
[#164]: https://github.com/jerus-org/gen-circleci-orb/pull/164
[#165]: https://github.com/jerus-org/gen-circleci-orb/pull/165
[#166]: https://github.com/jerus-org/gen-circleci-orb/pull/166
[#167]: https://github.com/jerus-org/gen-circleci-orb/pull/167
[#168]: https://github.com/jerus-org/gen-circleci-orb/pull/168
[#169]: https://github.com/jerus-org/gen-circleci-orb/pull/169
[#170]: https://github.com/jerus-org/gen-circleci-orb/pull/170
[#171]: https://github.com/jerus-org/gen-circleci-orb/pull/171
[#173]: https://github.com/jerus-org/gen-circleci-orb/pull/173
[#174]: https://github.com/jerus-org/gen-circleci-orb/pull/174
[#175]: https://github.com/jerus-org/gen-circleci-orb/pull/175
[#176]: https://github.com/jerus-org/gen-circleci-orb/pull/176
[#178]: https://github.com/jerus-org/gen-circleci-orb/pull/178
[#179]: https://github.com/jerus-org/gen-circleci-orb/pull/179
[#180]: https://github.com/jerus-org/gen-circleci-orb/pull/180
[#181]: https://github.com/jerus-org/gen-circleci-orb/pull/181
[#183]: https://github.com/jerus-org/gen-circleci-orb/pull/183
[#184]: https://github.com/jerus-org/gen-circleci-orb/pull/184
[#182]: https://github.com/jerus-org/gen-circleci-orb/pull/182
[#185]: https://github.com/jerus-org/gen-circleci-orb/pull/185
[#186]: https://github.com/jerus-org/gen-circleci-orb/pull/186
[#187]: https://github.com/jerus-org/gen-circleci-orb/pull/187
[#190]: https://github.com/jerus-org/gen-circleci-orb/pull/190
[#191]: https://github.com/jerus-org/gen-circleci-orb/pull/191
[#189]: https://github.com/jerus-org/gen-circleci-orb/pull/189
[#193]: https://github.com/jerus-org/gen-circleci-orb/pull/193
[#196]: https://github.com/jerus-org/gen-circleci-orb/pull/196
[#194]: https://github.com/jerus-org/gen-circleci-orb/pull/194
[#195]: https://github.com/jerus-org/gen-circleci-orb/pull/195
[#188]: https://github.com/jerus-org/gen-circleci-orb/pull/188
[#197]: https://github.com/jerus-org/gen-circleci-orb/pull/197
[#192]: https://github.com/jerus-org/gen-circleci-orb/pull/192
[#198]: https://github.com/jerus-org/gen-circleci-orb/pull/198
[#199]: https://github.com/jerus-org/gen-circleci-orb/pull/199
[#203]: https://github.com/jerus-org/gen-circleci-orb/pull/203
[#205]: https://github.com/jerus-org/gen-circleci-orb/pull/205
[#209]: https://github.com/jerus-org/gen-circleci-orb/pull/209
[#206]: https://github.com/jerus-org/gen-circleci-orb/pull/206
[#207]: https://github.com/jerus-org/gen-circleci-orb/pull/207
[#208]: https://github.com/jerus-org/gen-circleci-orb/pull/208
[#202]: https://github.com/jerus-org/gen-circleci-orb/pull/202
[#204]: https://github.com/jerus-org/gen-circleci-orb/pull/204
[#210]: https://github.com/jerus-org/gen-circleci-orb/pull/210
[#212]: https://github.com/jerus-org/gen-circleci-orb/pull/212
[#211]: https://github.com/jerus-org/gen-circleci-orb/pull/211
[#213]: https://github.com/jerus-org/gen-circleci-orb/pull/213
[#214]: https://github.com/jerus-org/gen-circleci-orb/pull/214
[#215]: https://github.com/jerus-org/gen-circleci-orb/pull/215
[#216]: https://github.com/jerus-org/gen-circleci-orb/pull/216
[#217]: https://github.com/jerus-org/gen-circleci-orb/pull/217
[#218]: https://github.com/jerus-org/gen-circleci-orb/pull/218
[#220]: https://github.com/jerus-org/gen-circleci-orb/pull/220
[#219]: https://github.com/jerus-org/gen-circleci-orb/pull/219
[Unreleased]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.1.2...HEAD
[0.1.2]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.62...v0.1.0
[0.0.62]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.61...v0.0.62
[0.0.61]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.60...v0.0.61
[0.0.60]: https://github.com/jerus-org/gen-circleci-orb/compare/v0.0.58...v0.0.60
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
[0.0.1]: https://github.com/jerus-org/gen-circleci-orb/releases/tag/v0.0.1
