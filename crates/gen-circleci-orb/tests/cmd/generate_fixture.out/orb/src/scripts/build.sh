set -- fixture-cli build
set -- "$@" --name "${BUILD_NAME}"
[[ -n "${FORMAT:-}" ]] && set -- "$@" --format "${FORMAT}"
case "${LOG_LEVEL:-default}" in
  quiet) set -- "$@" --quiet ;;
  v) set -- "$@" --verbose ;;
  vv) set -- "$@" --verbose --verbose ;;
  vvv) set -- "$@" --verbose --verbose --verbose ;;
  vvvv) set -- "$@" --verbose --verbose --verbose --verbose ;;
esac
set -- "$@" "${TARGET}"
"$@"
