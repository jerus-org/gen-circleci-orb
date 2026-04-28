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

The built-in `help` subcommand is automatically excluded from the generated output.

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

Boolean flags use CircleCI mustache conditionals in the run step:
```
<<# parameters.force >>--force<</ parameters.force >>
```

Optional string parameters:
```
<<# parameters.output >>--output "<< parameters.output >>"<</ parameters.output >>
```

Required string parameters (no conditional):
```
--orb-path "<< parameters.orb_path >>"
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
regenerate-orb:
  docker:
    - image: cimg/rust:stable
  steps:
    - checkout
    - run:
        name: Install gen-circleci-orb
        command: cargo binstall --no-confirm gen-circleci-orb
    - run:
        name: Regenerate orb source
        command: |
          gen-circleci-orb generate \
            --binary <binary> \
            --namespace <namespace> \
            --orb-dir <orb-dir>
```

Added to the build workflow:
```yaml
- regenerate-orb:
    requires: [<requires-job>]
- orb-tools/pack:
    name: pack-orb
    source_dir: <orb-dir>/src
    requires: [regenerate-orb]
- orb-tools/review:
    name: review-orb
    source_dir: <orb-dir>/src
    requires: [pack-orb]
```

`orb-tools/pack` (circleci/orb-tools@12) validates the orb during packing and persists
it to the workspace. `orb-tools/review` checks for orb best-practice violations.

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
