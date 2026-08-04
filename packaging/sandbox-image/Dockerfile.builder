# Dedicated CLI used by the one-shot sandbox-image builder. Keep Buildx in the
# controlled builder image rather than relying on the host Docker installation.
FROM docker:27-cli

RUN apk add --no-cache bash git docker-cli-buildx
