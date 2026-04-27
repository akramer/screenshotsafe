# Build stage
FROM rust:1.95-slim-bookworm AS builder

WORKDIR /app

# Install native dependencies that might be needed by some crates (like rusqlite bundled or image processing)
RUN apt-get update && apt-get install -y pkg-config libssl-dev cmake g++ && rm -rf /var/lib/apt/lists/*

# Copy the entire project
COPY . .

# Build the application in release mode
RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim

WORKDIR /app

# Install common runtime dependencies and troubleshooting utilities
RUN apt-get update && apt-get install -y \
    ca-certificates \
    sqlite3 \
    curl \
    procps \
    && rm -rf /var/lib/apt/lists/*

# Copy the compiled binary from the builder stage
COPY --from=builder /app/target/release/screenshotsafe /usr/local/bin/screenshotsafe

# Copy static assets into the final image
COPY --from=builder /app/static ./static

# Expose a default port (you can override this via environment variables when running)
EXPOSE 8080

ENTRYPOINT ["/usr/local/bin/screenshotsafe"]
