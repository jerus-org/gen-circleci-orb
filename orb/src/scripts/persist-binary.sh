BINARY="${BINARY_OVERRIDE:-$PACKAGE}"
mkdir -p /tmp/workspace
if [[ -f "target/debug/$BINARY" ]]; then
  SRC="target/debug/$BINARY"
elif [[ -f "target/release/$BINARY" ]]; then
  # cargo_args passed --release (or -r), overriding the debug-profile default.
  SRC="target/release/$BINARY"
else
  echo "No compiled binary found for '$BINARY' in target/debug or target/release" >&2
  exit 1
fi
cp "$SRC" "/tmp/workspace/$BINARY"
