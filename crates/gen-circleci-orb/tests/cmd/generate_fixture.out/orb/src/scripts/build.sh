set -- fixture-cli build
set -- "$@" --name "${GCO_BUILD_NAME}"
[[ -n "${GCO_FORMAT:-}" ]] && set -- "$@" --format "${GCO_FORMAT}"
case "${GCO_LOG_LEVEL:-default}" in
  quiet) set -- "$@" --quiet ;;
  v) set -- "$@" --verbose ;;
  vv) set -- "$@" --verbose --verbose ;;
  vvv) set -- "$@" --verbose --verbose --verbose ;;
  vvvv) set -- "$@" --verbose --verbose --verbose --verbose ;;
esac
set -- "$@" "${GCO_TARGET}"
"$@"
