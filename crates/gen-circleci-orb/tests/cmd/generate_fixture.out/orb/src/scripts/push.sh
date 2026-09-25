set -- fixture-cli push
[[ ! "${GCO_TAG:-}" =~ ^[[:space:]]*$ ]] && set -- "$@" --tag "${GCO_TAG}"
"$@"
