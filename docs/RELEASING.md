<!--
SPDX-FileCopyrightText: 2026 jerusdp

SPDX-License-Identifier: MIT OR Apache-2.0
-->

# Releasing & verifying releases

This document describes how gen-circleci-orb releases are produced and signed, and — most
importantly for downstream users — **how to verify a release**. It satisfies the "signed releases
with a documented verification process" expectation of the
[OpenSSF Best Practices badge](https://www.bestpractices.dev/projects/13667).

## What is signed

A gen-circleci-orb release carries three independent, cryptographically verifiable signatures:

| Artifact | Signature | Trust anchor |
|----------|-----------|--------------|
| Git **tag** `gen-circleci-orb-v<version>` and its release commit | **GPG** signature | The maintainer's GPG public key |
| The published **`.crate`** (crates.io) | **SLSA v0.2 provenance attestation** via Sigstore *keyless* signing | Fulcio root CA + Rekor transparency log + the CircleCI OIDC build identity |
| The **binary tarball** `gen-circleci-orb-<target>.tar.gz` | **minisign/rsign** signature (`.tar.gz.sig`) | The per-release public key published in the crate's `Cargo.toml` |

## Release process (overview)

Releases run on CircleCI ([`.circleci/release.yml`](../.circleci/release.yml)):

1. `calculate_versions` — computes the next version(s) with `nextsv`; shown for review.
2. **Manual approval** gate — a reviewer approves the calculated version before anything is published.
3. `release_crate` — builds and **GPG-signs** the release commit + tag, builds the release binary,
   generates an **ephemeral minisign keypair**, **signs the tarball**, injects that keypair's public
   key into `Cargo.toml` (`pcu release inject-pubkey`), publishes to **crates.io**, and produces the
   **SLSA provenance attestation** (`pcu release attest`, Sigstore keyless via the CircleCI
   `sigstore`-audience OIDC token → Fulcio → Rekor).
4. Pushing the `gen-circleci-orb-v*` tag triggers the tag-gated `orb-release` workflow (pack, build
   container, register + publish the orb, build the MCP server).

The private GPG key and the ephemeral signing key live only in CI contexts, never in the repository.

## Verifying a release

### 1. The signed git tag

Import the maintainer's public key (e.g. from GitHub) and verify the tag:

```bash
curl -sL https://github.com/jerusdp.gpg | gpg --import
git verify-tag gen-circleci-orb-v<version>
git verify-commit gen-circleci-orb-v<version>^{commit}
```

GitHub also shows a **Verified** badge on the signed tag/commit.

### 2. The crate's SLSA / Sigstore attestation

Each release attaches `gen-circleci-orb-<version>.crate.sigstore.json` (the Sigstore bundle) and
`gen-circleci-orb-<version>.provenance.json` (the SLSA predicate). Verify without any extra tooling:

```bash
# Download the crate and the bundle
VER=<version>
curl -sL -o "gen-circleci-orb-$VER.crate" \
  "https://crates.io/api/v1/crates/gen-circleci-orb/$VER/download"
gh release download "gen-circleci-orb-v$VER" --repo jerus-org/gen-circleci-orb \
  --pattern "*.sigstore.json"

# (a) The bundle's messageDigest must equal the crate's SHA-256
sha256sum "gen-circleci-orb-$VER.crate"
python3 -c "import json,base64; b=json.load(open('gen-circleci-orb-$VER.crate.sigstore.json')); \
  print(base64.b64decode(b['messageSignature']['messageDigest']['digest']).hex())"

# (b) The Fulcio signing certificate binds the signature to this project's CI build identity
python3 -c "import json,base64; b=json.load(open('gen-circleci-orb-$VER.crate.sigstore.json')); \
  open('leaf.der','wb').write(base64.b64decode( \
  b['verificationMaterial']['x509CertificateChain']['certificates'][0]['rawBytes']))"
openssl x509 -inform DER -in leaf.der -noout -text | grep -A2 "Subject Alternative Name"
```

The two SHA-256 values in (a) must match. In (b) the certificate's SAN embeds the CircleCI pipeline
identity and the OIDC issuer `https://oidc.circleci.com/org/<org-id>`, proving the crate was signed by
this project's release pipeline. The signing event is also recorded in the public **Rekor**
transparency log referenced in the bundle.

### 3. The binary tarball signature

From **v0.1.3 onwards**, `cargo binstall` verifies the tarball automatically using the minisign public
key published in the crate's `Cargo.toml` (`[package.metadata.binstall.signing]`):

```bash
cargo binstall gen-circleci-orb          # verifies the .sig before installing
```

To verify by hand, obtain that version's public key from its `Cargo.toml` on crates.io and:

```bash
rsign verify -P "<pubkey>" -x gen-circleci-orb-<target>.tar.gz.sig gen-circleci-orb-<target>.tar.gz
```

> **Note:** releases up to and including **v0.1.2** attach a `.tar.gz.sig` but did **not** publish the
> signing public key (the `Cargo.toml` signing section was missing, so `pcu release inject-pubkey` had
> nothing to populate). Their tarball signature is therefore not verifiable; the crate SLSA
> attestation and the GPG-signed tag remain the verifiable trust anchors for those versions. This is
> fixed from v0.1.3.

## Trust model summary

- **GPG** — a long-lived maintainer key; verify against the maintainer's published public key.
- **Sigstore (crate)** — *keyless*: there is no long-lived key to trust; trust derives from the Fulcio
  CA, the Rekor transparency log, and the CircleCI OIDC identity embedded in the short-lived
  certificate.
- **minisign (binary)** — an ephemeral key generated per release; its public key is published in the
  released crate's `Cargo.toml` and used by `cargo binstall`.
