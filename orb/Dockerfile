FROM rust:1-slim-trixie@sha256:31ee7fc65186be7e0e0ccb3f2ca305f14e4739e7642a1ae65753aa5d7b874523 AS builder
RUN apt-get update \
    && apt-get install -y --no-install-recommends build-essential ca-certificates clang cmake libssl-dev pkg-config \
    && rm -rf /var/lib/apt/lists/* \
    && cargo install gen-circleci-orb --locked

FROM debian:13-slim AS cli-installer
ARG CIRCLECI_CLI_VERSION=0.1.38646
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && cd /tmp \
    && TARBALL="circleci-cli_${CIRCLECI_CLI_VERSION}_linux_amd64.tar.gz" \
    && curl -fLSs --proto '=https' "https://github.com/CircleCI-Public/circleci-cli/releases/download/v${CIRCLECI_CLI_VERSION}/${TARBALL}" -o "${TARBALL}" \
    && curl -fLSs --proto '=https' "https://github.com/CircleCI-Public/circleci-cli/releases/download/v${CIRCLECI_CLI_VERSION}/circleci-cli_${CIRCLECI_CLI_VERSION}_checksums.txt" -o checksums.txt \
    && grep "${TARBALL}" checksums.txt | sha256sum --check \
    && tar -xzf "${TARBALL}" --strip 1 \
    && install -m 755 circleci /usr/local/bin/circleci \
    && rm -rf "${TARBALL}" checksums.txt circleci

FROM rust:1-slim-trixie@sha256:31ee7fc65186be7e0e0ccb3f2ca305f14e4739e7642a1ae65753aa5d7b874523
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates git gnupg libssl-dev openssh-client pkg-config \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -ms /bin/bash circleci
COPY --from=builder /usr/local/cargo/bin/gen-circleci-orb /usr/local/bin/gen-circleci-orb
COPY --from=cli-installer /usr/local/bin/circleci /usr/local/bin/circleci
USER circleci
WORKDIR /home/circleci/project
