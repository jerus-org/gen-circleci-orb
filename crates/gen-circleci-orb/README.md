# gen-circleci-orb

Generate a CircleCI orb to provide the facilities offered by a CLI program.

[![Crates.io](https://img.shields.io/crates/v/gen-circleci-orb.svg)](https://crates.io/crates/gen-circleci-orb)
[![Documentation](https://docs.rs/gen-circleci-orb/badge.svg)](https://docs.rs/gen-circleci-orb)
[![License](https://img.shields.io/crates/l/gen-circleci-orb.svg)](https://github.com/jerus-org/gen-circleci-orb#license)
[![OpenSSF Best Practices](https://www.bestpractices.dev/projects/13667/badge)](https://www.bestpractices.dev/projects/13667)

## Overview

**gen-circleci-orb** reads the `--help` output of a Rust [clap](https://docs.rs/clap) CLI
binary and generates a complete CircleCI orb: one command and one job per subcommand, an
executor, a Dockerfile, and optionally the CI configuration to keep the orb in sync with the
binary automatically.

The workflow is captured once in a `gen-circleci-orb.toml` file (`init`), regenerated on
demand from the binary's `--help` (`generate`), and kept in sync as the generator evolves
(`update`). It can also wire in [gen-orb-mcp](https://github.com/jerus-org/gen-orb-mcp)
so an AI assistant can understand the generated orb.

> **Pre-production (0.1.x).** The generator, the `gen-circleci-orb.toml` schema, and the
> generated CI shape are stable enough for real use — this project dogfoods its own orb —
> but the CLI and config surface may still change ahead of 1.0. Pin the orb version and
> let [Renovate](https://docs.renovatebot.com/) manage upgrades.

## Installation

```bash
cargo binstall gen-circleci-orb
# or
cargo install gen-circleci-orb
```

## Quick start

**Step 1 — set up with `init` (run once from project root):**

`init` is the entry point. It runs `generate`, patches `.circleci/config.yml`, and records
every value in a `gen-circleci-orb.toml` so later commands need no flags. It is interactive:
run it with just the binary and it prompts for the required values it doesn't have, each
pre-filled with a sensible default.

```bash
gen-circleci-orb init --binary my-tool
```

Passing a flag skips its prompt, so the same command is fully scriptable (and non-interactive
under `--dry-run` or without a TTY):

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

`init` patches `.circleci/config.yml` to add:
- A `build-binary` + `regenerate-orb` job pair that rebuilds and re-generates the orb on every build
- `orb-tools/pack` + `orb-tools/review` steps to validate the generated orb
- A tag-triggered `orb-release:` workflow that builds the container, registers the orb,
  and publishes it to the CircleCI registry on each crate release tag

Commit `gen-circleci-orb.toml`.

**Step 2 — regenerate the orb source with `generate`:**

After `init`, `generate` needs no flags — it reads the binary, namespaces, and base image
from `gen-circleci-orb.toml`. This is what the `regenerate-orb` CI job runs on every build:

```bash
gen-circleci-orb generate
```

It writes orb source into an `orb/` subdirectory (the default `--orb-dir`):
- `orb/src/@orb.yml` — orb metadata (version, description)
- `orb/src/commands/<subcommand>.yml` — one per leaf subcommand
- `orb/src/jobs/<subcommand>.yml` — one per leaf subcommand
- `orb/src/executors/default.yml` — Docker executor with a `tag` parameter
- `orb/Dockerfile` — image that pre-installs your binary

You can also run `generate` without a config for a quick one-off, supplying the values
explicitly: `gen-circleci-orb generate --binary my-tool --orb-namespace my-org`.

**Step 3 — keep the wiring current as the generator evolves:**

```bash
gen-circleci-orb update --check   # CI: fail if the managed wiring is out of date
gen-circleci-orb update           # rewrite the managed blocks in place
```

`update` reads the committed `gen-circleci-orb.toml` (it never overwrites it) and rewrites
only the gen-circleci-orb-managed blocks in `.circleci/config.yml`, preserving your own
jobs and customizations. Run `--check` in CI to fail when a generator upgrade has changed
the canonical wiring; run without `--check` to apply it. Renovate bumps the pinned orb
version, and `update --check` flags the drift so you re-sync deliberately.

## `generate` reference

```
gen-circleci-orb generate [OPTIONS] --binary <BINARY> --orb-namespace <NAMESPACE>

Options:
  --binary <BINARY>               Binary to introspect (must be on PATH)
  --orb-namespace <NAMESPACE>     CircleCI orb namespace (repeatable)
  --output <DIR>                  Project root directory [default: .]
  --orb-dir <DIR>                 Orb subdirectory within --output [default: orb]
  --install-method <METHOD>       binstall | apt [default: binstall]
  --base-image <IMAGE>            Docker base image [default: debian:13-slim]
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
                                      [default: 0.1.48]
  --mcp-context <CTX>                 CircleCI context for MCP server publish (used with --mcp)
                                      [default: pcu-app]
  --dry-run                           Print planned changes, write nothing
```

## `update` reference

```
gen-circleci-orb update [OPTIONS]

Options:
  --config <FILE>   Path to gen-circleci-orb.toml [default: gen-circleci-orb.toml]
  --ci-dir <DIR>    Path to the .circleci/ directory [default: .circleci]
  --check           Verify mode: write nothing and exit non-zero (with a diff and
                    guidance) when the CI wiring is out of date. For use in CI.
```

`update` is non-interactive and relies entirely on the committed `gen-circleci-orb.toml`.
It fails (pointing you at `init`) when a required section is missing and warns on
present-but-empty required fields, rather than guessing.

## `config` reference

`config` inspects and edits `gen-circleci-orb.toml` without hand-editing TOML:

```
gen-circleci-orb config [--config <FILE>] <SUBCOMMAND>

  show                              Print the current configuration
  suppress-job <SUBCOMMAND>         Stop generating a job for a subcommand
  unsuppress-job <SUBCOMMAND>       Re-enable a previously suppressed job
  add-job-group --name <NAME> --steps <a,b,c> [--description <D>] [--parameters <p,q>]
                                    Compose several subcommand steps into one job
  set-parameter-default --subcommand <S> --parameter <P> --value <V>
                                    Override a generated parameter default
```

## Configuration file (`gen-circleci-orb.toml`)

`init` writes this file; `generate` and `update` read it. It is the single source of truth
for the generated orb and CI, so it is safe to commit and review.

| Section | Purpose |
|---------|---------|
| `[orb]` | `binary`, `namespaces`, `orb_dir`, `base_image`, `builder_image`, `circleci_cli_version` — the orb's own source and container |
| `[ci]` | Workflow/job wiring: `build_workflow`, `release_workflow`, `requires_job`, `release_after_job`, `crate_tag_prefix`, `docker_namespace`, `docker_context`, `orb_context`, MCP fields, and `rust_image` |
| `[record]` | Optional auto-record: after `generate`, commit the regenerated orb source back (GPG-signed) so the published orb stays in sync with the CLI. Stores only env-var **names** — the secrets stay in CI contexts |
| `[orbs]`, `[[job_group]]`, `[[extra_job]]`, `[subcommand.*]` | Extra orb pins, composed jobs, custom jobs, and per-subcommand overrides (including `interactive` / `generate_job`) |

Two image knobs are easy to confuse:

- `[orb].base_image` / `[orb].builder_image` configure the **orb's own** generated
  `Dockerfile` (the image your orb's consumers run).
- `[ci].rust_image` configures the image the **CI build jobs** (`build-binary`,
  `orb-release-binary`) compile in. The default `rust:latest` has no libclang; set a
  clang-equipped, digest-pinned image (e.g. `jerusdp/ci-rust:rolling-6mo@sha256:…`) when
  the workspace pulls a bindgen-based `-sys` crate.

For the full walkthrough of these settings — and of composing a single complex job from several
commands (as gen-orb-mcp's `build_mcp_server` does) — see the
[Configuration Guide](https://github.com/jerus-org/gen-circleci-orb/blob/main/docs/configuration-guide.md)
and [Advanced Configuration Guide](https://github.com/jerus-org/gen-circleci-orb/blob/main/docs/advanced-configuration.md).

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

Binary distributions bundle third-party Rust crates under their own license terms; the full
notices are in [THIRD-PARTY-LICENSES.md](THIRD-PARTY-LICENSES.md) (generated with
[`cargo-about`](https://github.com/EmbarkStudios/cargo-about) — run `just licenses` to refresh).

## Contributing

See the [Contributing Guide](https://github.com/jerus-org/gen-circleci-orb/blob/main/CONTRIBUTING.md)
and [Code of Conduct](https://github.com/jerus-org/gen-circleci-orb/blob/main/CODE_OF_CONDUCT.md).

## Changelog

See [CHANGELOG.md](CHANGELOG.md) for release history.
