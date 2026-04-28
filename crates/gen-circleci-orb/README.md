# gen-circleci-orb

Generate a CircleCI orb to provide the facilities offered by a CLI program.

[![Crates.io](https://img.shields.io/crates/v/gen-circleci-orb.svg)](https://crates.io/crates/gen-circleci-orb)
[![Documentation](https://docs.rs/gen-circleci-orb/badge.svg)](https://docs.rs/gen-circleci-orb)
[![License](https://img.shields.io/crates/l/gen-circleci-orb.svg)](https://github.com/jerus-org/gen-circleci-orb#license)

## Overview

**gen-circleci-orb** reads the `--help` output of any CLI binary and generates a complete
CircleCI orb: one command and one job per subcommand, an executor, a Dockerfile, and optionally
the CI configuration to keep the orb in sync with the binary automatically.

## Installation

```bash
cargo binstall gen-circleci-orb
# or
cargo install gen-circleci-orb
```

## Quick start

**Step 1 — generate orb source for your binary (run from project root):**

```bash
gen-circleci-orb generate \
  --binary my-tool \
  --namespace my-org
```

This writes orb source into an `orb/` subdirectory (the default `--orb-dir`):
- `orb/src/@orb.yml` — orb metadata (version, description)
- `orb/src/commands/<subcommand>.yml` — one per leaf subcommand
- `orb/src/jobs/<subcommand>.yml` — one per leaf subcommand
- `orb/src/executors/default.yml` — Docker executor with a `tag` parameter
- `orb/Dockerfile` — image that pre-installs your binary

The orb source is always isolated in its own subdirectory so it cannot be confused
with existing project source. If the target directory already exists but doesn't
contain a CircleCI orb, an error is raised.

**Step 2 — wire orb generation into CI:**

```bash
gen-circleci-orb init \
  --binary my-tool \
  --namespace my-org \
  --build-workflow validation \
  --release-workflow release \
  --requires-job common-tests \
  --release-after-job release-my-tool
```

This patches `.circleci/config.yml` and `.circleci/release.yml` to add:
- A `regenerate-orb` job that re-runs `generate` on every build
- `orb-tools/pack` + `orb-tools/validate` steps to verify the generated orb
- A `build-container` job in the release workflow to publish the Docker image
- An `orb-tools/publish` step to publish the orb to the CircleCI registry

## `generate` reference

```
gen-circleci-orb generate [OPTIONS] --binary <BINARY> --namespace <NAMESPACE>

Options:
  --binary <BINARY>               Binary to introspect (must be on PATH)
  --namespace <NAMESPACE>         CircleCI namespace (repeatable)
  --output <DIR>                  Project root directory [default: .]
  --orb-dir <DIR>                 Orb subdirectory within --output [default: orb]
  --install-method <METHOD>       binstall | apt [default: binstall]
  --base-image <IMAGE>            Docker base image [default: ubuntu:24.04]
  --home-url <URL>                Home URL for orb registry display
  --source-url <URL>              Source URL for orb registry display
  --dry-run                       Print planned files, write nothing
```

## `init` reference

```
gen-circleci-orb init [OPTIONS] --binary <BINARY> --namespace <NAMESPACE>
                                 --build-workflow <WF> --release-workflow <WF>

Options:
  --binary <BINARY>               Binary to introspect
  --namespace <NAMESPACE>         CircleCI namespace (repeatable)
  --build-workflow <WF>           Validation workflow name
  --release-workflow <WF>         Release workflow name
  --requires-job <JOB>            Job regenerate-orb should require
  --release-after-job <JOB>       Job build-container should require
  --orb-dir <DIR>                 Orb output directory [default: orb]
  --ci-dir <DIR>                  CircleCI config directory [default: .circleci]
  --orb-tools-version <VER>       circleci/orb-tools pin [default: 12.3.3]
  --docker-orb-version <VER>      circleci/docker pin [default: 3.2.0]
  --docker-context <CTX>          CircleCI context for Docker Hub [default: docker-credentials]
  --orb-context <CTX>             CircleCI context for orb publish [default: orb-publishing]
  --mcp                           Wire in toolkit/build_mcp_server (jerus-org toolkit only)
  --dry-run                       Print planned changes, write nothing
```

## Generated artifacts

| File | Description |
|------|-------------|
| `src/@orb.yml` | Orb metadata: version 2.1, description, display URLs |
| `src/executors/default.yml` | Docker executor with `tag` parameter (default: latest) |
| `src/commands/<name>.yml` | Reusable command with all parameters and a `run:` step |
| `src/jobs/<name>.yml` | Job wrapping the command: checkout + delegate |
| `Dockerfile` | `FROM <base-image>` + install step (binstall or apt) |

## Environment requirements

| Tool | Purpose |
|------|---------|
| The target binary | Must be on `PATH` for `--help` introspection |
| `circleci` CLI | To pack/validate the generated orb (optional, for local testing) |

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.

## Contributing

See the [Contributing Guide](https://github.com/jerus-org/gen-circleci-orb/blob/main/CONTRIBUTING.md)
and [Code of Conduct](https://github.com/jerus-org/gen-circleci-orb/blob/main/CODE_OF_CONDUCT.md).

## Changelog

See [CHANGELOG.md](CHANGELOG.md) for release history.
