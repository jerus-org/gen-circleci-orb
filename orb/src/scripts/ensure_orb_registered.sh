CREATE_FLAGS="--no-prompt"
if [ "${PRIVATE}" = "1" ] || [ "${PRIVATE}" = "true" ]; then
  CREATE_FLAGS="--private --no-prompt"
fi

# Authenticate via env var — avoids `circleci setup` which was removed in
# newer CLI releases.
export CIRCLECI_CLI_TOKEN="${CIRCLE_TOKEN}"

set +e
circleci orb info "${ORB_NAME}"
orb_info_exit=$?
set -e
echo "orb info exit: ${orb_info_exit}"
if [ "${orb_info_exit}" -ne 0 ] && [ "${orb_info_exit}" -ne 255 ]; then
  set +e
  # shellcheck disable=SC2086
  create_output=$(circleci orb create "${ORB_NAME}" ${CREATE_FLAGS} 2>&1)
  create_exit=$?
  set -e
  echo "${create_output}"
  if [ "${create_exit}" -ne 0 ] && ! echo "${create_output}" | grep -q "already exists"; then
    exit "${create_exit}"
  fi
fi
echo "Orb is registered."
