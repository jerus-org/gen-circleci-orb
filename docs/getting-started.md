# Getting started with gen-circleci-orb

This guide takes you from installation to a running CI pipeline that auto-generates and
publishes a CircleCI orb for your CLI tool.

## Install

```bash
cargo binstall gen-circleci-orb
```

Or build from source:

```bash
cargo install gen-circleci-orb
```

## Generate an orb

Run from your project root, pointing `generate` at any binary on your `PATH`:

```bash
gen-circleci-orb generate \
  --binary my-tool \
  --orb-namespace my-org
```

The tool runs `my-tool --help` and `my-tool <subcommand> --help` for each subcommand,
then writes the full unpacked orb into an `orb/` subdirectory (the default `--orb-dir`):

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

Verify the orb locally:

```bash
circleci orb pack orb/src > /tmp/my-tool-orb.yml
```

## Wire into CI

Run `init` once from your repo root:

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
1. Runs `generate` to write `orb/` (same as the manual step above)
2. Patches `.circleci/config.yml`:
   - Adds `gen-circleci-orb:` and `orb-tools:` to the `orbs:` section
   - Adds a `build-binary` + `regenerate-orb` job pair: builds the binary from source on
     every CI run, then re-generates the orb source so it always reflects the current binary
   - Adds `orb-tools/pack` and `orb-tools/review` steps to the validation workflow to
     verify the generated orb on every build
   - Adds a tag-triggered `orb-release:` workflow that fires on `<crate-tag-prefix>*` tags
     and runs the full release sequence using gen-circleci-orb orb jobs

Preview what would change without writing anything:

```bash
gen-circleci-orb init ... --dry-run
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

## Keeping orb versions up to date

`init` writes current orb version pins on first run and does not update them on
subsequent runs. See
[docs/user-guide.md § Keeping CI up to date](user-guide.md#keeping-ci-up-to-date)
for the recommended Renovate setup and the MCP-assisted alternative.
