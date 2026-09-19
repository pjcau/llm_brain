# Test image: real aider, used by the testcontainers test in crates/brain/tests.
# Build: docker build -f docker/aider-test.Dockerfile -t llm-brain-aider-test .
FROM python:3.12-slim
RUN apt-get update && apt-get install -y --no-install-recommends git ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && pip install --no-cache-dir aider-chat \
    && git config --global user.email test@llm-brain.local \
    && git config --global user.name llm-brain-test \
    && git config --global init.defaultBranch main
WORKDIR /work
CMD ["sleep", "infinity"]
