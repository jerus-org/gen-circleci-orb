CREATE_FLAGS="--no-prompt"
if [ "${PRIVATE}" = "1" ] || [ "${PRIVATE}" = "true" ]; then
  CREATE_FLAGS="--private --no-prompt"
fi

set +e
circleci setup --token "${CIRCLE_TOKEN}" --host https://circleci.com --no-prompt
setup_exit=$?
set -e
if [ "${setup_exit}" -ne 0 ] && [ "${setup_exit}" -ne 255 ]; then
  echo "circleci setup failed with exit ${setup_exit}" >&2
  exit "${setup_exit}"
fi

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
