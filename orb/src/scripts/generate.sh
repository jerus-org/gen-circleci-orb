set -- gen-circleci-orb generate
[[ -n "${GCO_BINARY:-}" ]] && set -- "$@" --binary "${GCO_BINARY}"
[[ -n "${GCO_ORB_NAMESPACE:-}" ]] && set -- "$@" --orb-namespace "${GCO_ORB_NAMESPACE}"
[[ -n "${GCO_OUTPUT:-}" ]] && set -- "$@" --output "${GCO_OUTPUT}"
[[ -n "${GCO_INSTALL_METHOD:-}" ]] && set -- "$@" --install-method "${GCO_INSTALL_METHOD}"
[[ -n "${GCO_BASE_IMAGE:-}" ]] && set -- "$@" --base-image "${GCO_BASE_IMAGE}"
[[ -n "${GCO_HOME_URL:-}" ]] && set -- "$@" --home-url "${GCO_HOME_URL}"
[[ -n "${GCO_SOURCE_URL:-}" ]] && set -- "$@" --source-url "${GCO_SOURCE_URL}"
[[ -n "${GCO_ORB_DIR:-}" ]] && set -- "$@" --orb-dir "${GCO_ORB_DIR}"
[[ -n "${GCO_GIT_PUSH_SUBCOMMAND:-}" ]] && set -- "$@" --git-push-subcommand "${GCO_GIT_PUSH_SUBCOMMAND}"
[[ -n "${GCO_CIRCLECI_CLI_VERSION:-}" ]] && set -- "$@" --circleci-cli-version "${GCO_CIRCLECI_CLI_VERSION}"
[[ -n "${GCO_APT_PACKAGES:-}" ]] && set -- "$@" --apt-packages "${GCO_APT_PACKAGES}"
[[ -n "${GCO_CARGO_TOOL:-}" ]] && set -- "$@" --cargo-tool "${GCO_CARGO_TOOL}"
[[ "${GCO_DRY_RUN:-false}" = "true" ]] && set -- "$@" --dry-run
[[ -n "${GCO_CONFIG:-}" ]] && set -- "$@" --config "${GCO_CONFIG}"
[[ "${GCO_NO_RECORD:-false}" = "true" ]] && set -- "$@" --no-record
[[ "${GCO_CHECK:-false}" = "true" ]] && set -- "$@" --check
[[ "${GCO_ALLOW_MAIN_RECORD:-false}" = "true" ]] && set -- "$@" --allow-main-record
"$@"
