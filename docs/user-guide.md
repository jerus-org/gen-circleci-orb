# gen-circleci-orb User Guide

## Overview

`gen-circleci-orb` has four subcommands:

| Subcommand | What it does |
|-----------|-------------|
| `init` | Interactively captures config into `gen-circleci-orb.toml`, runs `generate`, and patches the repo's CircleCI configs to keep the orb in sync |
| `generate` | Introspects a binary's `--help` output and writes orb source files (reads `gen-circleci-orb.toml` when present) |
| `update` | Re-syncs the managed CI blocks to the current generator flow from the committed config |
| `config` | Inspects and edits orb-content generation settings in `gen-circleci-orb.toml` |

Run `init` once per project to wire everything into CI. Use `generate` on its own for a one-off
or to inspect the output before committing; use `update` to re-sync the wiring after a generator
upgrade.

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

CircleCI also restricts the parameter name `name` within orb command definitions. When
a CLI flag with a restricted name is encountered, `gen-circleci-orb` renames the orb
parameter to `{subcommand}_{param}` rather than dropping it — see the note under
[Parameter rendering](#parameter-rendering) below. The underlying CLI flag is unchanged.
For clearest generated output, prefer descriptive flag names from the outset
(e.g. `--server-name` rather than `--name`).

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
<<# parameters.server_name >>--name "<< parameters.server_name >>"<</ parameters.server_name >>
```
```
<<# parameters.force >>--force<</ parameters.force >>
```

> **Note:** CircleCI restricts certain parameter names in command definitions (currently
> `name`). When a CLI flag uses a restricted name, the generator renames the orb parameter
> to `{subcommand}_{param}` — for example, a `--name` flag on the `generate` subcommand
> becomes the orb parameter `generate_name`. The original CLI flag is still emitted
> unchanged in the script, so the underlying binary call is unaffected.

### Jobs

Each job file contains the same parameters as the corresponding command, plus:
- `executor: default` — uses the generated executor
- `steps: [checkout, <command-name>: {parameters}]` — checkout then delegate

### Executor

`src/executors/default.yml` defines a Docker executor with a `tag` parameter
(default: `latest`) pointing to `<docker-namespace>/<binary>:<< parameters.tag >>`,
where `<docker-namespace>` is the value passed to `--docker-namespace` at `init` time
(or `--orb-namespace` at `generate` time, which defaults to the first namespace value passed to `init`).

### Dockerfile

```dockerfile
FROM debian:13-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl git \
    && rm -rf /var/lib/apt/lists/* \
    && curl -L --proto '=https' --tlsv1.2 -sSf \
       https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh | bash \
    && cargo-binstall --no-confirm <binary> \
    && rm -rf /root/.cargo/registry /root/.cargo/git
```

`debian:13-slim` provides glibc and TLS roots without unnecessary tooling. `git` is
included because CircleCI's `checkout` step requires it. The
cargo-binstall bootstrap script downloads a pre-built binstall binary — no Rust toolchain
is installed in the image. The cargo cache directories are removed after install to
keep the image small.

With `--install-method apt`:

```dockerfile
FROM debian:13-slim
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

Each namespace has independent visibility. Use `--public-orb-namespace <NS>` and
`--private-orb-namespace <NS>` (both repeatable) to declare each namespace explicitly.
At least one of the two flags must be provided; the total set of namespaces is the union.

```bash
# Single public namespace
gen-circleci-orb init --binary mytool \
  --public-orb-namespace my-org ...

# Single private namespace
gen-circleci-orb init --binary mytool \
  --private-orb-namespace my-org ...

# Mixed: production namespace public, preprod namespace private
gen-circleci-orb init --binary mytool \
  --public-orb-namespace my-org \
  --private-orb-namespace my-org-preprod ...

# All namespaces private
gen-circleci-orb init --binary mytool \
  --private-orb-namespace my-org \
  --private-orb-namespace my-org-preprod ...
```

`--private-orb-namespace` controls whether `--private` is passed to `circleci orb create`
in that namespace's `ensure-orb-registered-<ns>` job. The other namespace's job is unaffected.

**This must be decided before running `init` for the first time.** CircleCI sets orb
visibility at creation time and it cannot be changed afterwards. Running `init` again
with different visibility flags has no effect if the orb already exists —
the `ensure-orb-registered-<ns>` job silently skips `circleci orb create` when the orb
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
            --orb-namespace <ns> \
            --orb-dir <orb-dir>
```

`build-binary` compiles the binary from the current source using the official public
`rust:latest` image and persists it to the CircleCI workspace. This ensures `regenerate-orb`
always introspects the binary that matches the current commit, not a previously published
release.

`regenerate-orb` uses the `jerusdp/gen-circleci-orb` Docker image, which has `gen-circleci-orb`
pre-installed (`debian:13-slim` base). It attaches the workspace at `/tmp/bin` to get the target
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
CircleCI orb namespace (`--public-orb-namespace` / `--private-orb-namespace`). The `CRATE_VERSION_<BINARY_UPPERCASED>` variable
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

With a single `--public-orb-namespace my-org` this produces:
```yaml
requires: [publish-orb-my-org]
```

With `--public-orb-namespace my-org --private-orb-namespace my-org-preprod`:
```yaml
requires: [publish-orb-my-org, publish-orb-my-org-preprod]
```

The publish job names are derived from the namespace values — no additional flag is
needed. If `toolkit/release_crate:` is not present in the release workflow, this step is
silently skipped.

### Idempotency

`init` checks for existing entries before inserting. Running it twice produces identical
output. Key idempotency checks for `config.yml`:

- `gen-circleci-orb:` in `orbs:` → skip gen-circleci-orb orb entry
- `orb-tools:` in `orbs:` → skip orb-tools orb entry
- `build-binary:` and `regenerate-orb:` in content → skip inline job definitions
- `orb-tools/pack:` in content → skip validation workflow steps
- `  orb-release:` in content → skip entire tag-triggered release workflow

The idempotency check is presence-based: if the key exists, the entry is left unchanged.
Version updates are handled separately — see [Keeping CI up to date](#keeping-ci-up-to-date).

### Design principle

The release pipeline in `config.yml` uses only public orbs:

- `circleci/orb-tools` — pack, review, publish
- `jerus-org/gen-circleci-orb` — binary build, Docker image build, orb registration

The `regenerate-orb` job uses the `jerusdp/gen-circleci-orb` Docker image with
`gen-circleci-orb` pre-installed and does not require the orb being generated to be
published yet — there is no circular dependency at init time.

Container builds and orb registration logic live in the gen-circleci-orb orb itself
(`build_container`, `ensure_orb_registered` jobs). This means bug fixes and improvements
to those steps propagate to all consumers via a Renovate bump to the
`gen-circleci-orb:` pin — no re-running `init` required.

## Bootstrapping sequence

The first release after running `init` triggers this sequence automatically:

1. A `<crate-tag-prefix>*` git tag triggers the `orb-release:` workflow in `config.yml`
2. `gen-circleci-orb/build_rust_binary` compiles the binary and persists it to workspace
3. `orb-tools/pack` checks out the repo, injects the release version into the executor
   `default.yml`, and packs the orb source
4. `gen-circleci-orb/build_container` pulls the binary from workspace, builds and pushes
   the Docker image tagged `:<version>` and `:latest`
5. `gen-circleci-orb/ensure_orb_registered` creates the orb in each namespace if it does
   not already exist
6. `orb-tools/publish` publishes the packed orb to each namespace after container, pack,
   and registration all succeed

There is no circular dependency: `regenerate-orb` (in the build workflow) uses the
`jerusdp/gen-circleci-orb` Docker image and does not depend on the orb being published.

## Keeping CI up to date

There are three independent kinds of drift to keep on top of: **orb version pins** (the
`@<version>` in `orbs:`), the **generated wiring shape** (the jobs and workflow structure
gen-circleci-orb emits, which can change as the generator itself evolves), and **container
image pins** (the tag or digest on images pinned in `gen-circleci-orb.toml`).

### Re-syncing the wiring: `update`

`update` handles the second kind. It reads the committed `gen-circleci-orb.toml` (never
overwriting it) and rewrites only the gen-circleci-orb-managed blocks in `config.yml`,
preserving your own jobs:

```bash
gen-circleci-orb update --check   # in CI: exit non-zero + show a diff when out of date
gen-circleci-orb update           # apply the re-sync
```

Wire `update --check` into your validation workflow so a generator upgrade shows up as a
failing check with guidance, rather than as silent drift. Because `update` is
non-interactive, it relies entirely on `gen-circleci-orb.toml`: it fails (pointing you at
`init`) on a missing required section and warns on present-but-empty fields, rather than
guessing. To change the wiring, edit `gen-circleci-orb.toml` and re-run `update` (or re-run
`init` to be re-prompted for the values).

### Orb version pins

`init` writes current orb versions on the first run. After that, **three orb entries in
`config.yml` can become stale** as new versions are released:

```yaml
orbs:
  gen-circleci-orb: jerus-org/gen-circleci-orb@<version>  # set to running binary version
  orb-tools: circleci/orb-tools@<version>                 # set via --orb-tools-version
  gen-orb-mcp: jerus-org/gen-orb-mcp@<version>            # if --mcp, set via --gen-orb-mcp-version
```

### Recommended: Renovate

Renovate's default configuration includes the CircleCI orb datasource and will
automatically track and bump all three entries. Once the PR is raised you get a diff
showing exactly what changed in each orb before merging.

```json
{
  "extends": ["config:base"]
}
```

This is the recommended approach because it handles all orb versions uniformly — not just
the ones `gen-circleci-orb` introduced — and requires no manual tracking.

### Alternative: MCP-assisted updates

If Renovate is not in use, the gen-circleci-orb MCP server can assist. Once your orb is
published, install the MCP server (`gen-orb-mcp generate --orb-path orb/src/@orb.yml`)
and connect it to your AI coding assistant. The MCP server exposes the orb's jobs,
commands, and conformance rules as resources, enabling the assistant to:

- Identify which orb versions are pinned in your CI config
- Suggest or apply version bumps with awareness of breaking changes
- Walk you through any required migration steps if a new orb version introduces renamed
  or restructured jobs

This path suits teams that prefer AI-assisted, on-demand maintenance over automated
dependency bots, or that need guidance through a migration alongside the version bump.

**Worth using `--mcp` even without an AI assistant.** When `gen-circleci-orb init` is run
with `--mcp`, it wires a `gen-orb-mcp/build_mcp_server` step into the release pipeline. Each
release of the orb then generates a `migrations/<version>.json` file in the repository,
recording any renamed or restructured jobs in a structured, human-readable format. This
file can be consulted directly — without an AI tool — when manually upgrading consumers
from one orb version to another. Passing `--mcp` now means the migration trail exists if
it is ever needed, regardless of whether AI tooling is in use at the time.

### Container image pins

The images you pin in `gen-circleci-orb.toml` — `[orb].base_image` / `builder_image` and
`[ci].rust_image` — go stale: a newer tag supersedes the one you pinned, or the tag you
pinned is rebuilt under a new digest. Either way the pin should move on its own, without a
gen-circleci-orb release: it lives in your repo and is yours to control. A rebuild of the
same tag routinely carries security fixes you want in promptly.

The toml is the single source of truth for every pin, but the two generated artifacts it
feeds behave differently, and that difference is what a pin-management tool must account
for:

| Artifact | Regenerated at run time? | Pin tracked where |
|---|---|---|
| `orb/Dockerfile` | Yes — rebuilt from the toml on every run | Toml only; a pin written into the Dockerfile is stripped on the next regeneration |
| `.circleci/config.yml` | No — CircleCI reads it from the commit | Toml **and** the committed config, which must agree |

So `[ci].rust_image` is stored twice: in the toml, and in the `rust_image:` lines `update`
emits into the CI config. **Both copies have to move together.** Bump one without the
other and `update --check` fails on the drift — correctly, since the wiring genuinely no
longer matches the toml. This applies to whatever the pin holds: a tag bump splits the two
copies exactly as a digest bump does.

#### Example: Renovate

Any pin-management tool works, provided it updates both copies in the same change.
Renovate needs a custom manager per pin, because an image inside a toml value or a
CircleCI job parameter is not something its built-in managers recognise. The example
below pins by digest, the usual case; the same shape works for a tag-only pin with the
`currentDigest` group dropped:

```json
{
  "customManagers": [
    {
      "customType": "regex",
      "description": "Pinned build image digest in gen-circleci-orb.toml ([ci].rust_image)",
      "managerFilePatterns": ["/^gen-circleci-orb\\.toml$/"],
      "matchStrings": [
        "rust_image\\s*=\\s*\"(?<depName>[^:\"]+):(?<currentValue>[^@\"]+)@(?<currentDigest>sha256:[a-f0-9]+)\""
      ],
      "datasourceTemplate": "docker"
    },
    {
      "customType": "regex",
      "description": "The same digest, as emitted into the CircleCI config",
      "managerFilePatterns": ["/^\\.circleci/.+\\.ya?ml$/"],
      "matchStrings": [
        "rust_image:\\s*(?<depName>[^:\\s]+):(?<currentValue>[^@\\s]+)@(?<currentDigest>sha256:[a-f0-9]+)"
      ],
      "datasourceTemplate": "docker"
    }
  ],
  "packageRules": [
    {
      "description": "Keep both copies in one PR so update --check stays green",
      "matchDatasources": ["docker"],
      "matchPackageNames": ["my-org/ci-rust"],
      "groupName": "pinned containers"
    }
  ]
}
```

Both managers resolve to the same image, so the shared `groupName` puts them in a single
PR that moves the toml and the config together. Grouping is the load-bearing part: without
it the two copies can land in separate PRs, and whichever merges first breaks the check.

A matching rule should disable the `dockerfile` manager on `orb/Dockerfile`, so the bot
does not write a digest into a file that is regenerated out from under it:

```json
{
  "matchManagers": ["dockerfile"],
  "matchFileNames": ["orb/Dockerfile"],
  "enabled": false
}
```
