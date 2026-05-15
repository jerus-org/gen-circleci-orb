# gen-circleci-orb

Generate a CircleCI orb to provide the facilities offered by a CLI program.

[![Crates.io](https://img.shields.io/crates/v/gen-circleci-orb.svg)](https://crates.io/crates/gen-circleci-orb)
[![Documentation](https://docs.rs/gen-circleci-orb/badge.svg)](https://docs.rs/gen-circleci-orb)
[![License](https://img.shields.io/crates/l/gen-circleci-orb.svg)](https://github.com/jerus-org/gen-circleci-orb#license)

## Overview

**gen-circleci-orb** reads the `--help` output of any CLI binary and generates a complete
CircleCI orb: one command and one job per subcommand, an executor, a Dockerfile, and
optionally the CI configuration to keep the orb in sync with the binary automatically.

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
  --orb-namespace my-org
```

This writes orb source into an `orb/` subdirectory (the default `--orb-dir`):
- `orb/src/@orb.yml` — orb metadata (version, description)
- `orb/src/commands/<subcommand>.yml` — one per leaf subcommand
- `orb/src/jobs/<subcommand>.yml` — one per leaf subcommand
- `orb/src/executors/default.yml` — Docker executor with a `tag` parameter
- `orb/Dockerfile` — image that pre-installs your binary

**Step 2 — wire orb generation into CI:**

```bash
gen-circleci-orb init \
  --binary my-tool \
  --public-orb-namespace my-org \
  --docker-namespace my-docker-org \
  --build-workflow validation \
  --release-workflow release \
  --crate-tag-prefix my-tool-v \
  --requires-job common-tests \
  --release-after-job release-my-tool
```

This patches `.circleci/config.yml` to add:
- A `build-binary` + `regenerate-orb` job pair that rebuilds and re-generates the orb on every build
- `orb-tools/pack` + `orb-tools/review` steps to validate the generated orb
- A tag-triggered `orb-release:` workflow that builds the container, registers the orb,
  and publishes it to the CircleCI registry on each crate release tag

## `generate` reference

```
gen-circleci-orb generate [OPTIONS] --binary <BINARY> --orb-namespace <NAMESPACE>

Options:
  --binary <BINARY>               Binary to introspect (must be on PATH)
  --orb-namespace <NAMESPACE>     CircleCI orb namespace (repeatable)
  --output <DIR>                  Project root directory [default: .]
  --orb-dir <DIR>                 Orb subdirectory within --output [default: orb]
  --install-method <METHOD>       binstall | apt [default: binstall]
  --base-image <IMAGE>            Docker base image [default: debian:12-slim]
  --home-url <URL>                Home URL for orb registry display
  --source-url <URL>              Source URL for orb registry display
  --dry-run                       Print planned files, write nothing
```

## `init` reference

```
gen-circleci-orb init [OPTIONS]
    --binary <BINARY>
    --public-orb-namespace <NS> | --private-orb-namespace <NS>
    --docker-namespace <NS>
    --build-workflow <WF>
    --release-workflow <WF>
    --crate-tag-prefix <PREFIX>
    --release-after-job <JOB>

Required:
  --binary <BINARY>                   Binary to introspect (must be on PATH)
  --public-orb-namespace <NS>         CircleCI orb namespace, public (repeatable)
  --private-orb-namespace <NS>        CircleCI orb namespace, private (repeatable)
  --docker-namespace <NS>             Docker Hub (or registry) namespace for the container image
  --build-workflow <WF>               Validation workflow name to patch
  --release-workflow <WF>             Release workflow name to patch
  --crate-tag-prefix <PREFIX>         Crate release tag prefix (e.g. my-tool-v); filters the
                                      orb-release: workflow trigger
  --release-after-job <JOB>           Job in the release workflow after which orb release jobs run

Options:
  --requires-job <JOB>                Job that regenerate-orb should require
  --orb-dir <DIR>                     Orb output directory [default: orb]
  --ci-dir <DIR>                      CircleCI config directory [default: .circleci]
  --orb-tools-version <VER>           circleci/orb-tools pin [default: 12.3.3]
  --gen-circleci-orb-version <VER>    jerus-org/gen-circleci-orb orb pin
                                      [default: running binary version]
  --docker-context <CTX>              CircleCI context for Docker Hub credentials
                                      [default: docker-credentials]
  --orb-context <CTX>                 CircleCI context for orb publish credentials
                                      [default: orb-publishing]
  --mcp                               Wire in gen-orb-mcp MCP server generation + publish
  --gen-orb-mcp-version <VER>         jerus-org/gen-orb-mcp orb pin (used with --mcp)
                                      [default: 0.1.14]
  --mcp-context <CTX>                 CircleCI context for MCP server publish (used with --mcp)
                                      [default: pcu-app]
  --dry-run                           Print planned changes, write nothing
```

## Generated artifacts

| File | Description |
|------|-------------|
| `src/@orb.yml` | Orb metadata: version 2.1, description, display URLs |
| `src/executors/default.yml` | Docker executor with `tag` parameter (default: latest) |
| `src/commands/<name>.yml` | Reusable command with all parameters and a `run:` step |
| `src/jobs/<name>.yml` | Job wrapping the command: checkout + delegate |
| `Dockerfile` | `FROM <base-image>` + install step (binstall or apt) |

### Parameter required vs optional

A parameter is required in the generated orb only if the CLI itself requires it — read
from the `Usage:` line: flags outside any `[...]` group are required; flags inside
`[OPTIONS]` are optional.

Optional parameters always have a `default:` so orb consumers can omit them:
- boolean flags default to `false`
- string/enum flags with a CLI default use that value
- string flags with no CLI default use `""` (empty string)

The run step uses CircleCI mustache conditionals for optional parameters so blank or false
values are not forwarded as empty flags to the binary.

### Excluded flags

Two flags are always excluded from the generated output:
- `-h/--help` — clap built-in
- `-V/--version` **with no `<VALUE>` metavar** — clap built-in (prints binary version)

An application flag named `--version` that accepts a value (e.g. `--version <VERSION>`)
is **not** excluded. However, reusing the `-V/--version` name for application purposes
is discouraged: it conflicts with the widely-understood convention and requires
special-case handling in any tool that parses `--help` output. Prefer an explicit name
such as `--crate-version` or `--output-version` instead.

## Environment requirements

| Tool | Purpose |
|------|---------|
| The target binary | Must be on `PATH` for `--help` introspection |
| `circleci` CLI | To pack/validate the generated orb (optional, for local testing) |

## Keeping orb versions up to date

`init` writes current orb versions on first run and does not update them on re-runs.
The recommended approach for ongoing updates is [Renovate](https://docs.renovatebot.com/)
with the CircleCI orb datasource (`config:base` includes it). See
[docs/user-guide.md](https://github.com/jerus-org/gen-circleci-orb/blob/main/docs/user-guide.md#keeping-ci-up-to-date)
for alternatives including MCP-assisted updates.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.

## Contributing

See the [Contributing Guide](https://github.com/jerus-org/gen-circleci-orb/blob/main/CONTRIBUTING.md)
and [Code of Conduct](https://github.com/jerus-org/gen-circleci-orb/blob/main/CODE_OF_CONDUCT.md).

## Changelog

See [CHANGELOG.md](CHANGELOG.md) for release history.
