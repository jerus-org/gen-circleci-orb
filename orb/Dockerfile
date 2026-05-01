FROM debian:12-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates git \
    && rm -rf /var/lib/apt/lists/*
COPY gen-circleci-orb /usr/local/bin/gen-circleci-orb
