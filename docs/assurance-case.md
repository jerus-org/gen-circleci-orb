<!--
SPDX-FileCopyrightText: 2026 jerusdp

SPDX-License-Identifier: MIT OR Apache-2.0
-->

# Security Assurance Case

This document is the project's **assurance case**: a structured argument, with
supporting evidence, that gen-circleci-orb is adequately secure for its intended use.
It states the security requirements, the threat model, the secure-design principles
applied, and how inputs and cryptography are handled. It complements the reporting
process in [`SECURITY.md`](../SECURITY.md).

## 1. What the software is and does

gen-circleci-orb is a **developer-time command-line code generator**. Given a
clap-based Rust CLI binary, it:

1. Executes `<binary> --help` and, recursively, `<binary> <subcommand> --help`
   (`src/help_parser/`) to derive a structured `CliDefinition`.
2. Generates a CircleCI orb — commands, jobs, an executor, a Dockerfile — plus optional
   CI wiring to keep the orb in sync with the binary (`src/orb_generator/`,
   `src/ci_patcher/`, `src/output_writer/`).
3. Optionally records/commits/pushes the regenerated orb using GPG-signed commits via
   the `pcu` library (`src/commands/generate.rs`).

It runs on a developer workstation or in CI. It is **not** a network service and has no
users, sessions, or stored credentials of its own.

## 2. Security requirements

The security objectives, in priority order, are:

- **R1 — Integrity of generated output.** The generated orb / CI configuration must
  faithfully reflect the (trusted) input binary and configuration, and must be
  reviewable before it is committed or published.
- **R2 — Protection of credentials.** Signing keys and tokens used by the optional
  commit/push features must not be leaked into generated files, logs, or the repository.
- **R3 — Authenticity of releases.** Released artifacts (git tags, commits, and the
  published crate) must be verifiable as originating from the project.
- **R4 — Supply-chain integrity.** Third-party dependencies must be pinned, monitored,
  and free of known vulnerabilities to the extent practical.
- **R5 — Safe failure.** When inputs are malformed or the environment is
  misconfigured, the tool must fail safely (error out) rather than produce silently
  incorrect or dangerous output.

## 3. Trust boundaries and assumptions

| Boundary | Trusted? | Assumption |
|----------|----------|------------|
| The **target binary** whose `--help` is executed | **Untrusted by the tool; trusted by the user** | The operator points the tool only at binaries they already trust to run (see threat T1). |
| The **`gen-circleci-orb.toml` / project config** | Trusted | Authored and reviewed by the repository owner. |
| **Environment variables** carrying credentials (GPG key material, tokens) | Trusted, sensitive | Provided by the operator's secret-management/CI system. |
| **Third-party crates and orbs** | Semi-trusted | Pinned and monitored; see R4 / T4. |
| **The network** (git push / fetch via `git2`/`pcu`) | Untrusted transport | Confidentiality/integrity provided by TLS; see T5. |

## 4. Threat model

