FROM node:24-trixie-slim AS frontend
RUN npm install --global pnpm@12.4.2
WORKDIR /web
COPY device/frontend/package.json device/frontend/pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile
COPY device/frontend/ ./
RUN pnpm check && pnpm test && pnpm build

FROM rust:1.95.0-trixie
RUN dpkg --add-architecture arm64 && apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
       crossbuild-essential-arm64 pkg-config dpkg-dev qemu-user ca-certificates \
       libglib2.0-dev:arm64 libgstreamer1.0-dev:arm64 \
       libgstreamer-plugins-base1.0-dev:arm64 libgstrtspserver-1.0-dev:arm64 \
       gstreamer1.0-tools:arm64 gstreamer1.0-plugins-base:arm64 \
       gstreamer1.0-plugins-good:arm64 gstreamer1.0-plugins-bad:arm64 \
       gstreamer1.0-plugins-ugly:arm64 gstreamer1.0-libav:arm64 ffmpeg:amd64 \
    && rm -rf /var/lib/apt/lists/*
RUN rustup target add aarch64-unknown-linux-gnu \
    && rustup component add rustfmt clippy rust-src
RUN PKG_CONFIG_LIBDIR=/usr/lib/x86_64-linux-gnu/pkgconfig:/usr/share/pkgconfig \
    cargo install cargo-deb --version 3.8.0 --locked
# Test host binfmt support before compiling the application. Child ARM64 test
# processes need it too, so using only Cargo's QEMU runner is insufficient.
RUN printf 'int main(void) { return 0; }\n' >/tmp/arm64-probe.c \
    && aarch64-linux-gnu-gcc -static /tmp/arm64-probe.c -o /usr/local/bin/openstreamer-arm64-probe \
    && rm /tmp/arm64-probe.c
COPY --from=frontend /web/dist /opt/openstreamer-web
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc
ENV OPENSTREAMER_FRONTEND_DIST=/opt/openstreamer-web
WORKDIR /build/device/backend
