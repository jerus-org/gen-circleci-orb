chmod +x "${WORKSPACE_BIN_PATH}/${NAME}"
echo "export PATH=${WORKSPACE_BIN_PATH}:\$PATH" >> "$BASH_ENV"
VERSION="${CIRCLE_TAG#${TAG_PREFIX}}"
echo "export VERSION=${VERSION}" >> "$BASH_ENV"
echo "export CIRCLE_BRANCH=main" >> "$BASH_ENV"
git fetch origin main
git checkout -B main origin/main
