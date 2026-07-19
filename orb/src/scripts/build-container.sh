VERSION="${CIRCLE_TAG#${CRATE_TAG_PREFIX}}"
docker build \
  -t "${DOCKER_NAMESPACE}/${BINARY}:${VERSION}" \
  -t "${DOCKER_NAMESPACE}/${BINARY}:latest" \
  "${ORB_DIR}"

# Fail-closed smoke gate (before push): if the Dockerfile installs the circleci
# CLI, the freshly-built image MUST be able to run `circleci orb`. This is the
# exact capability that silently went missing and deadlocked the 0.0.58/0.0.59
# releases (ensure_orb_registered needs `circleci orb`; a container without it
# blocks publish with no way to ship the fix). Verifying the just-built image
# here means a broken container never gets pushed and the release fails loudly
# instead of publishing a broken orb. Skipped for images that do not install the
# CLI (the grep is false), so generic consumers are unaffected.
if grep -q 'CIRCLECI_CLI_VERSION' "${ORB_DIR}/Dockerfile"; then
  echo "Dockerfile installs the circleci CLI — verifying 'circleci orb' works in the built image"
  if ! docker run --rm "${DOCKER_NAMESPACE}/${BINARY}:${VERSION}" circleci orb --help >/dev/null; then
    echo "FATAL: 'circleci orb' does not work in the freshly-built image; refusing to publish a broken orb container." >&2
    exit 1
  fi
fi

echo "${DOCKERHUB_PASSWORD}" | docker login -u "${DOCKERHUB_USERNAME}" --password-stdin
docker push "${DOCKER_NAMESPACE}/${BINARY}:${VERSION}"
docker push "${DOCKER_NAMESPACE}/${BINARY}:latest"
