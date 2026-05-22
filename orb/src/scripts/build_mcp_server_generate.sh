NAME_UNDERSCORED=$(echo "${NAME}" | tr '-' '_')
gen-orb-mcp generate \
  --format binary \
  --name "${NAME}" \
  --orb-path "${ORB_PATH}" \
  --output /tmp/mcp-server \
  --version "${VERSION}" \
  --force \
  --prior-versions "${PRIOR_VERSIONS_DIR}" \
  --migrations "${MIGRATIONS_DIR}"
