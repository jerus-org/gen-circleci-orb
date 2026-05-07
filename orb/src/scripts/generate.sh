set -- gen-circleci-orb generate \
  --binary "${BINARY}" \
  --orb-namespace "${ORB_NAMESPACE}" \
  --output "${OUTPUT}" \
  --install-method "${INSTALL_METHOD}" \
  --base-image "${BASE_IMAGE}" \
  --orb-dir "${ORB_DIR}"

[ -n "${HOME_URL:-}" ]   && set -- "$@" --home-url "${HOME_URL}"
[ -n "${SOURCE_URL:-}" ] && set -- "$@" --source-url "${SOURCE_URL}"
[ "${DRY_RUN:-false}" = "true" ] && set -- "$@" --dry-run

"$@"
