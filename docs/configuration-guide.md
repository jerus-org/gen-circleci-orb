# Configuration guide

`gen-circleci-orb.toml` is the single source of truth for the generated orb and its CI. `init`
writes it, and `generate` and `update` read it — so the flags you supply once at `init` are
recorded here and never re-typed. It is plain TOML, safe to commit and review.

This guide covers the file `init` writes and the simple ways to tune generation. For composing a
single complex job from several commands, see the
[Advanced Configuration Guide](advanced-configuration.md).

## The sections `init` writes

| Section | Purpose |
|---------|---------|
| `[orb]` | The orb's own source and container |
| `[ci]` | Workflow and job wiring for the release pipeline |
| `[record]` | Optional auto-record of the regenerated orb source |

## `[orb]` — the orb's source and container

```toml
[orb]
binary = "my-tool"                 # binary to introspect (its --help drives generation)
namespaces = ["my-org"]            # CircleCI orb namespace(s)
orb_dir = "orb"                    # output subdirectory
base_image = "debian:13-slim"      # FROM for the orb's own generated Dockerfile
builder_image = "rust:1-slim-trixie"   # image for the Dockerfile's binstall builder stage
```

`base_image` / `builder_image` configure the **orb's own container** — the image your orb's
consumers run. Do not confuse them with `[ci].rust_image`, which is the image the *CI build jobs*
compile in (below).

### `apt_packages` — extra OS packages in the executor

Extra apt packages installed into the generated Dockerfile's **runtime** stage, alongside the
baseline (`ca-certificates`, `git`) and sorted together. Use for OS-level runtime dependencies of
the orb's binary (e.g. a tool that shells out to `cargo` needs `libssl-dev` + `pkg-config`).

```toml
[orb]
apt_packages = ["libssl-dev", "pkg-config"]
```

Also settable per-run with the repeatable `--apt-packages` flag (CLI overrides the config value).

### `cargo_tools` — extra cargo tools in the executor

Extra cargo tools to install into the executor image, for orbs whose executor **orchestrates other
cargo commands** (e.g. a security gate that runs `cargo-audit` and `cargo-deny`). Each entry is a
crate name; the generated Dockerfile installs `cargo-binstall` in the builder stage (from crates.io,
not a `curl | bash` installer), `cargo binstall`s the listed crates, and copies their binaries into
the runtime. The binaries land on `PATH` under their crate name (e.g. `cargo-audit`), so the runtime
needs no Rust toolchain to invoke them.

```toml
[orb]
cargo_tools = ["cargo-audit", "cargo-deny"]
```

Also settable with the repeatable `--cargo-tool` flag (CLI overrides the config value). Supported
with the **binstall** install method only — it relies on the Dockerfile's Rust builder stage; using
it with `apt`/`local` is an error. It assumes each tool's binary shares its crate name (true for
`cargo-*` tools generally).

### `crate_wait_attempts` / `crate_wait_seconds` — the crates.io propagation gate

```toml
[orb]
crate_wait_attempts = 40   # default: 40 (maximum 240)
crate_wait_seconds = 15    # default: 15  → 39 sleeps, so ~9m45s of waiting
```

The orb's container is built from the crate that was **just** published, so the build races the
crates.io sparse index. The generated Dockerfile retries `cargo install <binary> --version
"${CRATE_VERSION}"` on a bounded loop. The bound and the loud failure are deliberate: without the
pinned version the install would silently resolve the *previous* release and ship a container whose
binary does not match its own tag.

Only an index delay is waited out. A **build** failure — `cargo install` reporting `failed to
compile` — stops the loop immediately, because it is deterministic: retrying it recompiles the whole
crate every attempt and buries the compiler error N repetitions deep.

Between attempts the loop drops cargo's *local* sparse-index cache, turning the next lookup into a
full request rather than a conditional one. Cargo revalidates per invocation anyway, and this cannot
clear staleness at the CDN edge — it is cheap insurance on the one layer the build controls, not the
mechanism that makes waiting work.

