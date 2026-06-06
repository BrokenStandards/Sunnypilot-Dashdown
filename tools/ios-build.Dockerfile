# iOS build environment for Linux hosts (e.g. Arch/EndeavourOS without a native
# Swift toolchain). Provides Swift 6.3 + Rust (iOS target) + xtool so we can:
#   - build the Darwin Swift SDK from a host-mounted Xcode.xip (`xtool sdk build`)
#   - cross-compile the Rust core for aarch64-apple-ios (build-rust-ios.sh)
#   - build the SwiftUI app (`xtool dev build`)
#
# Build:  docker build -f tools/ios-build.Dockerfile -t dashdown-ios-build .
# Use:    see tools/ios-build.sh (mounts the repo, the Xcode.xip, and caches the
#         Swift SDK + cargo registry across runs).
FROM swift:6.3-jammy

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl build-essential pkg-config git xz-utils zip unzip cmake \
    && rm -rf /var/lib/apt/lists/*

# Rust via rustup; the repo's rust-toolchain.toml pins channel + targets, which
# rustup auto-provisions on first use. Pre-add the iOS device target to warm it.
RUN curl -fsS https://sh.rustup.rs | sh -s -- -y --default-toolchain stable --profile minimal
ENV PATH="/root/.cargo/bin:${PATH}"
RUN rustup target add aarch64-apple-ios

# xtool (AppImage). Docker has no FUSE, so always extract-and-run.
ENV APPIMAGE_EXTRACT_AND_RUN=1
RUN curl -fL "https://github.com/xtool-org/xtool/releases/latest/download/xtool-x86_64.AppImage" \
        -o /usr/local/bin/xtool \
    && chmod +x /usr/local/bin/xtool

WORKDIR /work
CMD ["/bin/bash"]
