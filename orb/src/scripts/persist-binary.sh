BINARY="${BINARY_OVERRIDE:-$PACKAGE}"
mkdir -p /tmp/workspace
cp "target/release/$BINARY" "/tmp/workspace/$BINARY"
