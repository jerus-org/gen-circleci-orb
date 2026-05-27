NAME_UNDERSCORED=$(echo "${NAME}" | tr '-' '_')
gen-orb-mcp publish \
  --binary "/tmp/mcp-server/target/release/${NAME_UNDERSCORED}_mcp" \
  --asset-name "${NAME_UNDERSCORED}_mcp-linux-x86_64" \
  --tag "${CIRCLE_TAG}"
