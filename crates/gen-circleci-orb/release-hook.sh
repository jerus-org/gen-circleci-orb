#!/bin/bash
set -exo pipefail
gen-changelog generate \
    --display-summaries \
    --name "CHANGELOG.md" \
    --package "gen-circleci-orb" \
    --repository-dir "../.." \
    --next-version "${NEW_VERSION:-${1}}"

# Refresh the third-party license notices so every release ships current
# attribution — the same release-time assurance exercise as the changelog above.
# This runs from the crate directory, where about.toml / about.hbs live and where
# THIRD-PARTY-LICENSES.md is packaged.
#
# Guarded until cargo-about is guaranteed in the release container
# (jerus-org/ci-container#541 released + this repo's rust_image digest bumped).
# Once it always is, drop the guard so a missing tool fails the release.
if command -v cargo-about >/dev/null 2>&1; then
    cargo about generate about.hbs --output-file THIRD-PARTY-LICENSES.md
else
    echo "WARN: cargo-about not installed; skipping THIRD-PARTY-LICENSES.md refresh" >&2
fi