| ID | Threat | Mitigation |
|----|--------|------------|
| **T1** | **Arbitrary code execution** — the tool executes the target binary (`Command::new(binary)`), so pointing it at a malicious binary runs that code. | This is inherent to reading a program's `--help`. It is documented explicitly in `SECURITY.md` ("Only run gen-circleci-orb against binaries you trust"). The tool adds no additional privilege; it runs the binary with the operator's own permissions. |
| **T2** | **Injection into generated YAML** — hostile `--help` text or config could try to inject unintended CircleCI YAML. | Output is produced by typed serialization (`serde_yaml`) from an allowlist-parsed model, not by string concatenation of raw help text. Generated output is **reviewed in a diff** before commit and **validated** by the `orb-tools/pack` + `review` CI steps and the `regenerate-orb` `update --check` gate. |
| **T3** | **Credential leakage** — GPG key material / tokens exposed in output or logs. | Credentials are read from environment variables at point of use (`generate.rs`), passed to `pcu`/`git2` for signing, and never written into generated files or emitted to logs. In CI the signing key is ephemeral and scoped to a context. Satisfies R2. |
| **T4** | **Supply-chain compromise** — a malicious or vulnerable dependency. | `Cargo.lock` is committed; `cargo-audit` and `cargo-deny` (advisories, bans, licenses, sources) run in CI; Renovate keeps dependencies current; container base images are **digest-pinned**; sources are restricted to crates.io in `deny.toml`. Satisfies R4. |
| **T5** | **Man-in-the-middle on git operations** — tampering with fetch/push. | Network access goes through `git2`/`pcu` over **HTTPS with TLS certificate verification enabled by default**; the generated wiring's `set_https_remote` forces the origin to HTTPS so pushes use an authenticated App token rather than falling back to an SSH key. |
| **T6** | **Tampering with committed CI wiring / releases.** | Commits and tags are **GPG-signed** (`release.toml`: `sign-tag`, `sign-commit`), a CI job **verifies commit signatures**, and release artifacts carry **SLSA/sigstore attestation** with a documented verification process provided alongside the release tooling. Satisfies R3. |
| **T7** | **Malformed input causing incorrect output.** | Inputs are parsed into typed models with validation; unparseable help/config yields an error rather than partial output (R5). Idempotency is covered by an integration test (`generate_is_idempotent`). |

## 5. Secure-design principles applied

- **Least privilege.** The tool holds no ambient credentials; it uses only what the
  operator supplies via the environment, at the moment it is needed.
- **Defense in depth.** Multiple independent controls protect release integrity:
  signed commits/tags, signature verification in CI, dependency auditing, the
  `update --check` drift gate, and release attestation.
- **Fail safe / fail closed.** Malformed inputs and drifted CI wiring cause a non-zero
  exit rather than silent, possibly-unsafe output.
- **Economy of mechanism / no home-grown crypto.** All cryptography is delegated to
  vetted libraries (`git2`/libgit2 TLS, GPG for signing, sigstore for attestation); the
  project implements none of its own.
- **Complete mediation of output.** Generated artifacts are always subject to human
  review (diff) and automated validation before they take effect.

## 6. Input validation

All external input is converted to typed, validated models — an allowlist approach —
before use:

- **CLI arguments** are parsed and validated by `clap`.
- **`--help` text** from the target binary is parsed by `src/help_parser/` into a
  structured `CliDefinition`; only recognised structure is consumed. The raw text is
  never interpolated verbatim into generated YAML.
- **Configuration** (`gen-circleci-orb.toml`) is deserialized with `serde` into typed
  structures with known fields; unknown or malformed input is rejected.
- The tool takes **no untrusted network input**; git transport is handled by `git2`.

## 7. Cryptography posture

- **No custom cryptography.** Signing uses GPG (via `pcu`); transport uses the TLS stack
  in `git2`/libgit2; release attestation uses sigstore/Fulcio/Rekor.
- **Certificate verification** for TLS is enabled by default (libgit2 default); private
  information (push credentials) is only sent over verified HTTPS connections.
- **Credential agility.** Signing keys and tokens are supplied via environment variables
  and can be rotated without recompiling or changing the source; no key material is
  embedded in the binary or repository.
- **No known-weak algorithms** are selected by the project; algorithm choices are those
  of the underlying, actively-maintained libraries.

## 8. Residual risk

- **T1 (executing the target binary)** is accepted and documented: it is intrinsic to
  introspecting a program's help output. The mitigation is operator discipline (trust
  the binary), stated in `SECURITY.md`.
- **Bus factor of one** (single maintainer) is a project-continuity risk, addressed by
  organisation-level ownership and documented processes in [`GOVERNANCE.md`](../GOVERNANCE.md).

This assurance case is reviewed when the threat surface changes materially (new input
sources, new network or credential handling, or new release mechanisms).
