set -- fixture-cli push
[[ -n "${TAG:-}" ]] && set -- "$@" --tag "${TAG}"
"$@"
