# --- Chef Stage ---
FROM rust:1.95-slim-bookworm AS chef
WORKDIR /app
# Install cargo-chef
RUN cargo install cargo-chef --locked

# --- Recipe Stage ---
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# --- Build Stage ---
FROM chef AS builder
WORKDIR /app

# Install native dependencies that might be needed by some crates
RUN apt-get update && apt-get install -y pkg-config libssl-dev cmake g++ && rm -rf /var/lib/apt/lists/*

# Build dependencies - this layer is cached as long as your dependencies don't change!
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

# Build the actual application
COPY . .
RUN cargo build --release

# --- Runtime Stage ---
FROM debian:bookworm-slim

WORKDIR /app

# Install common runtime dependencies and troubleshooting utilities
RUN apt-get update && apt-get install -y \
    ca-certificates \
    sqlite3 \
    curl \
    procps \
    fonts-liberation \
    && rm -rf /var/lib/apt/lists/*

# Copy the compiled binary from the builder stage
COPY --from=builder /app/target/release/screenshotsafe /usr/local/bin/screenshotsafe

# Copy static assets into the final image
COPY --from=builder /app/static ./static

# Expose a default port (you can override this via environment variables when running)
EXPOSE 8080

ENTRYPOINT ["/usr/local/bin/screenshotsafe"]
