apt-get update -qq && apt-get install -y --no-install-recommends libssl-dev pkg-config
NAME_UNDERSCORED=$(echo "${NAME}" | tr '-' '_')
gen-orb-mcp generate \
  --format binary \
  --name "${NAME}" \
  --orb-path "${ORB_PATH}" \
  --output /tmp/mcp-server \
  --crate-version "${VERSION}" \
  --force \
  --prior-versions "${PRIOR_VERSIONS_DIR}" \
  --migrations "${MIGRATIONS_DIR}"
