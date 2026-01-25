# Build stage with musl target for static linking
FROM rust:alpine AS builder

# Install build dependencies
RUN apk add --no-cache musl-dev

WORKDIR /app

# Copy manifests
COPY Cargo.toml Cargo.lock ./
COPY benches ./benches

# Build dependencies first (for better caching)
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release
RUN rm -rf src

# Copy source code
COPY src ./src

# Build the application
ARG GIT_REVISION
ENV GIT_REVISION=${GIT_REVISION}
RUN cargo build --release

# Minimal
FROM scratch

# Copy the statically linked binary
COPY --from=builder /app/target/release/iotd /usr/bin/iotd

# Expose MQTT port
EXPOSE 1883

# Use ENTRYPOINT for better signal handling
ENTRYPOINT ["/usr/bin/iotd"]
