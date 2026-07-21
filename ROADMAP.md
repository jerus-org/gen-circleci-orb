<!--
SPDX-FileCopyrightText: 2026 jerusdp

SPDX-License-Identifier: MIT OR Apache-2.0
-->

# Roadmap

_Last updated: 2026-07-21._

This roadmap describes the intended direction of gen-circleci-orb over roughly the next year.
It is a statement of intent, not a commitment: priorities may shift with user feedback and
maintainer availability (see [GOVERNANCE.md](GOVERNANCE.md)). Concrete work is tracked in the
[issue tracker](https://github.com/jerus-org/gen-circleci-orb/issues); this document groups that
work into themes and horizons.

## Current status

gen-circleci-orb is **pre-1.0 (0.1.x)**. It generates a complete CircleCI orb from a clap-based
Rust CLI's `--help` output, and can wire the CI needed to keep the orb in sync. The CLI and the
`gen-circleci-orb.toml` configuration surface may still change ahead of 1.0.

## Near term (next ~2 quarters, through end of 2026)

- **Project hardening / OpenSSF Best Practices badge.** Complete the governance, security, and
  quality documentation and achieve (and display) at least the Silver badge.
- **Per-crate changelog & security recording.** Finish activating `CHANGELOG.md` generation and
  the `security:` commit convention for recording fixed vulnerabilities (see
  [SECURITY.md](SECURITY.md)).
- **[#147 — dynamic-config (setup workflow) wiring.](https://github.com/jerus-org/gen-circleci-orb/issues/147)**
  Optional generation of CircleCI dynamic-config / setup-workflow wiring.
- **[#148 — rename a managed job during `update`.](https://github.com/jerus-org/gen-circleci-orb/issues/148)**
  Allow `update` to rename a managed job in an existing consumer's CI without manual edits.
- **Reliability of `update`/`generate`** on real consumer repositories, driven by field feedback.

## Medium term (H1 2027) — toward 1.0

- **Stabilise the configuration schema and CLI.** Settle the `gen-circleci-orb.toml` schema and the
  command-line surface so that `0.1.x → 1.0` is a stability milestone with a documented migration.
- **Custom job specification.** Let users define jobs that combine multiple generated commands or
  add steps not derivable from the CLI structure (noted as future work in
  [docs/design.md](docs/design.md)).
- **Broader help-format support.** Improve best-effort parsing for non-clap and non-Rust CLIs,
  extending beyond the current clap-focused MVP.
- **Documentation completeness for 1.0**, including a stable configuration reference and upgrade
  guidance.

## Longer term (beyond 1.0)

- Deeper integration options with the wider jerus-org CI toolkit and orb ecosystem.
- Additional generated CI patterns as common needs emerge from real usage.

## How to influence the roadmap

Open an issue (feature request) or comment on an existing roadmap issue. Contributions that move
roadmap items forward are welcome — see [CONTRIBUTING.md](CONTRIBUTING.md).
