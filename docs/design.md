# gen-circleci-orb — Design Document

> Status: **DRAFT** — design decisions recorded; roadmap items deferred.

---

## 1. Purpose

`gen-circleci-orb` is a CLI tool that takes an existing CLI application as input and generates the
full suite of CircleCI infrastructure needed to expose that application's commands as reusable
CircleCI orb jobs and commands.

The generated output includes:

| Artifact | Description |
|----------|-------------|
| CircleCI orb | An orb following CircleCI's standard template structure, with jobs and commands derived from the CLI's subcommand tree |
| Docker container | A minimal execution environment image pre-installing the CLI binary, embedded in the orb repo |
| Orb CI pipeline | CircleCI config (3-file model) wiring the full release chain |
| MCP server | Post-publish invocation of `gen-orb-mcp` producing an MCP server for AI agent integration |

The goal is that a developer with a working CLI tool can run `gen-circleci-orb` once and receive a
fully wired, production-ready CircleCI orb — including its container, CI pipeline, and AI agent
integration — with no manual CircleCI authoring required.

The tool makes no assumptions about the source language or build system of the target CLI. Its
only requirement is a runnable binary. It is equally suited to first-party tools (run as part of
the tool's own build pipeline) and third-party tools (where only a binary is available).

---

## 2. Motivation

Packaging a CLI tool as a CircleCI orb is repetitive work: every tool needs the same executor
definition, the same job/command boilerplate, the same container build pipeline, and the same
release wiring. The pattern is identical across tools; only the command names, parameters, and
binary name differ.

`gen-circleci-orb` eliminates this repetition by treating the orb as a derived artefact of the
CLI's own `--help` output.

Secondary motivation: orbs published without a corresponding MCP server are invisible to AI coding
agents. By including `gen-orb-mcp` in the release chain, every generated orb ships with
first-class agent support from the first release.

---

## 3. High-Level Flow

```mermaid
flowchart TD
    CLI["CLI binary\n(any language)"]
    GCO["gen-circleci-orb"]
    ORB["CircleCI orb\nsrc/@orb.yml\ncommands/ jobs/\nexecutors/ examples/"]
    DOCKER["Dockerfile\n(embedded in orb repo)"]
    ORBCI[".circleci/\nconfig.yml\nrelease.yml\nupdate_prlog.yml"]
    MCP["gen-orb-mcp\n→ MCP server binary\n→ GitHub release asset"]
    REGISTRY["CircleCI orb registry\n(one or more namespaces)"]
    AGENTS["AI coding agents\n(Claude, Cursor, etc.)"]

    CLI -->|"--help parsing"| GCO
    GCO --> ORB
    GCO --> DOCKER
    GCO --> ORBCI

    ORB --> ORBCI
    ORBCI -->|"build CLI → crates.io\nbuild container → docker.io\norb-tools publish"| REGISTRY
    ORBCI -->|"post-publish"| MCP
    MCP --> AGENTS
```

---

## 4. Example Application

`gen-orb-mcp` is a CLI tool that generates MCP servers from CircleCI orb definitions. It has five
subcommands discovered by running `gen-orb-mcp --help`:

```
Commands:
  generate  Generate an MCP server from an orb definition
  validate  Validate an orb definition without generating
  diff      Compute conformance rules by diffing two orb versions
  migrate   Apply conformance-based migration to a consumer's .circleci/ directory
  prime     Populate prior-versions/ and migrations/ from git history
```

A developer wanting to expose `gen-orb-mcp` via CircleCI runs:

```bash
gen-circleci-orb generate \
  --binary gen-orb-mcp \
  --namespace jerus-org \
  --namespace digital-prstv \
  --output ./gen-orb-mcp-orb
```

`gen-circleci-orb` executes `gen-orb-mcp --help`, `gen-orb-mcp generate --help`,
`gen-orb-mcp validate --help`, etc., parses the output, and produces:

### 4.1 Generated orb entry point

```yaml
# gen-orb-mcp-orb/src/@orb.yml
version: 2.1
description: >
  Generate MCP servers from CircleCI orb definitions.
display:
  home_url: https://github.com/jerus-org/gen-orb-mcp
  source_url: https://github.com/jerus-org/gen-orb-mcp-orb
commands:
  generate: { ref: "commands/generate.yml" }
  validate: { ref: "commands/validate.yml" }
  diff:     { ref: "commands/diff.yml" }
  migrate:  { ref: "commands/migrate.yml" }
  prime:    { ref: "commands/prime.yml" }
jobs:
  generate: { ref: "jobs/generate.yml" }
  validate: { ref: "jobs/validate.yml" }
  diff:     { ref: "jobs/diff.yml" }
  migrate:  { ref: "jobs/migrate.yml" }
  prime:    { ref: "jobs/prime.yml" }
executors:
  default: { ref: "executors/default.yml" }
```

### 4.2 Generated orb command (example: `generate`)

Derived from `gen-orb-mcp generate --help` output. All leaf-level subcommands become commands.

```yaml
# gen-orb-mcp-orb/src/commands/generate.yml
description: Generate an MCP server from an orb definition.
parameters:
  orb_path:
    type: string
    description: "Path to the orb YAML file (e.g., src/@orb.yml)"
  output:
    type: string
    default: "./dist"
    description: "Output directory for generated server"
  format:
    type: enum
    default: "source"
    enum: ["binary", "source"]
    description: "Output format"
  name:
    type: string
    default: ""
    description: "Name for the generated orb server (defaults to filename)"
  version:
    type: string
    default: ""
    description: "Version for the generated MCP server crate"
  force:
    type: boolean
    default: false
    description: "Overwrite existing files without confirmation"
  migrations:
    type: string
    default: ""
    description: "Directory containing conformance rule JSON files to embed"
  prior_versions:
    type: string
    default: ""
    description: "Directory of prior orb version YAML snapshots to embed"
  tag_prefix:
    type: string
    default: "v"
    description: "Tag prefix used to discover the orb version from git tags"
steps:
  - run:
      name: gen-orb-mcp generate
      command: |
        gen-orb-mcp generate \
          --orb-path "<< parameters.orb_path >>" \
          --output "<< parameters.output >>" \
          --format "<< parameters.format >>" \
          <<# parameters.name >>--name "<< parameters.name >>"<</ parameters.name >> \
          <<# parameters.version >>--version "<< parameters.version >>"<</ parameters.version >> \
          <<# parameters.force >>--force<</ parameters.force >> \
          <<# parameters.migrations >>--migrations "<< parameters.migrations >>"<</ parameters.migrations >> \
          <<# parameters.prior_versions >>--prior-versions "<< parameters.prior_versions >>"<</ parameters.prior_versions >>
```

### 4.3 Generated orb job (example: `generate`)

All commands have a corresponding job one level up that wraps checkout + the command.

```yaml
# gen-orb-mcp-orb/src/jobs/generate.yml
description: Run gen-orb-mcp generate in a dedicated job.
executor: default
parameters:
  orb_path:
    type: string
  output:
    type: string
    default: "./dist"
  # ... same parameters as command ...
steps:
  - checkout
  - generate:
      orb_path: << parameters.orb_path >>
      output: << parameters.output >>
```

### 4.4 Generated executor

```yaml
# gen-orb-mcp-orb/src/executors/default.yml
description: Execution environment with gen-orb-mcp pre-installed.
docker:
  - image: jerusdp/gen-orb-mcp:<< parameters.tag >>
parameters:
  tag:
    type: string
    default: latest
```

### 4.5 Generated Dockerfile (embedded in orb repo)

```dockerfile
FROM ubuntu:24.04
RUN cargo binstall --no-confirm gen-orb-mcp
```

### 4.6 Full release pipeline (generated `.circleci/release.yml`)

The generated release workflow orchestrates the complete chain from CLI build to MCP publication:

```
1. build-cli       → cargo build --release
2. publish-crate   → cargo publish → crates.io          (Rust tools)
3. build-container → docker build → docker.io
4. publish-orb     → orb-tools publish → CircleCI registry (one job per namespace)
5. build-mcp       → gen-orb-mcp prime + generate + compile → GitHub release asset
```

```mermaid
flowchart LR
    BC["build-cli"] --> PC["publish-crate\n(crates.io)"]
    PC --> CONT["build-container\n(docker.io)"]
    CONT --> PO1["publish-orb\njerus-org"]
    CONT --> PO2["publish-orb\ndigital-prstv"]
    PO1 & PO2 --> MCP["build-mcp\n(GitHub release)"]
```

### 4.7 Dogfooding: gen-circleci-orb generating its own orb

The primary validation target is gen-circleci-orb itself. Once the tool has a `generate`
subcommand, it generates its own orb:

```bash
gen-circleci-orb generate \
  --binary gen-circleci-orb \
  --namespace jerus-org \
  --output ./gen-circleci-orb-orb
```

This creates a reference implementation demonstrating what the tool produces and serves as a
continuous integration test: the orb must stay consistent with the CLI's actual `--help` output.

---

## 5. Architecture Overview

```mermaid
flowchart LR
    subgraph "gen-circleci-orb binary"
        INTROSPECT["Help parser\n(--help execution\n+ output parsing)"]
        MODEL["CommandModel\n(language-agnostic IR)"]
        RENDER["Template renderer\n(diff-aware)"]
        OUTPUT["Output writer"]
    end

    subgraph "Inputs"
        BIN["CLI binary\n(any language)"]
        FLAGS["CLI flags\n--namespace (repeatable)\n--install-method\n--orb-tools-version\n--output\n--dry-run"]
    end

    subgraph "Output artefacts"
        OA["orb src/\n(commands/ jobs/\nexecutors/ examples/)"]
        OB["Dockerfile"]
        OC[".circleci/\n(3-file model)"]
    end

    BIN -->|"binary --help\nbinary <sub> --help"| INTROSPECT
    FLAGS --> INTROSPECT
    INTROSPECT --> MODEL
    MODEL --> RENDER
    RENDER --> OUTPUT
    OUTPUT --> OA & OB & OC
```

### 5.1 Help parser

Executes the target binary with `--help` to obtain the top-level command list and description,
then executes `<binary> <subcommand> --help` for each discovered subcommand, recursively. Produces
a normalised `CommandModel` regardless of the source CLI's language or build system. MVP targets
clap's help output format; best-effort mode applies to non-clap CLIs (see §6.2).

### 5.2 CommandModel

A language-agnostic intermediate representation:

```
CommandModel
├── binary_name: String
├── description: String
└── commands: Vec<Command>
    ├── name: String
    ├── description: String
    ├── is_leaf: bool          // true = generates command; false = generates job only
    ├── parameters: Vec<Parameter>
    │   ├── long_name: String  // e.g. "orb-path" → parameter name "orb_path"
    │   ├── short: Option<char>
    │   ├── param_type: ParamType  // String | Boolean | Enum(Vec<String>) | Integer
    │   ├── default: Option<String>
    │   ├── required: bool
    │   └── description: String
    └── subcommands: Vec<Command>   // recursive
```

### 5.3 Subcommand → orb element mapping

| CLI level | Orb element generated | Rationale |
|-----------|----------------------|-----------|
| Leaf subcommand (no children) | `commands/<name>.yml` | Maximum flexibility for composing custom jobs |
| Parent of leaf subcommands | `jobs/<name>.yml` (wraps its leaf commands) | Jobs provide the checkout + environment; leaf commands provide the steps |
| Top-level binary | `executors/default.yml`, `@orb.yml` | One executor per tool |

For a flat CLI (all subcommands are leaves, e.g. `gen-orb-mcp`), every subcommand gets both a
command and a job.

For a nested CLI (e.g. `tool server start`, `tool server stop`):
- `start` and `stop` → `commands/server_start.yml`, `commands/server_stop.yml`
- `server` → `jobs/server.yml` (with `action` enum parameter: `[start, stop]`)

*Future:* A custom job specification feature will allow users to define jobs that combine multiple
commands or add custom steps not derivable from the CLI structure.

### 5.4 Parameter type inference

Inferred from clap's structured help output:

| Signal in `--help` text | Inferred CircleCI type |
|------------------------|----------------------|
| `[possible values: a, b, ...]` | `enum` with listed values |
| Flag has no `<VALUE>` metavar (boolean presence flag) | `boolean` |
| `[default: <value>]` present | type inferred from default; adds `default:` to parameter |
| Metavar `<PATH>`, `<DIR>`, `<FILE>`, `<OUTPUT>` | `string` |
| All other cases | `string` (safe fallback) |

### 5.5 Template renderer

Walks the `CommandModel` and renders all output files. Diff-aware: files are only written if their
rendered content differs from the existing file, minimising noisy commits on regeneration runs.
`--dry-run` prints the diff without writing.

### 5.6 Output writer

Writes to `--output <dir>`. Fails on unrecognised existing files unless `--force` is passed.
On greenfield runs (empty output dir) prompts for `--orb-tools-version` if not supplied.
On brownfield runs (existing orb dir) reads the version from the existing `.circleci/config.yml`
unless explicitly overridden.

---

## 6. Detailed Design

### 6.1 Container installation method

Controlled by `--install-method <method>`:

| Method | Generated Dockerfile snippet | When to use |
|--------|------------------------------|-------------|
| `binstall` (default) | `RUN cargo binstall --no-confirm <name>` | Rust tool published to crates.io with binstall metadata |
| `apt` | `RUN apt-get install -y <name>` | Binary available in apt package repository |

Both methods install into a base image selected by `--base-image` (default: `ubuntu:24.04`).
The Dockerfile is embedded directly in the orb repo alongside the orb source; no separate
container repository is generated.

Additional install methods (GitHub release download, Homebrew, etc.) are deferred to the roadmap.

### 6.2 Help format handling

The parser targets clap's `--help` output format as the MVP baseline. Clap produces stable,
structured output including `[possible values: ...]` annotations, consistent flag/argument
formatting, and grouped sections.

For non-clap CLIs, a best-effort parser applies: it extracts subcommands and flags using
heuristics (indentation, leading `--`, presence of description text) but may miss type
information. In best-effort mode all parameters default to `type: string`.

### 6.3 Namespace publishing

`--namespace` is required and repeatable. Each namespace produces a separate `publish-orb-<ns>`
job in the generated release workflow. The orb name is always `<namespace>/<binary-name>`.

```bash
# Single namespace
gen-circleci-orb generate --binary gen-orb-mcp --namespace jerus-org --output ./out

# Multiple namespaces (parallel publish jobs)
gen-circleci-orb generate --binary gen-orb-mcp \
  --namespace jerus-org \
  --namespace digital-prstv \
  --output ./out
```

### 6.4 Diff-aware regeneration

On each run the renderer compares generated content against existing files:

1. Files with changed content → overwritten
2. Files with identical content → skipped (no write, no git change)
3. Files present in output but not in the new render → flagged as stale (not deleted by default;
   `--prune` removes them)

This keeps regeneration commits minimal and reviewable.

### 6.5 orb-tools version

Exposed as `--orb-tools-version <version>`. Behaviour:

- **Greenfield** (no existing `.circleci/`): required; prompted interactively if not supplied
- **Brownfield** (existing `.circleci/`): read from the current config and preserved unless
  explicitly overridden with `--orb-tools-version`

The version is embedded in the generated `.circleci/config.yml` as a pipeline parameter with a
default value, making future upgrades a one-line change.

### 6.6 MCP server generation (post-publish)

The generated `release.yml` includes a `build-mcp` job that runs after all `publish-orb-*` jobs
complete. It mirrors the `toolkit/build_mcp_server` job pattern:

1. `gen-orb-mcp prime` — populate `prior-versions/` and `migrations/` from git tag history
2. `gen-orb-mcp generate --format binary` — compile the MCP server binary
3. Upload binary to the GitHub release as an asset

This is the same pattern currently used by the `circleci-toolkit` orb itself.

---

## 7. Design Decisions

| # | Question | Decision |
|---|----------|----------|
| 1 | Subcommand → orb mapping | Leaf subcommands → commands; parents of leaves → jobs. Future: custom job specification feature. |
| 2 | Container install method | `cargo binstall` default; `apt` option. Others deferred to roadmap. |
| 3 | Container scope | Dockerfile embedded in the orb repo. No separate container repository. |
| 4 | Release pipeline | Full chain: CLI build → crates.io → docker.io → CircleCI registry (per namespace) → GitHub release (MCP binary). MVP targets these four registries/repositories. |
| 5 | MCP server placement | Post orb publish in CI (Option A). Mirrors `toolkit/build_mcp_server` pattern. |
| 6 | Namespace | `--namespace` flag, required, repeatable. Generates one publish job per namespace. |
| 7 | Regeneration | Diff-aware. Only changed files are written; `--prune` removes stale files. |
| 8 | orb-tools version | Exposed as `--orb-tools-version`. Prompted on greenfield; preserved from existing config on brownfield. |
| 9 | Help format | MVP targets Rust/clap. Best-effort mode for non-clap CLIs (all params default to `string`). |
| 10 | First validation target | gen-circleci-orb dogfoods itself. The generated orb for the `generate` subcommand is the reference implementation and continuous integration test. |

---

## 8. Roadmap (Deferred Items)

| Item | Notes |
|------|-------|
| Additional install methods | GitHub release download, Homebrew, custom install script |
| Alternative registries | npm, PyPI, Homebrew tap as orb executor sources |
| Custom job specification | Allow users to define jobs combining multiple commands or adding custom steps not derivable from CLI structure |
| Non-crates.io publishing | For non-Rust CLIs the crates.io step is skipped; future support for language-specific registries (npm, PyPI, etc.) |
| Separate container repo scaffolding | Option to generate a dedicated container repo (ci-container / zola-container pattern) for teams that prefer the separation |
| orb-tools version auto-update | Renovate-style automation to keep the pinned orb-tools version current in generated configs |
