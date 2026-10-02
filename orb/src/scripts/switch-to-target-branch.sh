git fetch origin "${TARGET_BRANCH}"
git checkout -B "${TARGET_BRANCH}" "origin/${TARGET_BRANCH}"
echo "export CIRCLE_BRANCH=\"${TARGET_BRANCH}\"" >> "$BASH_ENV"
