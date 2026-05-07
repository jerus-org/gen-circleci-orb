set -- gen-circleci-orb init \
  --binary "${BINARY}" \
  --build-workflow "${BUILD_WORKFLOW}" \
  --release-workflow "${RELEASE_WORKFLOW}" \
  --orb-dir "${ORB_DIR}" \
  --ci-dir "${CI_DIR}" \
  --orb-tools-version "${ORB_TOOLS_VERSION}" \
  --docker-orb-version "${DOCKER_ORB_VERSION}" \
  --docker-namespace "${DOCKER_NAMESPACE}" \
  --docker-context "${DOCKER_CONTEXT}" \
  --orb-context "${ORB_CONTEXT}"

[ -n "${PUBLIC_ORB_NAMESPACE:-}" ]  && set -- "$@" --public-orb-namespace "${PUBLIC_ORB_NAMESPACE}"
[ -n "${PRIVATE_ORB_NAMESPACE:-}" ] && set -- "$@" --private-orb-namespace "${PRIVATE_ORB_NAMESPACE}"
[ -n "${REQUIRES_JOB:-}" ]          && set -- "$@" --requires-job "${REQUIRES_JOB}"
[ -n "${RELEASE_AFTER_JOB:-}" ]     && set -- "$@" --release-after-job "${RELEASE_AFTER_JOB}"
[ "${MCP:-false}" = "true" ]        && set -- "$@" --mcp
[ "${DRY_RUN:-false}" = "true" ]    && set -- "$@" --dry-run

"$@"
