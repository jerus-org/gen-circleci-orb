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
    && apt-get install -y --no-install-recommends ca-certificates curl git \
    && rm -rf /var/lib/apt/lists/* \
    && curl -L --proto '=https' --tlsv1.2 -sSf \
       https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh | bash \
    && cargo-binstall --no-confirm <binary> \
    && rm -rf /root/.cargo/registry /root/.cargo/git
```

`debian:12-slim` provides glibc and TLS roots without unnecessary tooling. `git` is
included because CircleCI's `checkout` step requires it. The
cargo-binstall bootstrap script downloads a pre-built binstall binary — no Rust toolchain
is installed in the image. The cargo cache directories are removed after install to
keep the image small.

With `--install-method apt`:

```dockerfile
FROM debian:12-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends git <binary> \
    && rm -rf /var/lib/apt/lists/*
```

## Diff-aware writes

`generate` reads existing files before writing. If the content is identical, the file is
left untouched. The final line reports `created`, `updated`, and `unchanged` counts.
Re-running after no changes produces `0 created, 0 updated, N unchanged`.

## CI patching (init)

`init` calls `generate` first, then patches two CI config files additively.

### Orb visibility: public vs private

Pass `--private` when the orb should only be accessible within the organization.
This flag controls the `--private` argument to `circleci orb create` in the generated
`ensure-orb-registered-<ns>` job:

```bash
# Public orb (default) — listed in the CircleCI orb registry
gen-circleci-orb init --binary mytool --namespace my-org ...

# Private orb — accessible only within my-org
gen-circleci-orb init --binary mytool --namespace my-org --private ...
```

**This must be decided before running `init` for the first time.** CircleCI sets orb
visibility at creation time and it cannot be changed afterwards. Running `init` again
with or without `--private` has no effect if the orb already exists — the
`ensure-orb-registered-<ns>` job silently skips `circleci orb create` when the orb
is already registered.

### config.yml changes

Added to `orbs:`:
```yaml
orb-tools: circleci/orb-tools@<version>
```

Added to `jobs:`:
```yaml
build-binary:
  docker:
    - image: rust:latest
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
    - image: jerusdp/gen-circleci-orb:latest
  steps:
    - checkout
    - attach_workspace:
        at: /tmp/bin
    - run:
        name: Regenerate orb source
        command: |
          export PATH="/tmp/bin:$PATH"
          gen-circleci-orb generate \
            --binary <binary> \
            --namespace <namespace> \
            --orb-dir <orb-dir>
```

`build-binary` compiles the binary from the current source using the official public
`rust:latest` image and persists it to the CircleCI workspace. This ensures `regenerate-orb`
always introspects the binary that matches the current commit, not a previously published
release.

`regenerate-orb` uses the `jerusdp/gen-circleci-orb` Docker image, which has `gen-circleci-orb`
pre-installed (`debian:12-slim` base). It attaches the workspace at `/tmp/bin` to get the target
binary, adds it to `$PATH`, then runs `gen-circleci-orb generate`. No runtime installation needed.

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

Three jobs are added to `jobs:`:

**`build-binary-release`** compiles the release binary and persists it for later use in the
Docker build. The `-p <binary>` flag scopes the build to the named package, which is required
in workspace projects where `cargo build --release` without a `-p` flag would compile all
workspace members.

```yaml
build-binary-release:
  docker:
    - image: rust:latest
  steps:
    - checkout
    - run:
        name: Build release binary
        command: cargo build --release -p <binary>
    - persist_to_workspace:
        root: target/release
        paths: [<binary>]
```

**`ensure-orb-registered`** checks that the orb namespace entry exists before attempting to
publish — `orb-tools/publish` fails if the orb has never been registered. This is a separate
inline job rather than a pre-step of `orb-tools/publish` because `orb-tools/publish`
configures CLI authentication in its own steps, which execute after pre-steps. Running
`circleci orb info` before that point would fail with "please set a token". The inline job
uses `executor: orb-tools/default` (the same `circleci/circleci-cli` image used by
`orb-tools/publish`) and the `orb-publishing` context, which injects `CIRCLE_TOKEN`.
`circleci setup` must be called explicitly to write the CLI config file before any
`circleci orb` commands.

```yaml
ensure-orb-registered:
  executor: orb-tools/default
  steps:
    - run:
        name: Ensure orb is registered
        command: |
          circleci setup --token "${CIRCLE_TOKEN}" --host https://circleci.com --no-prompt
          circleci orb info <namespace>/<binary> > /dev/null 2>&1 || \
            circleci orb create <namespace>/<binary> --no-prompt
```

**`build-container`** builds and pushes the Docker image for the orb executor. The release
pipeline is approval-triggered (not tag-triggered), so `$CIRCLE_TAG` is empty. The version
is read from `versions.env` written by `toolkit/calculate_versions` and persisted to the
CircleCI workspace. The compiled binary is attached from the `build-binary-release` workspace
and copied into the Docker build context so the image contains the freshly-built release
binary. `<docker-namespace>` is the value of `--docker-namespace` — independent of the
CircleCI orb namespace (`--namespace`). The `CRATE_VERSION_<BINARY_UPPERCASED>` variable
name matches the format written by `toolkit/calculate_versions` (hyphens replaced by
underscores, all uppercase).

```yaml
build-container:
  docker:
    - image: cimg/base:stable
  steps:
    - checkout
    - setup_remote_docker
    - attach_workspace:
        at: /tmp/release-versions
    - attach_workspace:
        at: /tmp/bin
    - run:
        name: Build and push Docker image
        command: |
          source /tmp/release-versions/versions.env
          VERSION=${CRATE_VERSION_<BINARY_UPPERCASED>}
          cp /tmp/bin/<binary> <orb-dir>/<binary>
          docker build -t <docker-namespace>/<binary>:${VERSION} -t <docker-namespace>/<binary>:latest <orb-dir>
          echo "${DOCKERHUB_PASSWORD}" | docker login -u "${DOCKERHUB_USERNAME}" --password-stdin
          docker push <docker-namespace>/<binary>:${VERSION}
          docker push <docker-namespace>/<binary>:latest
```

Added to the release workflow (one `ensure-orb-registered-<ns>` and one
`orb-tools/publish` per namespace; shown here for two namespaces `ns1`/`ns2`):

```yaml
- build-binary-release:
    requires: [<release-after-job>]
- orb-tools/pack:
    name: pack-orb-release
    source_dir: <orb-dir>/src
    requires: [<release-after-job>]
- build-container:
    requires: [build-binary-release]
    context: [<docker-context>]
# ── repeated once per namespace ──────────────────────────────────────────────
- ensure-orb-registered-<ns1>:
    requires: [<release-after-job>]
    context: [<orb-context>]
- orb-tools/publish:
    name: publish-orb-<ns1>
    pre-steps:
      - attach_workspace:
          at: /tmp/release-versions
      - run:
          name: Export orb version as CIRCLE_TAG
          command: |
            source /tmp/release-versions/versions.env
            echo "export CIRCLE_TAG=v${CRATE_VERSION_<BINARY_UPPERCASED>}" >> "$BASH_ENV"
    orb_name: <ns1>/<binary>
    pub_type: production
    vcs_type: github
    requires: [build-container, pack-orb-release, ensure-orb-registered-<ns1>]
    context: [<orb-context>]
- ensure-orb-registered-<ns2>:
    requires: [<release-after-job>]
    context: [<orb-context>]
- orb-tools/publish:
    name: publish-orb-<ns2>
    ...
    requires: [build-container, pack-orb-release, ensure-orb-registered-<ns2>]
    context: [<orb-context>]
```

`build-binary-release`, `orb-tools/pack`, and all `ensure-orb-registered-<ns>` jobs run
in parallel immediately after the approval gate. `build-container` is sequential after
`build-binary-release` because it needs the compiled binary from the workspace. Each
`orb-tools/publish` job fans in from `build-container`, `pack-orb-release`, and its own
namespace's `ensure-orb-registered-<ns>` job — keeping each namespace's publish path
independent while sharing the single container build.

`orb-tools/publish` requires `$CIRCLE_TAG` to match `^v[0-9]+\.[0-9]+\.[0-9]+$` for
`pub_type: production`. The pre-steps inject it from `versions.env` via `$BASH_ENV` since
the pipeline is approval-triggered and `$CIRCLE_TAG` is otherwise empty.

### Release ordering: Docker → orb → crates.io

If the release workflow contains `toolkit/release_crate:`, `init` automatically rewires
its `requires:` line to list every `publish-orb-<ns>` job. This ensures crates.io is
published last — after all orb namespaces and the Docker image are live.

With a single `--namespace jerus-org` this produces:
```yaml
requires: [publish-orb-jerus-org]
```

With `--namespace jerus-org --namespace digital-prstv`:
```yaml
requires: [publish-orb-jerus-org, publish-orb-digital-prstv]
```

The publish job names are derived from the `--namespace` values — no additional flag is
needed. If `toolkit/release_crate:` is not present in the release workflow, this step is
silently skipped.

### Idempotency

`init` checks for existing entries before inserting. Running it twice produces identical
output. Specific checks for `release.yml`:

- `  docker: circleci/` in `orbs:` → skip docker orb insertion
- `  orb-tools: circleci/` in `orbs:` → skip orb-tools orb insertion
- `build-binary-release:` in content → skip job definition
- `ensure-orb-registered-<ns>:` present for every `--namespace` → skip job definitions
- `build-container:` in content → skip job definition
- `pack-orb-release` + `- build-binary-release:` + `- build-container:` + all per-namespace
  `ensure-orb-registered-<ns>:` and `publish-orb-<ns>` entries present → skip workflow steps
- `toolkit/release_crate:` requires already lists all `publish-orb-<ns>` jobs → skip rewire

### Design principle

`init` uses only public orbs (`circleci/orb-tools`, `circleci/docker`) for container
builds and orb publishing. The `regenerate-orb` job uses the `jerusdp/gen-circleci-orb`
Docker image (pre-installed tool, no runtime installation) and does not require the orb
being generated to be published yet.

**Exception**: step 4 (release ordering rewire) assumes `toolkit/release_crate` is the
crates.io publish job. This step is skipped silently for projects that do not use
`toolkit/release_crate`. It is intentional for jerus-org projects — the toolkit is the
standard crates.io publish mechanism across all repos in this organization.

## Bootstrapping sequence

The first release after running `init` triggers this sequence automatically:

1. CI runs `regenerate-orb` on every push → keeps orb source in sync with the binary
2. On release approval: `build-binary-release`, `orb-tools/pack`, and all
   `ensure-orb-registered-<ns>` jobs run in parallel
3. `build-container` builds and pushes the Docker image (requires compiled binary)
4. Each `orb-tools/publish: name: publish-orb-<ns>` fans in from `build-container`,
   `pack-orb-release`, and its namespace's `ensure-orb-registered-<ns>`
5. `toolkit/release_crate` publishes to crates.io after all `publish-orb-*` jobs finish

There is no circular dependency: `regenerate-orb` uses the `jerusdp/gen-circleci-orb`
image with `gen-circleci-orb` pre-installed, and does not depend on the orb being
published first.
