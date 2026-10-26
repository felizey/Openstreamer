#!/bin/sh
set -eu
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4}
export CARGO_TARGET_DIR=/build/target
# Use a Linux volume for compilation, keeping Windows bind-mounted paths out
# of Rust build outputs. /source is a read-only snapshot of this fork.
# /build is this builder's dedicated Docker volume. Inputs are refreshed while
# compiler outputs stay in /build/target, so removed branding cannot survive.
[ ! -L /build/device ] || { echo "Unexpected source symlink" >&2; exit 1; }
rm -rf -- /build/device
mkdir -p /build/device/backend /build/device/frontend /build/device/packaging
tar -C /source/device/backend --exclude=./target -cf - . | tar -C /build/device/backend -xf -
cp -a "${OPENSTREAMER_FRONTEND_DIST:-/source/device/frontend/dist}" /build/device/frontend/dist
cp -a /source/device/packaging/. /build/device/packaging/
cp /source/LICENSE /source/CHANGELOG.md /build/
cd /build/device/backend
cargo fmt --all -- --check
PKG_CONFIG_ALLOW_CROSS=1 \
PKG_CONFIG_LIBDIR=/usr/lib/aarch64-linux-gnu/pkgconfig:/usr/share/pkgconfig \
    cargo test --release --locked --target aarch64-unknown-linux-gnu
PKG_CONFIG_ALLOW_CROSS=1 \
PKG_CONFIG_LIBDIR=/usr/lib/aarch64-linux-gnu/pkgconfig:/usr/share/pkgconfig \
    cargo test --release --locked --target aarch64-unknown-linux-gnu \
    output::splash::video::tests:: -- --ignored --test-threads=1
PKG_CONFIG_ALLOW_CROSS=1 \
PKG_CONFIG_LIBDIR=/usr/lib/aarch64-linux-gnu/pkgconfig:/usr/share/pkgconfig \
    cargo test --release --locked --target aarch64-unknown-linux-gnu \
    input::goggles3::tests::generated_startup_idr_decodes_after_parameter_only_access_units -- --ignored --test-threads=1
PKG_CONFIG_ALLOW_CROSS=1 \
PKG_CONFIG_LIBDIR=/usr/lib/aarch64-linux-gnu/pkgconfig:/usr/share/pkgconfig \
    cargo clippy --release --locked --target aarch64-unknown-linux-gnu --all-targets -- -D warnings
PKG_CONFIG_ALLOW_CROSS=1 \
PKG_CONFIG_LIBDIR=/usr/lib/aarch64-linux-gnu/pkgconfig:/usr/share/pkgconfig \
CARGO_TARGET_DIR=/build/target \
    cargo build --release --locked --target aarch64-unknown-linux-gnu
PKG_CONFIG_ALLOW_CROSS=1 \
PKG_CONFIG_LIBDIR=/usr/lib/aarch64-linux-gnu/pkgconfig:/usr/share/pkgconfig \
CARGO_TARGET_DIR=/build/target \
    cargo deb --no-build --no-strip --target aarch64-unknown-linux-gnu --output /out/
cp /build/target/aarch64-unknown-linux-gnu/release/openstreamer /out/openstreamer-rpi4b-arm64
