set -- gen-circleci-orb generate
[[ ! "${GCO_BINARY:-}" =~ ^[[:space:]]*$ ]] && set -- "$@" --binary "${GCO_BINARY}"
[[ ! "${GCO_CONFIG:-}" =~ ^[[:space:]]*$ ]] && set -- "$@" --config "${GCO_CONFIG}"
[[ ! "${GCO_OUTPUT:-}" =~ ^[[:space:]]*$ ]] && set -- "$@" --output "${GCO_OUTPUT}"
[[ ! "${GCO_ORB_DIR:-}" =~ ^[[:space:]]*$ ]] && set -- "$@" --orb-dir "${GCO_ORB_DIR}"
[[ ! "${GCO_ORB_NAMESPACE:-}" =~ ^[[:space:]]*$ ]] && set -- "$@" --orb-namespace "${GCO_ORB_NAMESPACE}"
[[ ! "${GCO_HOME_URL:-}" =~ ^[[:space:]]*$ ]] && set -- "$@" --home-url "${GCO_HOME_URL}"
[[ ! "${GCO_SOURCE_URL:-}" =~ ^[[:space:]]*$ ]] && set -- "$@" --source-url "${GCO_SOURCE_URL}"
[[ ! "${GCO_INSTALL_METHOD:-}" =~ ^[[:space:]]*$ ]] && set -- "$@" --install-method "${GCO_INSTALL_METHOD}"
[[ ! "${GCO_BASE_IMAGE:-}" =~ ^[[:space:]]*$ ]] && set -- "$@" --base-image "${GCO_BASE_IMAGE}"
[[ ! "${GCO_CIRCLECI_CLI_VERSION:-}" =~ ^[[:space:]]*$ ]] && set -- "$@" --circleci-cli-version "${GCO_CIRCLECI_CLI_VERSION}"
[[ ! "${GCO_APT_PACKAGES:-}" =~ ^[[:space:]]*$ ]] && set -- "$@" --apt-packages "${GCO_APT_PACKAGES}"
[[ ! "${GCO_CARGO_TOOL:-}" =~ ^[[:space:]]*$ ]] && set -- "$@" --cargo-tool "${GCO_CARGO_TOOL}"
[[ ! "${GCO_GIT_PUSH_SUBCOMMAND:-}" =~ ^[[:space:]]*$ ]] && set -- "$@" --git-push-subcommand "${GCO_GIT_PUSH_SUBCOMMAND}"
[[ "${GCO_DRY_RUN:-false}" = "true" ]] && set -- "$@" --dry-run
[[ "${GCO_NO_RECORD:-false}" = "true" ]] && set -- "$@" --no-record
[[ "${GCO_CHECK:-false}" = "true" ]] && set -- "$@" --check
[[ "${GCO_ALLOW_MAIN_RECORD:-false}" = "true" ]] && set -- "$@" --allow-main-record
"$@"
