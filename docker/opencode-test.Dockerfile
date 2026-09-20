# OpenCode (open-source agent with tools) in a container, for the benchmark
# and for local trials through the proxy.
# Build: docker build -f docker/opencode-test.Dockerfile -t llm-brain-opencode-test .
FROM node:20-slim
RUN apt-get update && apt-get install -y --no-install-recommends git ca-certificates python3 curl \
    && rm -rf /var/lib/apt/lists/* \
    && npm install -g opencode-ai \
    && git config --system init.defaultBranch main \
    && git config --system --add safe.directory '*'
WORKDIR /work
CMD ["sleep", "infinity"]
