set -- fixture-cli push
[[ -n "${GCO_TAG:-}" ]] && set -- "$@" --tag "${GCO_TAG}"
"$@"
