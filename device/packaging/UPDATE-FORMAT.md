# Openstreamer application updates

The `.osupd` file contains, in order:

1. The ASCII bytes `OPENSTREAMER-UPDATE/1` followed by LF.
2. A raw 64-byte Ed25519 signature of the complete Debian package.
3. The unmodified ARM64 `.deb` package.

There is no archive extraction, file path or install command in this envelope.
The device verifies the signature using its compiled Openstreamer public key,
then checks the package name and device architecture before staging the `.deb`.
The normal installation, health check and previous-package rollback apply.
The web upload limit is 64 MiB, including the envelope.

`make-update-bundle.py` verifies the `.deb.sig` with `update-key.pub` before
creating `.osupd`. The separate `.deb` and base64 `.sig` remain supported for
the first transition from the earlier 0.1.9 test interface. Signing keys are
kept outside distributed source archives and OS images.

Build with `OPENSTREAMER_SIGNING_KEY=/path/to/key.pem bash build.sh update`.
The result is an application update; the kernel and operating system are
not replaced. Use a new application version for each subsequent release.
