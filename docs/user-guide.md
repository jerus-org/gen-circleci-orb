# gen-circleci-orb User Guide

## Overview

`gen-circleci-orb` has two subcommands:

| Subcommand | What it does |
|-----------|-------------|
| `generate` | Introspects a binary's `--help` output and writes orb source files |
| `init` | Runs `generate` and patches the repo's CircleCI configs to keep the orb in sync |

Use `generate` for a one-off or to inspect the output before committing.
Use `init` once per project to wire everything into CI.

Both commands are run from the project root. Orb source is always written into a dedicated
subdirectory (`orb/` by default) so it cannot be confused with existing project source.

## How the help parser works

`generate` runs `<binary> --help` to find the top-level description and subcommand list,
then runs `<binary> <subcommand> --help` for each subcommand. It targets clap-generated
help text but works on any tool that follows the same conventions:

- A `Commands:` section listing subcommands with two-space indentation
- An `Options:` section listing flags as `  -f, --flag-name <VALUE>   Description`
- Boolean flags have no `<VALUE>` metavar
- Enum flags are followed by an indented `Possible values:` block
- Defaults appear as `[default: value]` in the description text
- Required vs optional is read from the `Usage:` line: a flag listed **outside** any
  `[...]` bracket group is required; a flag inside `[OPTIONS]` or another `[...]` group
  is optional, even if it carries no default value

Two built-in flags are automatically excluded from the generated output:

- The `help` subcommand and `-h/--help` flag
- The `-V/--version` flag **when it has no `<VALUE>` metavar** — this is the clap
  built-in that prints the binary version and exits

An application flag that happens to be named `--version` but carries a `<VALUE>` metavar
(e.g. `--version <VERSION>`) is **not** excluded — it is treated as a regular string
parameter.

### Best practice: avoid reusing reserved flag names

The flags `-h/--help` and `-V/--version` have widely-understood, tool-agnostic meanings
established by POSIX convention and reinforced by clap's defaults. Using them for
application-level purposes creates ambiguity:

- Tools and scripts that parse `--help` output (including `gen-circleci-orb`) must
  special-case the flag to distinguish built-in from application use.
- Users who type `--version` expecting a version string are surprised when the flag
  instead accepts a value.
- Third-party documentation generators, shell completions, and other tooling may
  misinterpret the flag.

If a subcommand needs to accept a version string, prefer an explicit name that describes
what the version is for — for example `--crate-version`, `--server-version`, or
`--output-version`.

## Generated file structure

Orb source is always placed under `<output>/<orb-dir>/` (defaults: `.` and `orb`).
If `<output>/<orb-dir>/` already exists but does not contain `src/@orb.yml`, `generate`
refuses to write and exits with an error — this prevents accidentally overwriting an existing
`src/` directory or other project source.

```
<output>/
└── <orb-dir>/          # default: orb/
    ├── src/
    │   ├── @orb.yml                  # Orb metadata only (version: 2.1, description, display)
    │   ├── executors/
    │   │   └── default.yml           # Docker executor with << parameters.tag >>
    │   ├── commands/
    │   │   └── <subcommand>.yml      # One per leaf subcommand
    │   └── jobs/
    │       └── <subcommand>.yml      # One per leaf subcommand
    └── Dockerfile
```

### @orb.yml

Contains only `version: 2.1`, `description`, and an optional `display:` block.
`circleci orb pack` discovers commands, jobs, and executors from the subdirectories
automatically — no explicit listing is needed or valid here.

### Commands

Each command file contains:
- `description` — from the binary's `--help` for that subcommand
- `parameters` — one per CLI flag (mapped to orb parameter types)
- `steps` — a single `run:` step invoking `<binary> <subcommand> [flags]`

Parameter types map as follows:

| CLI pattern | Orb parameter type |
|-------------|-------------------|
| `--flag` (no `<VALUE>`) | `boolean` |
| `--name <VALUE>` | `string` |
| `--count <VALUE>` (integer context) | `integer` |
| `--fmt <VALUE>` with `Possible values:` | `enum` |

