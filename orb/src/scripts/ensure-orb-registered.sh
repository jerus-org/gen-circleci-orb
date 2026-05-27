set -- gen-circleci-orb ensure-orb-registered
set -- "$@" --orb-name "${ORB_NAME}"
[[ "${PRIVATE:-false}" = "true" ]] && set -- "$@" --private
"$@"