# Sandbox image for running ecluse in hybrid mode inside a Daytona sandbox.
#
# Starts from a Docker-in-Docker base so `run = "docker"` services work, then
# adds everything ecluse and the hooks shell out to.
FROM docker:28-dind

RUN apk add --no-cache \
      bash curl git jq lsof tmux \
      nodejs npm \
      postgresql16-client

ARG ECLUSE_VERSION=v0.3.1
RUN arch="$(uname -m)" \
 && curl -fsSL "https://github.com/hefgi/ecluse/releases/download/${ECLUSE_VERSION}/ecluse-${ECLUSE_VERSION}-${arch}-unknown-linux-musl.tar.gz" \
    | tar xz -C /usr/local/bin ecluse \
 && ecluse --version

COPY bootstrap.sh /usr/local/bin/ecluse-bootstrap
RUN chmod +x /usr/local/bin/ecluse-bootstrap

# Keep the sandbox alive; dockerd is started by ecluse-bootstrap.
ENTRYPOINT []
CMD ["sleep", "infinity"]
