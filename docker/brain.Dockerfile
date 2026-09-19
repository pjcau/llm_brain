# Production image for the `brain` binary (used on the VPS only if a container
# is ever preferred over the systemd unit in deploy/).
FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release -p brain

FROM gcr.io/distroless/cc-debian12
COPY --from=build /src/target/release/brain /brain
COPY config /config
ENTRYPOINT ["/brain"]
