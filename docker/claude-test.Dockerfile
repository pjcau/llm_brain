# Claude Code in a container, for local tests through OpenRouter without
# touching the host's claude/subscription. Used by `brain setup claude-code --docker`.
# Build: docker build -f docker/claude-test.Dockerfile -t llm-brain-claude-test .
FROM node:20-slim
RUN apt-get update && apt-get install -y --no-install-recommends git ca-certificates python3 \
    && rm -rf /var/lib/apt/lists/* \
    && npm install -g @anthropic-ai/claude-code \
    && git config --system init.defaultBranch main \
    && git config --system --add safe.directory '*'
WORKDIR /work
CMD ["sleep", "infinity"]
