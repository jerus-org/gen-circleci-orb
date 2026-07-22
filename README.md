<!--
SPDX-FileCopyrightText: 2026 jerusdp

SPDX-License-Identifier: MIT OR Apache-2.0
-->

# gen-circleci-orb

Generate a CircleCI orb to provide the facilities offered by a CLI program.

[![Crates.io](https://img.shields.io/crates/v/gen-circleci-orb.svg)](https://crates.io/crates/gen-circleci-orb)
[![Documentation](https://docs.rs/gen-circleci-orb/badge.svg)](https://docs.rs/gen-circleci-orb)
[![License](https://img.shields.io/crates/l/gen-circleci-orb.svg)](#license)
[![OpenSSF Best Practices](https://www.bestpractices.dev/projects/13667/badge)](https://www.bestpractices.dev/projects/13667)

## Overview

**gen-circleci-orb** reads the `--help` output of a Rust [clap](https://docs.rs/clap) CLI
binary and generates a complete CircleCI orb: one command and one job per subcommand, an
executor, a Dockerfile, and optionally the CI configuration to keep the orb in sync with the
binary automatically.

The workflow is captured once in a `gen-circleci-orb.toml` file (`init`), regenerated on
demand from the binary's `--help` (`generate`), and kept in sync as the generator evolves
(`update`). It can also wire in [gen-orb-mcp](https://github.com/jerus-org/gen-orb-mcp) so an
AI assistant can understand the generated orb.

> **Repository layout.** This is a Cargo workspace. The primary crate — with full user
> documentation, installation instructions, and CLI reference — lives in
> [`crates/gen-circleci-orb/`](crates/gen-circleci-orb/README.md).

## Quick start

```bash
cargo install gen-circleci-orb        # or: cargo binstall gen-circleci-orb

gen-circleci-orb init                 # capture the workflow in gen-circleci-orb.toml
gen-circleci-orb generate             # generate the orb from the binary's --help
gen-circleci-orb update               # re-sync CI wiring as the generator evolves
```

See the [crate README](crates/gen-circleci-orb/README.md) for the full guide, and the
[`docs/`](docs/) directory for the getting-started, configuration, user, design, and
architecture guides.

## Documentation

| Document | Purpose |
|----------|---------|
| [crate README](crates/gen-circleci-orb/README.md) | Full usage guide, CLI reference, configuration |
| [docs/getting-started.md](docs/getting-started.md) | First-run walkthrough |
| [docs/user-guide.md](docs/user-guide.md) | In-depth usage |
| [docs/configuration-guide.md](docs/configuration-guide.md) / [docs/advanced-configuration.md](docs/advanced-configuration.md) | Configuration reference |
| [docs/architecture.md](docs/architecture.md) | High-level architecture |
| [docs/design.md](docs/design.md) | Detailed design document |
| [docs/assurance-case.md](docs/assurance-case.md) | Security assurance case & threat model |
| [docs/RELEASING.md](docs/RELEASING.md) | Release signing & how to verify a release |
| [ROADMAP.md](ROADMAP.md) | Planned direction |
| [PRLOG.md](PRLOG.md) / [crate CHANGELOG](crates/gen-circleci-orb/CHANGELOG.md) | Release history |

## Contributing & project information

- [Contributing guide](CONTRIBUTING.md)
- [Code of Conduct](CODE_OF_CONDUCT.md)
- [Governance](GOVERNANCE.md)
- [Security policy](SECURITY.md)

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.

Binary distributions bundle third-party Rust crates under their own license terms; the full
notices ship with the crate in
[crates/gen-circleci-orb/THIRD-PARTY-LICENSES.md](crates/gen-circleci-orb/THIRD-PARTY-LICENSES.md)
(generated with [`cargo-about`](https://github.com/EmbarkStudios/cargo-about); run `just licenses`
to refresh).

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion
in the work by you, as defined in the Apache-2.0 license, shall be dual-licensed as above,
without any additional terms or conditions.
