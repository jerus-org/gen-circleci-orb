<!--
SPDX-FileCopyrightText: 2026 jerusdp

SPDX-License-Identifier: MIT OR Apache-2.0
-->

# OpenSSF Best Practices — criterion → evidence

Working sheet for completing the [OpenSSF Best Practices Badge](https://www.bestpractices.dev/projects/13667)
questionnaire (project **13667**), targeting at least **Silver**. Silver requires the **Passing**
badge first, so both levels are mapped below.

Status key: **Met** · **N/A** (not applicable, with justification) · **Met (justified SHOULD gap)**.

Repository paths are relative to the repo root; the primary crate is `crates/gen-circleci-orb/`.

## Passing level

### Basics
| Criterion | Status | Evidence |
|-----------|--------|----------|
| description_good | Met | `README.md` / crate `README.md` overview — states what it does and the problem solved |
| interact | Met | README (install, links to issues + `CONTRIBUTING.md`) |
| contribution | Met | `CONTRIBUTING.md` — PR workflow |
| contribution_requirements | Met | `CONTRIBUTING.md` — coding standards, testing policy, DCO |
| floss_license | Met | Dual **MIT OR Apache-2.0** |
| floss_license_osi | Met | Both licenses are OSI-approved |
| license_location | Met | `LICENSE-MIT` / `LICENSE-APACHE` at repo root **and** crate dir |
| documentation_basics | Met | `README.md`, `docs/` (getting-started, user-guide, configuration) |
| documentation_interface | Met | Crate README CLI reference, `docs/user-guide.md`, and `--help` |
| sites_https | Met | GitHub, crates.io, docs.rs all serve over HTTPS/TLS |
| discussion | Met | GitHub Issues — searchable, URL-addressable, open participation |
| english | Met | All documentation is in English |
| maintained | Met | Active development (recent commits, releases, Renovate) |

### Change control
| Criterion | Status | Evidence |
|-----------|--------|----------|
| repo_public | Met | `https://github.com/jerus-org/gen-circleci-orb` |
| repo_track | Met | Git records author/date/change |
| repo_interim | Met | Full commit history + merged PRs, not only releases |
| repo_distributed | Met | Git |
| version_unique | Met | SemVer versions + `gen-circleci-orb-v*` tags |
| version_semver | Met | SemVer — declared in `PRLOG.md` / `CHANGELOG.md` |
| version_tags | Met | Signed `gen-circleci-orb-v*` git tags |
| release_notes | Met | `PRLOG.md` (workspace) + per-crate `crates/gen-circleci-orb/CHANGELOG.md` |
| release_notes_vulns | Met | `SECURITY.md` documents the `security:` commit convention → CHANGELOG **Security** section + advisory id in PR title → PRLOG. No CVE-assigned vulns fixed to date |

### Reporting
| Criterion | Status | Evidence |
|-----------|--------|----------|
| report_process | Met | GitHub Issues + `.github/ISSUE_TEMPLATE/` |
| report_tracker | Met | GitHub Issues |
| report_responses | Met | Maintainer responds to issues (see issue history) |
| enhancement_responses | Met | Enhancement issues triaged (see `ROADMAP.md` / issues #147/#148) |
| report_archive | Met | GitHub Issues public archive |
| vulnerability_report_process | Met | `SECURITY.md` |
| vulnerability_report_private | Met | GitHub **private** Security Advisories (Security tab) |
| vulnerability_report_response | Met | `SECURITY.md` commits to acknowledge within **3 business days** (≤14 days) |

### Quality
| Criterion | Status | Evidence |
|-----------|--------|----------|
| build | Met | `cargo build` (workspace) |
| build_common_tools | Met | Cargo |
| build_floss_tools | Met | Rust/Cargo toolchain is FLOSS |
| test | Met | `cargo test`; how-to in `CONTRIBUTING.md` / `justfile` |
| test_invocation | Met | `cargo test` (standard) |
| test_most | Met | **92.46% line coverage** (`cargo llvm-cov`) |
| test_continuous_integration | Met | CircleCI on every PR |
| test_policy | Met | `CONTRIBUTING.md` — testing policy |
| tests_are_added | Met | Recent PRs add tests (RED/GREEN TDD) |
| tests_documented_added | Met | `CONTRIBUTING.md` + `.github/PULL_REQUEST_TEMPLATE.md` tests checklist |
| warnings | Met | Clippy + compiler warnings |
| warnings_fixed | Met | CI + `just clippy` run `-D warnings` (zero warnings) |
| warnings_strict | Met | `clippy … -D warnings`, `RUSTDOCFLAGS="-D warnings"` |

### Security
| Criterion | Status | Evidence |
|-----------|--------|----------|
| know_secure_design | Met | `docs/assurance-case.md` (secure-design principles) |
| know_common_errors | Met | `docs/assurance-case.md` (threat model + mitigations: injection, cred leakage, MITM, supply chain) |
| crypto_published | Met | Only published algorithms — GPG, Sigstore, TLS (via `git2`) |
| crypto_call | Met | No home-grown crypto; delegates to `git2`/GPG/Sigstore |
| crypto_floss | Met | All crypto via FLOSS libraries |
| crypto_keylength | Met | Defaults from the underlying libraries meet NIST minimums |
| crypto_working | Met | No broken algorithms selected |
| crypto_weaknesses | Met | No dependence on SHA-1/MD5 etc. |
| crypto_pfs | N/A | No key-agreement protocol implemented (delegated to TLS libs, which provide PFS) |
| crypto_password_storage | N/A | The tool stores no passwords |
| crypto_random | Met | No custom key/nonce generation; delegated to Sigstore/rsign (secure RNG) |
| delivery_mitm | Met | Distribution over HTTPS (crates.io, GitHub); git over HTTPS |
| delivery_unsigned | Met | Releases are signed (GPG tags, SLSA/Sigstore, minisign); see `docs/RELEASING.md` |
| vulnerabilities_fixed_60_days | Met | `cargo audit`/`cargo deny` clean; only `RUSTSEC-2023-0071` ignored (transitive `rsa` timing side-channel, no upstream fix, not medium+ exploitable here) |
| vulnerabilities_critical_fixed | Met | No known critical vulnerabilities outstanding |
| no_leaked_credentials | Met | Secrets come from CI env/contexts; none in the repo (`docs/assurance-case.md`) |

### Analysis
| Criterion | Status | Evidence |
|-----------|--------|----------|
| static_analysis | Met | Clippy + SonarCloud (`sonar-project.properties`) |
| static_analysis_common_vulnerabilities | Met | SonarCloud + `cargo-audit`/`cargo-deny` |
| static_analysis_fixed | Met | Findings addressed; CI enforces |
| static_analysis_often | Met | Runs on every PR |
| dynamic_analysis | N/A | Memory-safe Rust, no `unsafe` in `src/`; static analysis covers the surface |
| dynamic_analysis_unsafe | N/A | Memory-safe language; no `unsafe` blocks |
| dynamic_analysis_enable_assertions | Met | Debug/test builds run with assertions enabled |
| dynamic_analysis_fixed | Met | None found to fix |

## Silver level

### Basics — project oversight & documentation
| Criterion | Status | Evidence |
|-----------|--------|----------|
| achieve_passing | Prerequisite | Complete the Passing level above first |
| dco | Met | `CONTRIBUTING.md` DCO section; commits use `git commit -s` |
| governance | Met | `GOVERNANCE.md` |
| code_of_conduct | Met | `CODE_OF_CONDUCT.md` (Contributor Covenant) |
| roles_responsibilities | Met | `GOVERNANCE.md` — roles table |
| access_continuity | Met | `GOVERNANCE.md` — access & continuity plan |
| bus_factor | Met (justified SHOULD gap) | Single maintainer; documented honestly in `GOVERNANCE.md` with mitigations (org ownership, automated processes) |
| documentation_roadmap | Met | `ROADMAP.md` (covers the next ~year) |
| documentation_architecture | Met | `docs/architecture.md` + `docs/design.md` |
| documentation_security | Met | `SECURITY.md` + `docs/assurance-case.md` |
| documentation_quick_start | Met | README quick start |
| documentation_current | Met | Docs kept current with the release |
| documentation_achievements | Met | OpenSSF badge displayed in root + crate READMEs |
| accessibility_best_practices | N/A | Developer CLI; no GUI/web UI |
| internationalization | N/A | Developer CLI; English-only interface by design |
| sites_password_security | N/A | No site with user passwords |

### Change control
| Criterion | Status | Evidence |
|-----------|--------|----------|
| maintenance_or_update | Met | `SECURITY.md` supported-versions + upgrade path (latest `0.1.x`) |

### Reporting
| Criterion | Status | Evidence |
|-----------|--------|----------|
| report_tracker | Met | GitHub Issues |
| vulnerability_report_credit | Met | `SECURITY.md` credits reporters unless anonymity requested |
| vulnerability_response_process | Met | `SECURITY.md` response process + timelines |

### Quality
| Criterion | Status | Evidence |
|-----------|--------|----------|
| coding_standards | Met | `CONTRIBUTING.md` — rustfmt, Clippy, Rust API Guidelines, Conventional Commits |
| coding_standards_enforced | Met | CI (`fmt`/`clippy`) + `just clippy -D warnings` |
| build_standard_variables | Met | Cargo honors `RUSTFLAGS`/`CC`/etc. |
| build_preserve_debug | Met | Cargo release/debug profiles preserve debug info as configured |
| build_non_recursive | Met | Cargo builds are non-recursive |
| build_repeatable | Met | `Cargo.lock` committed; digest-pinned CI images |
| installation_common | Met | `cargo install` / `cargo binstall` |
| installation_standard_variables | Met | Cargo honors `CARGO_INSTALL_ROOT` etc. |
| installation_development_quick | Met | `cargo build` / `just test` (see `CONTRIBUTING.md`) |
| external_dependencies | Met | `Cargo.toml` + `Cargo.lock` (machine-readable) |
| dependency_monitoring | Met | Renovate + `cargo-audit`/`cargo-deny` |
| updateable_reused_components | Met | Cargo dependencies, versioned |
| interfaces_current | Met | No deprecated APIs relied upon |
| automated_integration_testing | Met | CI runs the suite on every check-in |
| regression_tests_added50 | Met | `CONTRIBUTING.md` mandates a regression test per bug fix; followed in practice |
| test_statement_coverage80 | Met | **92.46%** line coverage (`cargo llvm-cov`) |
| test_policy_mandated | Met | `CONTRIBUTING.md` testing policy |
| tests_documented_added | Met | `CONTRIBUTING.md` + PR template |
| warnings_strict | Met | `clippy … -D warnings`, `RUSTDOCFLAGS="-D warnings"` |

### Security
| Criterion | Status | Evidence |
|-----------|--------|----------|
| implement_secure_design | Met | `docs/assurance-case.md` (§5 secure-design principles) |
| crypto_weaknesses | Met | No known-weak algorithms |
| crypto_algorithm_agility | Met | Algorithms provided by updatable libraries |
| crypto_credential_agility | Met | Signing keys/tokens via env vars; rotatable without recompiling |
| crypto_used_network | Met | HTTPS by default; SSH `insteadOf` neutralised (`set_https_remote`) |
| crypto_tls12 | Met | TLS ≥1.2 via `git2`/libgit2 |
| crypto_certificate_verification | Met | libgit2 verifies TLS certificates by default |
| crypto_verification_private | Met | Credentials only sent over verified HTTPS |
| signed_releases | Met | `docs/RELEASING.md` — GPG tags + SLSA/Sigstore attestation + minisign binary, with verification steps |
| version_tags_signed | Met | `release.toml` `sign-tag = true` |
| input_validation | Met | `docs/assurance-case.md` §6 — typed/allowlist parsing of `--help` + config |
| hardening | N/A | Developer CLI; no long-running network service to harden |
| assurance_case | Met | `docs/assurance-case.md` |

### Analysis
| Criterion | Status | Evidence |
|-----------|--------|----------|
| static_analysis_common_vulnerabilities | Met | SonarCloud + `cargo-audit`/`cargo-deny` |
| dynamic_analysis_unsafe | N/A | Memory-safe Rust; no `unsafe` in `src/` |

## Notes for the questionnaire

- The bestpractices.dev URL for the project is **https://www.bestpractices.dev/projects/13667**.
- For `bus_factor` (Silver SHOULD), select the honest answer and reference `GOVERNANCE.md`, which
  records the single-maintainer limitation and its mitigations.
- N/A answers above each carry a one-line justification to paste into the questionnaire's rationale.
