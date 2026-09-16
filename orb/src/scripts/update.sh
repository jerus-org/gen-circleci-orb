set -- gen-circleci-orb update
[[ -n "${GCO_CONFIG:-}" ]] && set -- "$@" --config "${GCO_CONFIG}"
[[ -n "${GCO_CI_DIR:-}" ]] && set -- "$@" --ci-dir "${GCO_CI_DIR}"
[[ "${GCO_CHECK:-false}" = "true" ]] && set -- "$@" --check
"$@"
