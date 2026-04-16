#!/bin/bash
set -exo pipefail
gen-changelog generate \
    --display-summaries \
    --name "CHANGELOG.md" \
    --package "gen-circleci-orb" \
    --repository-dir "../.." \
    --next-version "${NEW_VERSION:-${1}}"
