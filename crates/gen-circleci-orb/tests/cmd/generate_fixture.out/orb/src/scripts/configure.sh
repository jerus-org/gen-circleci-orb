set -- fixture-cli configure
[[ -n "${GCO_CONFIG_PATH:-}" ]] && set -- "$@" --config-path "${GCO_CONFIG_PATH}"
"$@"
