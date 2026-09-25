set -- gen-circleci-orb update
[[ ! "${GCO_CONFIG:-}" =~ ^[[:space:]]*$ ]] && set -- "$@" --config "${GCO_CONFIG}"
[[ ! "${GCO_CI_DIR:-}" =~ ^[[:space:]]*$ ]] && set -- "$@" --ci-dir "${GCO_CI_DIR}"
[[ "${GCO_CHECK:-false}" = "true" ]] && set -- "$@" --check
"$@"
