# Getting started with gen-circleci-orb

This guide takes you from installation to a running CI pipeline that auto-generates and
publishes a CircleCI orb for your Rust [clap](https://docs.rs/clap) CLI tool.

## Install

```bash
cargo binstall gen-circleci-orb
```

Or build from source:

```bash
cargo install gen-circleci-orb
```

## Set up with `init`

`init` is the entry point. It captures your setup once into a `gen-circleci-orb.toml`, runs
`generate`, and patches your CI — you do not run `generate` first. It is interactive: run it
with just the binary and it prompts for the required values it doesn't have (workflow names,
namespaces, tag prefix, contexts), each pre-filled with a sensible default.

```bash
gen-circleci-orb init --binary my-tool
```

Passing a flag skips its prompt, so the same command is fully scriptable (and non-interactive
under `--dry-run` or without a TTY) by supplying everything up front:

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

`--crate-tag-prefix` is the tag prefix your release pipeline uses for the crate
(e.g. `my-tool-v` matches tags like `my-tool-v1.2.3`). It filters the tag-triggered
release workflow so only crate release tags trigger orb publishing.

`--docker-namespace` is the Docker Hub (or registry) org for the built container image.
`--public-orb-namespace` (or `--private-orb-namespace`) is the CircleCI orb namespace —
these are independent and often differ.

This:
1. Runs `generate` to write `orb/`
2. Patches `.circleci/config.yml`:
   - Adds `gen-circleci-orb:` and `orb-tools:` to the `orbs:` section
   - Adds a `build-binary` + `regenerate-orb` job pair: builds the binary from source on
     every CI run, then re-generates the orb source so it always reflects the current binary
   - Adds `orb-tools/pack` and `orb-tools/review` steps to the validation workflow to
     verify the generated orb on every build
   - Adds a tag-triggered `orb-release:` workflow that fires on `<crate-tag-prefix>*` tags
     and runs the full release sequence using gen-circleci-orb orb jobs
3. Writes a `gen-circleci-orb.toml` recording every value, so later runs of `generate` and
   `update` reproduce the same orb and CI without re-passing flags. Commit it.

Preview what would change without writing anything:

```bash
gen-circleci-orb init --binary my-tool ... --dry-run
```

## Regenerate with `generate`

`generate` (re)writes the orb source from the binary's current `--help`. Once `init` has written
the config, it needs **no flags** — it reads the binary, namespaces, base image, and output
directory from `gen-circleci-orb.toml`:

```bash
gen-circleci-orb generate
```

This is exactly what the `regenerate-orb` CI job runs on every build. It writes the full unpacked
orb into the configured `orb/` subdirectory:

```
orb/
├── src/
│   ├── @orb.yml
│   ├── commands/
│   │   ├── subcommand-a.yml
│   │   └── subcommand-b.yml
│   ├── executors/
│   │   └── default.yml
│   └── jobs/
│       ├── subcommand-a.yml
│       └── subcommand-b.yml
└── Dockerfile
```

The orb source is always isolated in `<output>/<orb-dir>/` so it cannot be mixed with
existing project source (e.g. a Rust `src/`). If the target directory exists but does not
look like a CircleCI orb, the command refuses to write and reports an error.

You can also run `generate` **without** a config — for a quick look, or to publish an orb by hand
without CI automation — but then you supply the values (and need their defaults) explicitly:

```bash
gen-circleci-orb generate --binary my-tool --orb-namespace my-org
```

Verify the orb locally:

```bash
circleci orb pack orb/src > /tmp/my-tool-orb.yml
```

## On your next release

Once the CI changes are merged, pushing a tag matching `<crate-tag-prefix>*` triggers the
`orb-release:` workflow automatically:

1. `gen-circleci-orb/build_rust_binary` compiles the release binary
2. `orb-tools/pack` packs the orb source
3. `gen-circleci-orb/build_container` builds and pushes the Docker image tagged
   `:<version>` and `:latest`
4. `gen-circleci-orb/ensure_orb_registered` creates the orb in each namespace if it
   does not already exist
5. `orb-tools/publish` publishes the packed orb to each namespace

From that point on, every build keeps the orb in sync with the binary — no manual
orb maintenance required.

## Keep the generated wiring current

As gen-circleci-orb itself evolves, the canonical CI wiring it emits can change. `update`
re-syncs the managed blocks in `.circleci/config.yml` from your committed
`gen-circleci-orb.toml`, leaving your own jobs and customizations intact:

```bash
gen-circleci-orb update --check   # in CI: fail if the wiring is out of date
gen-circleci-orb update           # apply the re-sync locally
```

Add `update --check` to your validation workflow so a generator upgrade surfaces as a
failing check rather than silent drift. `update` never edits `gen-circleci-orb.toml` — it
only reads it; to change the wiring, edit `gen-circleci-orb.toml` and re-run `update` (or re-run
`init` to be re-prompted for the values).

## Keeping orb versions up to date

`init` writes current orb version pins on first run and does not update them on
subsequent runs. See
[docs/user-guide.md § Keeping CI up to date](user-guide.md#keeping-ci-up-to-date)
for the recommended Renovate setup and the MCP-assisted alternative.
