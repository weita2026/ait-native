# syntax=docker/dockerfile:1.7@sha256:a57df69d0ea827fb7266491f2813635de6f17269be881f696fbfdf2d83dda33e
FROM docker.io/library/debian:bookworm-slim@sha256:abd67ffcfa541b485a3dff59865ab629aa048a6c613e639d36e7456b0b229241

ARG TARGETARCH
ARG AIT_RELEASE_SOURCE_COMMIT
ARG AIT_RELEASE_VERSION

LABEL org.opencontainers.image.description="AIT Native self-hosted server" \
      org.opencontainers.image.licenses="AGPL-3.0-only" \
      org.opencontainers.image.revision="${AIT_RELEASE_SOURCE_COMMIT}" \
      org.opencontainers.image.source="https://github.com/weita2026/ait-native" \
      org.opencontainers.image.title="ait-server" \
      org.opencontainers.image.version="${AIT_RELEASE_VERSION}"

COPY --chmod=0755 bin/${TARGETARCH}/ait-server /usr/local/bin/ait-server
COPY licenses/ /usr/share/licenses/ait-server/
COPY --chmod=0644 provenance.json /usr/share/ait-native/provenance.json
COPY --chown=65532:65532 runtime/ /var/lib/ait/

ENV AIT_NATIVE_SERVER_DATA=/var/lib/ait/server-data \
    HOME=/var/lib/ait
USER 65532:65532
EXPOSE 8088
ENTRYPOINT ["/usr/local/bin/ait-server"]
