set -- fixture-cli configure
[[ ! "${GCO_CONFIG_PATH:-}" =~ ^[[:space:]]*$ ]] && set -- "$@" --config-path "${GCO_CONFIG_PATH}"
"$@"
