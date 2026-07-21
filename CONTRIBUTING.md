<!--
SPDX-FileCopyrightText: 2026 jerusdp

SPDX-License-Identifier: MIT OR Apache-2.0
-->

# Contributing to gen-circleci-orb

Thank you for your interest in contributing! This project welcomes bug reports,
feature requests, documentation improvements, and code contributions. This guide
explains how to contribute and the standards a contribution must meet to be
accepted.

By participating in this project you agree to abide by our
[Code of Conduct](CODE_OF_CONDUCT.md).

## Ways to contribute

- **Report a bug** — open an issue using the *Bug report* template.
- **Request a feature** — open an issue using the *Feature request* template.
- **Report a security vulnerability** — **do not** open a public issue. Use GitHub's
  private vulnerability reporting at
  <https://github.com/jerus-org/gen-circleci-orb/security/advisories/new>.
  See [SECURITY.md](SECURITY.md) for the full policy (response times, disclosure,
  and crediting).
- **Improve documentation** — docs live in [`docs/`](docs/) and in the crate
  `README.md`.
- **Submit code** — see the workflow below.

## Development environment

Requirements:

- A recent stable Rust toolchain (the project's MSRV is declared as `rust-version`
  in `Cargo.toml`).
- [`just`](https://github.com/casey/just) for task automation (optional but
  recommended).
- The [`circleci`](https://circleci.com/docs/local-cli/) CLI for packing/validating
  generated orbs locally (optional).

Quick start:

```bash
git clone https://github.com/jerus-org/gen-circleci-orb.git
cd gen-circleci-orb
cargo build            # build the workspace
just test              # clippy + check + doc + unit tests (see justfile)
```

## Coding standards

All contributions must comply with the project's coding standards. These are
enforced automatically in CI; please run them locally before opening a pull
request:

- **Formatting** — code MUST be formatted with `rustfmt`:
  `cargo fmt --all --check`.
- **Linting** — code MUST be free of Clippy warnings:
  `cargo clippy --all --tests --all-features -- -D warnings`.
- **Idioms** — follow the
  [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/) and idiomatic
  Rust conventions.
- **Documentation** — public items should be documented; doc builds are warning-free
  (`RUSTDOCFLAGS="-D warnings" cargo doc --all --no-deps`).
- **Commit messages** — follow the
  [Conventional Commits](https://www.conventionalcommits.org/) format
  (`feat:`, `fix:`, `docs:`, `chore:`, `refactor:`, …). These drive changelog
  generation. Use the **`security:`** type for fixes to security vulnerabilities:
  `gen-changelog` routes `security` (and `dependency`, and `chore(deps)`) commits to a
  dedicated **Security** section in `CHANGELOG.md`. For a vulnerability fix, put the
  advisory identifier (GHSA / RUSTSEC / CVE) in the commit subject **and** in the pull
  request title so it is recorded in both `CHANGELOG.md` and `PRLOG.md`. See
  [SECURITY.md](SECURITY.md).
- **Test placement** — put `#[cfg(test)]` modules at the **end** of the file.

## Testing policy

Testing is mandatory, not optional:

- **Test-driven development** — write failing tests first (RED), then the minimum
  implementation to make them pass (GREEN), then verify the full suite.
- **New functionality** — every major new feature MUST be accompanied by automated
  tests covering the new behaviour.
- **Bug fixes** — every bug fix MUST add a regression test that fails before the fix
  and passes after it. This proves the fix and prevents recurrence.
- Run the full suite before submitting: `cargo test` (including the CLI tests,
  `cargo test --test cli_tests`).

Pull requests that add or change behaviour without corresponding tests will be asked
to add them before merge.

## AI-assisted contributions

AI and large-language-model tools may be used to help produce contributions — this
project itself is developed with such assistance. Two conditions apply:

- **You remain fully accountable.** You are responsible for everything you submit,
  regardless of how it was produced. Review and understand the change, ensure it
  meets the coding and testing standards above, and confirm you have the right to
  submit it under the project's license. Your DCO sign-off (below) certifies this.
- **Disclosure is required.** If AI/LLM tooling assisted in creating a contribution,
  state so in the pull request description (the pull request template includes a
  field for this). Undisclosed AI-assisted contributions may be asked to be
  re-submitted with disclosure.

## Pull request workflow

1. **Never commit directly to `main`.** Create a feature branch from an up-to-date
   `main`:
   ```bash
   git checkout main && git pull
   git checkout -b <type>/<short-description>
   ```
   Use a branch prefix matching the change: `feat/`, `fix/`, `docs/`, `chore/`,
   `refactor/`.
2. Make your change with tests and documentation.
3. Run the local checks (format, clippy, tests) — see above.
4. **Sign off your commits** to certify the Developer Certificate of Origin (see
   below): `git commit -s`.
5. Push and open a pull request with a Conventional Commits title and a clear
   description of what changed and why. Fill in the pull request template.
6. Ensure CI is green. A maintainer will review; address feedback by pushing
   additional commits to the branch.

Maintainers review and merge pull requests. Please be patient and responsive to
review comments.

## Developer Certificate of Origin (DCO)

Contributions to this project are certified under the
[Developer Certificate of Origin](https://developercertificate.org/). You certify
that you wrote the contribution or otherwise have the right to submit it under the
project's license.

To certify, sign off every commit:

```bash
git commit -s -m "feat: add new thing"
```

This appends a `Signed-off-by: Your Name <your.email@example.com>` line to the
commit message. Configure your identity with `git config user.name` and
`git config user.email` so the sign-off matches. Pull requests whose commits are not
signed off cannot be merged.

## Licensing

This project is dual-licensed under **MIT OR Apache-2.0**. By contributing, you agree
that your contributions will be licensed under the same terms. See
[LICENSE-MIT](crates/gen-circleci-orb/LICENSE-MIT) and
[LICENSE-APACHE](crates/gen-circleci-orb/LICENSE-APACHE).
