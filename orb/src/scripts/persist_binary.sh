BINARY="${BINARY_OVERRIDE:-<< parameters.package >>}"
mkdir -p /tmp/workspace
cp "target/release/$BINARY" "/tmp/workspace/$BINARY"
