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
    mkdir -p /rootfs/etc/iroh-relay /rootfs/etc/ssl/certs; \
    cp /etc/ssl/certs/ca-certificates.crt /rootfs/etc/ssl/certs/ca-certificates.crt; \
    touch /rootfs/etc/iroh-relay/.keep

FROM scratch AS runtime

COPY --from=fetch --chown=0:0 /out/iroh-relay /usr/local/bin/iroh-relay
COPY --from=fetch --chown=0:0 /rootfs/etc /etc

LABEL org.opencontainers.image.title="zterm iroh relay" \
      org.opencontainers.image.description="Verified upstream iroh-relay binary in a minimal non-root runtime" \
      org.opencontainers.image.source="https://github.com/n0-computer/iroh"

USER 65532:65532

EXPOSE 38451/tcp

STOPSIGNAL SIGINT
ENTRYPOINT ["/usr/local/bin/iroh-relay"]
CMD ["--config-path", "/etc/iroh-relay/relay.toml"]
