if ! grep -q 'AS cli-installer' "${ORB_DIR}/Dockerfile" 2>/dev/null; then
  echo "No cli-installer stage in ${ORB_DIR}/Dockerfile -- nothing to verify."
  exit 0
fi

echo "Building the cli-installer stage only (no Rust build, no push) to verify the circleci-cli pin"
docker build --target cli-installer -t gco-cli-pin-check "${ORB_DIR}"

if ! docker run --rm --entrypoint circleci gco-cli-pin-check orb --help >/dev/null; then
  echo "FATAL: 'circleci orb' does not work in the cli-installer stage -- the circleci_cli_version pin is likely broken (bad version, missing GitHub release, etc.). This is the exact class of bug that deadlocked gen-circleci-orb#326 -- caught now, at PR time, instead of at release time." >&2
  exit 1
fi

echo "circleci-cli pin OK"
