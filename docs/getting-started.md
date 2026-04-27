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
  --namespace my-org
```

The tool runs `my-tool --help` and `my-tool <subcommand> --help` for each subcommand, then
writes the full unpacked orb into an `orb/` subdirectory (the default `--orb-dir`):

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
circleci orb validate /tmp/my-tool-orb.yml
```

## Wire into CI

Run `init` once from your repo root:

```bash
gen-circleci-orb init \
  --binary my-tool \
  --namespace my-org \
  --build-workflow validation \
  --release-workflow release \
  --requires-job common-tests \
  --release-after-job release-my-tool
```

This:
1. Runs `generate` to write `orb/` (same as the manual step above)
2. Patches `.circleci/config.yml`:
   - Adds `orb-tools: circleci/orb-tools@12.3.3` to the `orbs:` section
   - Adds a `regenerate-orb` job that re-generates the orb on every CI run
   - Adds `orb-tools/pack` and `orb-tools/validate` steps to the validation workflow
3. Patches `.circleci/release.yml`:
   - Adds `docker: circleci/docker@3.2.0` and `orb-tools: circleci/orb-tools@12.3.3`
   - Adds a `build-container` job that builds and pushes the Docker image on release
   - Adds an `orb-tools/publish` step to publish the orb

Preview what would change without writing anything:

```bash
gen-circleci-orb init ... --dry-run
```

## On your next release

Once the CI changes are merged, your release pipeline will automatically:

1. Build the Docker image tagged with the release version
2. Push it to `jerusdp/my-tool:<version>` on Docker Hub
3. Publish the orb to `my-org/my-tool` on the CircleCI registry

From that point on, every build keeps the orb in sync with the binary — no manual
orb maintenance required.
