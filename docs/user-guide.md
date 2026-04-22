# gen-circleci-orb User Guide

## Overview

`gen-circleci-orb` has two subcommands:

| Subcommand | What it does |
|-----------|-------------|
| `generate` | Introspects a binary's `--help` output and writes orb source files |
| `init` | Runs `generate` and patches the repo's CircleCI configs to keep the orb in sync |

Use `generate` for a one-off or to inspect the output before committing.
Use `init` once per project to wire everything into CI.

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

```
<output>/
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
(default: `latest`) pointing to `jerusdp/<binary>:<< parameters.tag >>`.

### Dockerfile

```dockerfile
FROM ubuntu:24.04
RUN cargo binstall --no-confirm <binary>
```

Or with `--install-method apt`:

```dockerfile
FROM ubuntu:24.04
RUN apt-get update && apt-get install -y <binary>
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
            --output <orb-dir>
```

Added to the build workflow:
```yaml
- regenerate-orb:
    requires: [<requires-job>]
- orb-tools/pack:
    name: pack-orb
    source-dir: <orb-dir>/src
    destination-orb-path: /tmp/packed.yml
    requires: [regenerate-orb]
- orb-tools/validate:
    name: validate-orb
    orb-path: /tmp/packed.yml
    requires: [pack-orb]
```

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
          docker build -t jerusdp/<binary>:${CIRCLE_TAG} <orb-dir>
    - run:
        name: Push Docker image
        command: |
          docker push jerusdp/<binary>:${CIRCLE_TAG}
```

Added to the release workflow:
```yaml
- build-container:
    requires: [<release-after-job>]
    context: [<docker-context>]
- orb-tools/publish:
    name: publish-orb-<namespace>
    orb-path: /tmp/packed.yml
    orb-name: <namespace>/<binary>
    requires: [build-container]
    context: [<orb-context>]
```

### Idempotency

`init` checks for existing entries before inserting. Running it twice produces identical output.
Specific checks:
- `orb-tools:` in `orbs:` section → skip orb-tools insertion
- `  docker: circleci/` in `orbs:` section → skip docker orb insertion
- `regenerate-orb:` at job definition level → skip job insertion
- `orb-tools/pack:` in content → skip pack/validate workflow steps
- `      - build-container:` in release workflow → skip release workflow steps

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