If the gate expires the release stalls **half-published** — the crate is on crates.io, the container
was never built, and the orb was never published — and recovering means re-running the tag's release
workflow by hand. Raise these if a release keeps outrunning the window; the default is twice the
window that proved too short in practice.

A zero is ignored (the gate is not optional), and `crate_wait_attempts` is capped at 240 — an hour
at the default interval. A window long enough to outlast the CI job timeout is a hang, not a window,
and a hang is a worse outcome than the loud failure the gate exists to produce.

### `git_push_subcommands` — subcommands that push to git

Some tools have a subcommand that pushes to git (committing generated artifacts back, say). List
each such subcommand here:

```toml
[orb]
git_push_subcommands = ["save"]    # every subcommand of this tool that pushes to git
```

This is a **per-subcommand** setting, not an on/off flag, and it has two effects:

1. Each subcommand you list gets a `set_https_remote` step inserted into *its own* generated job
   (checkout → attach_workspace → `set_https_remote` → the command). `set_https_remote` strips the
   `insteadOf` ssh→https rewrite that CircleCI's `checkout` injects and points `origin` at HTTPS,
   so that job's push authenticates by token instead of being rewritten back to SSH.
2. As a byproduct, the shared `set_https_remote` **command** is generated. A composed job can
   reference it too — see the [Advanced Configuration Guide](advanced-configuration.md).

It is a list because a tool may have more than one push subcommand; list them all. `set_https_remote`
is generated whenever the list is non-empty, independently of whether the listed subcommand's job
is later suppressed (see `generate_job`).

### `custom_files`