### Required vs optional parameters

Whether a parameter is required in the generated orb mirrors whether it is required by
the CLI itself — determined by reading the `Usage:` line, not by whether a default is present.

| Usage line | Has CLI default | Orb `default:` | Run step | Consumer must supply? |
|-----------|----------------|----------------|----------|-----------------------|
| Outside `[...]` | no | _(none)_ | unconditional | **yes** |
| Inside `[OPTIONS]` | yes (`[default: x]`) | `x` | mustache conditional | no |
| Inside `[OPTIONS]` | no | `""` (string) or `false` (boolean) | mustache conditional | no |

**Required parameters** appear on the `Usage:` line outside any bracket group (e.g.
`Usage: tool cmd [OPTIONS] --binary <BINARY>`). The orb parameter has no `default:` key,
CircleCI enforces that the consumer supplies a value, and the run step passes the flag
unconditionally:
```
--binary "<< parameters.binary >>"
```

**Optional parameters with a CLI default** have a `default:` in the orb matching the
CLI default. The run step uses a mustache conditional so an empty value does not forward
a blank flag to the binary:
```
<<# parameters.output >>--output "<< parameters.output >>"<</ parameters.output >>
```

**Optional parameters without a CLI default** (inside `[OPTIONS]` but no `[default: …]`
annotation) receive `default: ""` (strings) or `default: false` (booleans) in the orb.
This makes the parameter optional for orb consumers — they can omit it. The mustache
conditional in the run step ensures the flag is not forwarded when the value is empty or false:
```
<<# parameters.name >>--name "<< parameters.name >>"<</ parameters.name >>
```
```
<<# parameters.force >>--force<</ parameters.force >>
```

### Jobs

Each job file contains the same parameters as the corresponding command, plus:
- `executor: default` — uses the generated executor
- `steps: [checkout, <command-name>: {parameters}]` — checkout then delegate

### Executor

`src/executors/default.yml` defines a Docker executor with a `tag` parameter
(default: `latest`) pointing to `<docker-namespace>/<binary>:<< parameters.tag >>`,
where `<docker-namespace>` is the value passed to `--docker-namespace` at `init` time
(or `--namespace` at `generate` time, which defaults to the first namespace value).

### Dockerfile

```dockerfile
FROM debian:12-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && curl -L --proto '=https' --tlsv1.2 -sSf \
       https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh | bash \
    && cargo-binstall --no-confirm <binary> \
    && rm -rf /root/.cargo/registry /root/.cargo/git
```

`debian:12-slim` provides glibc and TLS roots without unnecessary tooling. The
cargo-binstall bootstrap script downloads a pre-built binstall binary — no Rust toolchain
is installed in the image. The cargo cache directories are removed after install to
keep the image small.

With `--install-method apt`:

```dockerfile
FROM debian:12-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends <binary> \
    && rm -rf /var/lib/apt/lists/*
```

## Diff-aware writes

`generate` reads existing files before writing. If the content is identical, the file is
left untouched. The final line reports `created`, `updated`, and `unchanged` counts.
Re-running after no changes produces `0 created, 0 updated, N unchanged`.

## CI patching (init)

`init` calls `generate` first, then patches two CI config files additively.

### config.yml changes

Added to `orbs:`:
```yaml
orb-tools: circleci/orb-tools@<version>
```

Added to `jobs:`:
```yaml
build-binary:
  docker:
    - image: jerusdp/ci-rust:rolling-6mo
  steps:
    - checkout
    - run:
        name: Build binary
        command: cargo build --release
    - persist_to_workspace:
        root: target/release
        paths: [<binary>]

regenerate-orb:
  docker:
    - image: jerusdp/ci-rust:rolling-6mo
  steps:
    - checkout
    - attach_workspace:
        at: /tmp/bin
    - run:
        name: Install gen-circleci-orb
        command: |
          curl -L --proto '=https' --tlsv1.2 -sSf \
            https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh | bash
          cargo-binstall --no-confirm gen-circleci-orb
    - run:
        name: Regenerate orb source
        command: |
          export PATH="/tmp/bin:$PATH"
          gen-circleci-orb generate \
            --binary <binary> \
            --namespace <namespace> \
            --orb-dir <orb-dir>
```

