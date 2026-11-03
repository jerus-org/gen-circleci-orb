FROM rust:1-slim-trixie@sha256:b8e062b1c3a80183df83d9ba187cadd2b5ab84bc78d453dbfeb2fccd5f982247 AS builder
ARG CRATE_VERSION
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
    build-essential \
    ca-certificates \
    clang \
    cmake \
    libssl-dev \
    pkg-config \
    && rm -rf /var/lib/apt/lists/* \
    && { err=/tmp/cargo-install.err; n=0; s=15; \
       until cargo install gen-circleci-orb --locked \
             --version "${CRATE_VERSION}" 2>"$err"; do \
         cat "$err" >&2; \
         grep -q "failed to compile" "$err" \
           && echo "build failed, not an index delay" >&2 && exit 1; \
         n=$((n+1)); \
         if [ "$n" -ge 16 ]; then \
           echo "crates.io index never served ${CRATE_VERSION} after 16 tries (~57m)" >&2; \
           echo "polled: https://index.crates.io/ge/n-/gen-circleci-orb" >&2; \
           echo "The crate is published, but this container was not built" >&2; \
           echo "and the orb was not published." >&2; \
           echo "Once the index serves it, re-run the tag's workflow from the failed job." >&2; \
           echo "The wait is capped at an hour in total; the waits are set by [orb] crate_wait_*." >&2; \
           exit 1; \
         fi; \
         j=$(od -An -N1 -tu1 /dev/urandom 2>/dev/null); \
         w=$((s + ${j:-0} % (s / 5 + 1))); \
         echo "waiting for crates.io index: ${CRATE_VERSION} (try $n, next in ${w}s)"; \
         rm -rf "${CARGO_HOME:-/usr/local/cargo}"/registry/index/*/.cache; \
         sleep "$w"; \
         s=$((s * 2)); [ "$s" -le 300 ] || s=300; \
       done; }

FROM debian:13-slim AS cli-installer
ARG CIRCLECI_CLI_VERSION=1.13.0
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && cd /tmp \
    && TARBALL="circleci-cli_${CIRCLECI_CLI_VERSION}_linux_amd64.tar.gz" \
    && BASE="https://github.com/CircleCI-Public/circleci-cli/releases" \
    && REL="${BASE}/download/v${CIRCLECI_CLI_VERSION}" \
    && curl -fLSs --proto '=https' \
         "${REL}/${TARBALL}" -o "${TARBALL}" \
    && curl -fLSs --proto '=https' \
         "${REL}/circleci-cli_${CIRCLECI_CLI_VERSION}_checksums.txt" \
         -o checksums.txt \
    && grep "${TARBALL}" checksums.txt | sha256sum --check \
    && tar -xzf "${TARBALL}" circleci \
    && install -m 755 circleci /usr/local/bin/circleci \
    && rm -rf "${TARBALL}" checksums.txt circleci

FROM rust:1-slim-trixie@sha256:b8e062b1c3a80183df83d9ba187cadd2b5ab84bc78d453dbfeb2fccd5f982247
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
    ca-certificates \
    git \
    gnupg \
    libssl-dev \
    openssh-client \
    pkg-config \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -ms /bin/bash circleci
COPY --from=builder /usr/local/cargo/bin/gen-circleci-orb /usr/local/bin/gen-circleci-orb
COPY --from=cli-installer /usr/local/bin/circleci /usr/local/bin/circleci
USER circleci
WORKDIR /home/circleci/project
