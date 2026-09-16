if ! git fetch origin main >/dev/null 2>&1; then
  echo "Could not fetch origin/main - running the smoke test to be safe."
elif git diff --name-only origin/main...HEAD 2>/dev/null | grep -qE "^(${ORB_DIR}/Dockerfile|gen-circleci-orb\.toml)\$"; then
  echo "A relevant file changed on this branch - running the cli-installer smoke test."
else
  echo "Neither ${ORB_DIR}/Dockerfile nor gen-circleci-orb.toml changed on this branch - nothing to do."
  circleci-agent step halt
fi
