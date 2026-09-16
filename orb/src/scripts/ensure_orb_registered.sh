set -- gen-circleci-orb ensure-orb-registered
set -- "$@" --orb-name "${GCO_ORB_NAME}"
[[ "${GCO_PRIVATE:-false}" = "true" ]] && set -- "$@" --private
"$@"
