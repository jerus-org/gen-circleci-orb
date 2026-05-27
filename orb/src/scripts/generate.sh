set -- gen-circleci-orb generate
set -- "$@" --binary "${BINARY}"
set -- "$@" --orb-namespace "${ORB_NAMESPACE}"
[[ -n "${OUTPUT:-}" ]] && set -- "$@" --output "${OUTPUT}"
[[ -n "${INSTALL_METHOD:-}" ]] && set -- "$@" --install-method "${INSTALL_METHOD}"
[[ -n "${BASE_IMAGE:-}" ]] && set -- "$@" --base-image "${BASE_IMAGE}"
[[ -n "${HOME_URL:-}" ]] && set -- "$@" --home-url "${HOME_URL}"
[[ -n "${SOURCE_URL:-}" ]] && set -- "$@" --source-url "${SOURCE_URL}"
[[ -n "${ORB_DIR:-}" ]] && set -- "$@" --orb-dir "${ORB_DIR}"
[[ -n "${GIT_PUSH_SUBCOMMAND:-}" ]] && set -- "$@" --git-push-subcommand "${GIT_PUSH_SUBCOMMAND}"
[[ -n "${CIRCLECI_CLI_VERSION:-}" ]] && set -- "$@" --circleci-cli-version "${CIRCLECI_CLI_VERSION}"
[[ "${DRY_RUN:-false}" = "true" ]] && set -- "$@" --dry-run
"$@"
