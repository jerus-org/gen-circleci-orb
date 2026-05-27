VERSION="${CIRCLE_TAG#${CRATE_TAG_PREFIX}}"
cp "/tmp/bin/${BINARY}" "${ORB_DIR}/${BINARY}"
docker build \
  -t "${DOCKER_NAMESPACE}/${BINARY}:${VERSION}" \
  -t "${DOCKER_NAMESPACE}/${BINARY}:latest" \
  "${ORB_DIR}"
echo "${DOCKERHUB_PASSWORD}" | docker login -u "${DOCKERHUB_USERNAME}" --password-stdin
docker push "${DOCKER_NAMESPACE}/${BINARY}:${VERSION}"
docker push "${DOCKER_NAMESPACE}/${BINARY}:latest"
