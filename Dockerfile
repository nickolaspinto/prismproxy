FROM rust:1.77-slim AS builder
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/prismproxy /usr/local/bin/prismproxy
COPY config/default.toml /etc/prismproxy/config.toml
EXPOSE 8080 443 80
ENTRYPOINT ["prismproxy"]
CMD ["/etc/prismproxy/config.toml"]
