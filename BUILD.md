# Openstreamer 0.1.9

## Requirements

- Ubuntu/WSL2 x86_64 and a working Docker installation with Linux containers.
- Bash, Git, curl, dpkg, coreutils, util-linux, xz-utils, Python 3, and OpenSSL:
  `sudo apt-get update && sudo apt-get install -y git curl dpkg coreutils util-linux xz-utils python3 openssl`.
- Internet access for public Node 24 / Rust 1.95 / Debian Trixie images, packages,
  Cargo, pnpm, and pi-gen sources. Access to a private registry is not required.
- ARM64 execution support through binfmt/QEMU on the Docker host. Docker Desktop
  usually provides this. On a standalone Linux host, enable it if needed:

```bash
docker run --privileged --rm tonistiigi/binfmt --install arm64
```

This command registers emulation in the Docker host's kernel; run it on a
machine where this configuration is permitted. The script checks ARM64 support
before compiling the application and explains the error if support is unavailable.

## Application package

Extract the ZIP and navigate to the `Openstreamer` directory:

```bash
bash build.sh package
```

Docker builds the environment from the included Dockerfile, checks and rebuilds
the frontend using pnpm-lock.yaml, then runs Rust checks, integration tests,
Clippy, and the ARM64 build using Cargo.lock.
The required GStreamer plugins and FFmpeg are listed in the Dockerfile. Node,
Rust, FFmpeg, and previous Openstreamer images are not required on the host.

Output: `device/backend/target/debian/openstreamer_0.1.9-1_arm64.deb`
and a SHA-256 checksum file. Cargo and compiler caches are created automatically
for this directory; an empty cache only makes the first build take longer.

## microSD image

```bash
bash build.sh image
```

The command first builds the package, then the pi-gen tools from
`image/Dockerfile.test`, the base OS, dependencies, and the Raspberry Pi 4B image.
The completed `openstreamer-0.1.9-raspi4b-arm64.img.xz` and SHA-256 checksum file appear in `image/deploy/`.
The image includes the current interface, HDMI modes, and update verification key.
It also keeps the current version's package for rollback if an update fails.
The image builder uses a privileged container for loop devices and mounting the
image filesystem; physical host disks are not passed through to the container.
The image is not written to a microSD card automatically.

To rebuild on the same machine with a previously prepared base:

```bash
bash build.sh image-reuse
```

On a different machine, start with `image` instead of `image-reuse`. Do not specify
the SHA of a local Docker image from another computer. The script creates its own
layers and caches on the current machine. The first full image takes a long time
to build; caching speeds up subsequent builds, but a changed image still needs
to be exported and compressed.

## Archive contents

The archive includes the backend, frontend with its original compiled dist,
documentation, Dockerfiles, packaging scripts, and pi-gen. It excludes `.deb`
packages, microSD images, Docker volumes, registry/cache data, node_modules,
private captures, and logs.
The compiled frontend in the ZIP comes from the original build; `build.sh`
always rebuilds the interface from source. The application version is 0.1.9.

## Updates through the web interface

On the new image: System > Updates > Choose update > select one `.osupd` > Install.
The file contains only the application and signature, without the OS; recordings
and settings are preserved. If the new version fails to start, the installer
attempts to restore the previous package. There is no need to remove or rewrite
the microSD card to update the application.

To sign your own builds, you need the private key corresponding to
`device/packaging/update-key.pub` and OpenSSL on the build machine.

If the key is saved as `~/openstreamer-signing-key.pem`, run:

```bash
bash build.sh update
```

To use another path, specify it explicitly:

```bash
OPENSTREAMER_SIGNING_KEY=/path/to/private-key.pem bash build.sh update
```

Output in `device/backend/target/debian/`: **Openstreamer-0.1.9.osupd**. This command does not build an OS image.
For subsequent releases, increase the application version and use the same key.

The script checks that the keys match and reports an error if they do not.
Unsigned builds can still be installed through apt.
Keep the private key outside the public source archive and make a backup.
