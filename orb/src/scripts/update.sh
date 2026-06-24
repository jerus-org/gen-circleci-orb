set -- gen-circleci-orb update
[[ -n "${CONFIG:-}" ]] && set -- "$@" --config "${CONFIG}"
[[ -n "${CI_DIR:-}" ]] && set -- "$@" --ci-dir "${CI_DIR}"
[[ "${CHECK:-false}" = "true" ]] && set -- "$@" --check
"$@"
