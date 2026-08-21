FROM rust:1.98.0-bookworm

RUN apt-get update \
    && apt-get install --yes --no-install-recommends \
        ca-certificates \
        iproute2 \
        nftables \
        procps \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /workspace

ENV CARGO_TARGET_DIR=/tmp/zterm-foundation-target

CMD ["cargo", "test", "-p", "zterm-daemon", "--test", "iroh_network_gate", "--", "--ignored", "--nocapture"]
