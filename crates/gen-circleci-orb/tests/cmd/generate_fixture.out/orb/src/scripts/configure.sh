set -- fixture-cli configure
[[ -n "${CONFIG_PATH:-}" ]] && set -- "$@" --config-path "${CONFIG_PATH}"
"$@"
