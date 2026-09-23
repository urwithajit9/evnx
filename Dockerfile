# evnx — the CLI, as a container.
#
# ⚠️ This image does NOT build evnx. It packages the same static musl binary
# that `release.yml` already produced and that people download from the GitHub
# Release, so `docker run evnx --version` and a downloaded tarball are byte for
# byte the same program. Building here would mean two compilers, two sets of
# flags, and a second thing to keep honest.
#
# Built by the `docker` job in .github/workflows/release.yml, which puts one
# binary per architecture in the build context. To build it yourself:
#
#   cargo build --release --all-features --target x86_64-unknown-linux-musl
#   cp target/x86_64-unknown-linux-musl/release/evnx ./evnx-bin-amd64
#   docker build -t evnx .

FROM alpine:3.21

# ca-certificates is not optional: `evnx cloud` and `evnx migrate` speak TLS,
# and without a trust store every request fails with a certificate error that
# looks like a network problem.
RUN apk add --no-cache ca-certificates \
    && adduser -D -u 10001 evnx

# TARGETARCH is set by buildx (amd64 / arm64) and is how one Dockerfile picks
# the right prebuilt binary for each platform of a multi-arch build. The default
# keeps a plain `docker build` working without buildx.
ARG TARGETARCH=amd64
COPY evnx-bin-${TARGETARCH} /usr/local/bin/evnx
RUN chmod 0755 /usr/local/bin/evnx

# /work is where the caller mounts their project:
#   docker run --rm -v "$PWD:/work" ghcr.io/urwithajit9/evnx scan
#
# ⚠️ Commands that WRITE — sync, init, add, template, backup — need the caller's
# own uid, or the container writes files the host user cannot edit:
#
#   docker run --rm --user "$(id -u):$(id -g)" -v "$PWD:/work" \
#     ghcr.io/urwithajit9/evnx sync --direction forward
#
# Read-only commands (scan, validate, diff, convert to stdout) need nothing.
WORKDIR /work

# ⚠️ Non-root. evnx reads .env files and writes .env.example; it never needs
# root, and a container that runs as root will write files the host user then
# cannot edit. uid 10001 is high enough not to collide with a host account.
USER evnx

# ENTRYPOINT, not CMD, so `docker run evnx scan .` passes `scan .` to evnx
# rather than replacing it. `--help` is the default because a bare
# `docker run evnx` should explain itself, not sit waiting on a prompt.
ENTRYPOINT ["/usr/local/bin/evnx"]
CMD ["--help"]

LABEL org.opencontainers.image.title="evnx" \
      org.opencontainers.image.description="Manage .env files — validation, secret scanning, format conversion, encrypted cloud sync" \
      org.opencontainers.image.url="https://www.evnx.dev" \
      org.opencontainers.image.source="https://github.com/urwithajit9/evnx" \
      org.opencontainers.image.licenses="MIT"