`build-binary` compiles the binary from the current source and persists it to the CircleCI
workspace. This ensures `regenerate-orb` always introspects the binary that matches the
current commit, not a previously published release. Both jobs use `jerusdp/ci-rust:rolling-6mo`,
which provides the Rust toolchain and has `cargo-binstall` pre-installed.

`regenerate-orb` attaches the workspace at `/tmp/bin`, installs `gen-circleci-orb` via
`cargo binstall` (no bootstrap needed — `cargo-binstall` is already in the ci-rust image),
then adds `/tmp/bin` to `$PATH` so the binary is discoverable by name.

Added to the build workflow:
```yaml
- build-binary:
    requires: [<requires-job>]
- regenerate-orb:
    requires: [build-binary]
- orb-tools/pack:
    name: pack-orb
    source_dir: <orb-dir>/src
    requires: [regenerate-orb]
- orb-tools/review:
    name: review-orb
    source_dir: <orb-dir>/src
    requires: [pack-orb]
```

`build-binary` depends on the user-configured prerequisite job (e.g. `toolkit/common_tests`)
so the binary is only built after the test suite passes. `orb-tools/pack` validates the orb
during packing and persists it to the workspace. `orb-tools/review` checks for orb
best-practice violations.

### release.yml changes

Added to `orbs:`:
```yaml
docker: circleci/docker@<version>
orb-tools: circleci/orb-tools@<version>
```

Added to `jobs:`:
```yaml
build-container:
  docker:
    - image: cimg/base:stable
  steps:
    - checkout
    - setup_remote_docker
    - run:
        name: Build Docker image
        command: |
          docker build -t <docker-namespace>/<binary>:${CIRCLE_TAG} <orb-dir>
    - run:
        name: Push Docker image
        command: |
          docker push <docker-namespace>/<binary>:${CIRCLE_TAG}
```

`<docker-namespace>` is the value of `--docker-namespace` — independent of the CircleCI
orb namespace (`--namespace`).

Added to the release workflow:
```yaml
- orb-tools/pack:
    name: pack-orb-release
    source_dir: <orb-dir>/src
    requires: [<release-after-job>]
- build-container:
    requires: [<release-after-job>]
    context: [<docker-context>]
- orb-tools/publish:
    name: publish-orb-<namespace>
    orb_name: <namespace>/<binary>
    pub_type: production
    requires: [build-container, pack-orb-release]
    context: [<orb-context>]
```

`orb-tools/pack` runs in parallel with `build-container` and provides the packed orb
to `orb-tools/publish` via workspace persistence.

### Idempotency

`init` checks for existing entries before inserting. Running it twice produces identical output.
Specific checks:
- `orb-tools:` in `orbs:` section → skip orb-tools insertion
- `  docker: circleci/` in `orbs:` section → skip docker orb insertion
- `regenerate-orb:` at job definition level → skip job insertion
- `orb-tools/pack:` in content → skip pack/review workflow steps
- `pack-orb-release` + `build-container:` + `orb-tools/publish:` in release workflow → skip release workflow steps

### Design principle

`init` does not assume the consuming repo uses any specific orb or toolkit.
The generated CI installs `gen-circleci-orb` at runtime from crates.io and uses only
the standard public `circleci/orb-tools` and `circleci/docker` orbs.

## Bootstrapping sequence

The first release after running `init` triggers this sequence automatically:

1. CI runs `regenerate-orb` on every push → keeps orb source in sync
2. On release tag: `build-container` builds and pushes the Docker image
3. `orb-tools/publish` publishes the orb to the CircleCI registry
4. Future consumers can add `<namespace>/<binary>@<version>` to their orbs and use the
   generated jobs/commands directly

There is no circular dependency: `regenerate-orb` installs `gen-circleci-orb` fresh from
crates.io on each run and does not depend on the orb being published first.