Authorises hand-authored orb files the generator does not produce so they survive the prune step.
Covered in the [Advanced Configuration Guide](advanced-configuration.md#escape-hatches-extra_job-and-custom_files).

### `short_param` — name a short-only option

```toml
[subcommand.check.short_param]
f = "force"          # the CLI's `-f`, exposed to the orb as the `force` parameter
n = "repeat_count"
```

An option with a long form names its orb parameter after it (`--advisory-db` → `advisory_db`).
An option with **only** a short form (`-f`) has nothing to name after, so the generator takes the
first word of its description (`-f  Force the operation` → `force`). Where that word names nothing
useful — `-n  How many times` → `how` — generation fails and asks for a name here. Set an entry to
override a derived name too.

The option is still passed to the CLI by its short flag; only the orb parameter is named.

### `allow_unparsed_help` — generate despite a gap in the parse

```toml
[orb]
allow_unparsed_help = true   # default: false
```

Generation **fails** when the binary's `--help` declares an option or argument that produced no
orb parameter. The alternative — the behaviour before this guard existed — is a generated job that
silently cannot supply an input the CLI requires, with `generate` reporting success and
`update --check` reporting the wiring up to date, because the wiring *is* fine and it is the orb
content that is wrong.

The error names the declaration it could not turn into a parameter. Prefer fixing the CLI's help
shape (or reporting the shape so the generator can learn it). Set this to `true` only to keep
shipping while that happens: the missing input is then logged as a warning instead, and the orb
goes out with the gap.

## `[ci]` — release-pipeline wiring

```toml
[ci]
build_workflow = "validation"      # validation workflow to patch
release_workflow = "release"       # release workflow to patch
requires_job = "common-tests"      # job regenerate-orb should require
release_after_job = "release-my-tool"
crate_tag_prefix = "my-tool-v"     # tags that trigger the orb-release workflow
docker_namespace = "my-docker-org"
docker_context = "docker-credentials"   # context holding Docker Hub creds
orb_context = "orb-publishing"          # context holding orb publish creds
rust_image = "my-org/ci-rust:pinned@sha256:…"   # image the CI build jobs compile in
```

`rust_image` sets the image the `build-binary` / `orb-release-binary` jobs compile in. The default
`rust:latest` has no libclang; set a clang-equipped, digest-pinned image here when the workspace
pulls a bindgen-based `-sys` crate. This is the CI pipeline's image, distinct from the orb's own
`[orb].base_image` / `builder_image`.

If you pin it, note that `update` copies the value into the `rust_image:` lines of the generated
CI config, so the pin is committed in two places and both must be bumped together — unlike
`[orb].base_image` / `builder_image`, whose only artifact (`orb/Dockerfile`) is regenerated from
this file on every run. See [Container image pins](user-guide.md#container-image-pins) for how to
configure a pin-management tool to keep the two in step.

MCP integration (`--mcp`) adds `mcp`, `mcp_context`, `mcp_earliest_version`, and
`gen_orb_mcp_orb_version` here.

## `[record]` — auto-record the regenerated orb

Optional. When enabled, the `regenerate-orb` job commits the freshly regenerated orb source back
(GPG-signed) so the published orb stays in sync with the CLI. It stores only the **names** of the
env vars holding the signing material — the secret values live in the CI contexts:

```toml
[record]
enabled = true
gpg_key_env = "MY_GPG_KEY"         # names, not secrets
gpg_trust_env = "MY_GPG_TRUST"
user_name_env = "MY_USER_NAME"
user_email_env = "MY_USER_EMAIL"
signing_key_env = "MY_SIGN_KEY"
push_ssh_fingerprint = "SHA256:…"  # a public key hash, not a secret
contexts = ["my-release-context"]
```

## How CLI inputs become orb parameters

Every option and argument in the binary's `--help` becomes an orb parameter:

| In `--help` | Orb parameter | Passed to the CLI as |
|---|---|---|
| `--advisory-db <PATH>` | `advisory_db` | `--advisory-db "$ADVISORY_DB"` |
| `-p, --orb-path <PATH>` | `orb_path` | `--orb-path "$ORB_PATH"` |
| `-f` (no long form) | named by [`short_param`](#short_param--name-a-short-only-option) or the description | `-f` |
| `<VERSION>` under `Arguments:` | `version` | appended bare, after every option |
| `[TARGET]` under `Arguments:` | `target` | appended only when set |

Positionals are appended **after** all options, in the order clap lists them — a positional emitted
between an option and its value would be read as that value. A variadic positional (`<PATHS>...`)
becomes a single string parameter; pass multiple values as one space-separated string.

A required input (one the `Usage:` line lists outside `[...]`) generates a parameter with no
default, so CircleCI rejects a workflow that omits it.

## Tuning what gets generated

### Suppress a standalone job — keep the command

`generate_job = false` drops a subcommand's standalone **job** but keeps its **command** and
script. Use it for subcommands that only make sense inside a composed job, or that you simply do
not want to expose:

```toml
[subcommand.save]
generate_job = false
```

`gen-circleci-orb config suppress-job save` writes this for you (`unsuppress-job` reverts it).

### Exclude a subcommand entirely

`interactive = true` is stronger: it drops the command, job, **and** script (and, for a parent, its
whole subtree). Use it for interactive/CLI-only subcommands that have no place in CI. `init` and
`config` are interactive by default; set `interactive = false` to expose them.

```toml
[subcommand.login]
interactive = true
```

### Rename a run step

By default a command's run step is named after its short `--help` line, then its bare name. Set a
`label` for a readable name:

```toml
[subcommand.generate]
label = "Generate and compile the server"
```

### Override a parameter default

```toml
[subcommand.generate.param.output]
default = "./dist"
```

`gen-circleci-orb config set-parameter-default --subcommand generate --parameter output --value ./dist`
writes the same thing.

### Pin extra orbs

```toml
[orbs]
"some-org/helper" = "1.2.3"
```

### Compose a simple job

`[[job_group]]` in simple mode combines a few commands into one job with their shared parameters
wired through automatically:

```toml
[[job_group]]
name = "check_and_report"
steps = ["validate", "report"]
```

`gen-circleci-orb config add-job-group --name check_and_report --steps validate,report` writes it.
For goal-oriented jobs with explicit parameters, built-ins, custom scripts, and third-party-orb
steps, see the [Advanced Configuration Guide](advanced-configuration.md).

## See Also

- [Advanced Configuration Guide](advanced-configuration.md) — composing a single complex job
- [Getting Started](getting-started.md) — install to running pipeline
