# syntax=docker/dockerfile:1.7@sha256:a57df69d0ea827fb7266491f2813635de6f17269be881f696fbfdf2d83dda33e

# This digest is the official multi-platform alpine:3.23.3 image index.
FROM alpine:3.23.3@sha256:25109184c71bdad752c8312a8623239686a9a2071e8825f20acb8f2198c3f659 AS fetch

ARG TARGETARCH

COPY artifact.sh /usr/local/share/zterm-relay/artifact.sh

RUN set -eu; \
    . /usr/local/share/zterm-relay/artifact.sh; \
    relay_arch="${TARGETARCH:-}"; \
    if [ -z "$relay_arch" ]; then \
        case "$(uname -m)" in \
            x86_64) relay_arch=amd64 ;; \
            aarch64) relay_arch=arm64 ;; \
            *) echo "unsupported build architecture: $(uname -m)" >&2; exit 64 ;; \
        esac; \
    fi; \
    iroh_relay_resolve "$relay_arch"; \
    wget -q -O /tmp/iroh-relay.tar.gz "$IROH_RELAY_URL"; \
    iroh_relay_verify /tmp/iroh-relay.tar.gz; \
    mkdir -p /out; \
    tar -xzf /tmp/iroh-relay.tar.gz -C /out; \
    test -x /out/iroh-relay; \
    /out/iroh-relay --version; \
    mkdir -p /rootfs/etc/iroh-relay /rootfs/etc/ssl/certs /rootfs/var/lib/iroh-relay; \
    cp /etc/ssl/certs/ca-certificates.crt /rootfs/etc/ssl/certs/ca-certificates.crt; \
    printf 'zterm:x:65532:65532:zterm relay:/nonexistent:/sbin/nologin\n' > /rootfs/etc/passwd; \
    printf 'zterm:x:65532:\n' > /rootfs/etc/group; \
    touch /rootfs/etc/iroh-relay/.keep; \
    touch /rootfs/var/lib/iroh-relay/.keep

# A purpose-built one-shot probe keeps the final image shell- and downloader-free
# while allowing Docker to verify a live HTTP endpoint. The builder is the exact
# project Rust release, pinned to the official multi-platform image index. It
# runs once on BUILDPLATFORM and cross-compiles the tiny static probe, avoiding
# a full Rust builder pull and emulation for every target architecture.
FROM --platform=$BUILDPLATFORM rust:1.98.0-alpine3.23@sha256:4743b6231029d726d7a0f81d730a7c9f4eff23225a4499c01e275efb5e260235 AS healthcheck

ARG TARGETARCH

COPY artifact.sh /usr/local/share/zterm-relay/artifact.sh
COPY healthcheck.rs /src/healthcheck.rs

RUN set -eu; \
    . /usr/local/share/zterm-relay/artifact.sh; \
    iroh_relay_resolve "${TARGETARCH:-}"; \
    rustup target add "$IROH_RELAY_TARGET"; \
    host_target="$(rustc -vV | sed -n 's/^host: //p')"; \
    linker="$(rustc --print sysroot)/lib/rustlib/$host_target/bin/rust-lld"; \
    test -x "$linker"; \
    mkdir -p /out; \
    rustc --edition=2024 --target "$IROH_RELAY_TARGET" -C linker="$linker" \
        -O -C strip=symbols -C target-feature=+crt-static \
        /src/healthcheck.rs -o /out/zterm-relay-healthcheck

FROM scratch AS runtime

COPY --from=fetch --chown=0:0 /out/iroh-relay /usr/local/bin/iroh-relay
COPY --from=healthcheck --chown=0:0 /out/zterm-relay-healthcheck /usr/local/bin/zterm-relay-healthcheck
COPY --from=fetch --chown=0:0 /rootfs/etc /etc
COPY --from=fetch --chown=65532:65532 /rootfs/var/lib/iroh-relay /var/lib/iroh-relay

LABEL org.opencontainers.image.title="zterm iroh relay" \
      org.opencontainers.image.description="Verified upstream iroh-relay binary in a minimal non-root runtime" \
      org.opencontainers.image.source="https://github.com/n0-computer/iroh"

USER 65532:65532
WORKDIR /var/lib/iroh-relay

EXPOSE 80/tcp 443/tcp 7842/udp 9090/tcp 38451/tcp

STOPSIGNAL SIGINT
ENTRYPOINT ["/usr/local/bin/iroh-relay"]
