# The `brain` binary in a minimal image. Used for the local board
# (deploy/docker-compose.board.yml) and, later, on the VPS.
FROM rust:1-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY tools ./tools
RUN cargo build --release -p brain

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates git \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -m -u 1000 brain
COPY --from=build /src/target/release/brain /usr/local/bin/brain
USER brain
WORKDIR /brain
ENV BRAIN_HOME=/brain
ENTRYPOINT ["brain"]
CMD ["serve", "--bind", "0.0.0.0:8080"]
