set -- gen-circleci-orb add-job-group
set -- "$@" --name "${ADD_JOB_GROUP_NAME}"
[[ -n "${STEPS:-}" ]] && set -- "$@" --steps "${STEPS}"
[[ -n "${DESCRIPTION:-}" ]] && set -- "$@" --description "${DESCRIPTION}"
[[ -n "${PARAMS:-}" ]] && set -- "$@" --params "${PARAMS}"
"$@"
