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
