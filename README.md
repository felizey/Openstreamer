<div align="center">

<img src="device/frontend/public/logo.svg" alt="Openstreamer" width="96" height="96" />

# Openstreamer

Openstreamer is a fork of YStreamer with changes to its functionality.
All functionality is written using AI and tested on Raspberry Pi 4B.

**Your FPV feed, on every screen.**

Raspberry Pi 4 software that takes the live video from your DJI goggles or a USB
camera and sends it to an HDMI screen, any browser, RTSP/SRT/RTMP/UDP streams
and recordings - all at once, set up from a web interface.

[![License: GPL v3](https://img.shields.io/badge/License-GPLv3-2563eb.svg?style=flat-square)](LICENSE)
[![Raspberry Pi 4](https://img.shields.io/badge/Raspberry%20Pi-4%20Model%20B-c51a4a?style=flat-square&logo=raspberrypi&logoColor=white)](https://www.raspberrypi.com/products/raspberry-pi-4-model-b/)
[![Built with Rust](https://img.shields.io/badge/built%20with-Rust-b7410e?style=flat-square&logo=rust&logoColor=white)](https://www.rust-lang.org)
[![GStreamer](https://img.shields.io/badge/video-GStreamer-3d8bff?style=flat-square)](https://gstreamer.freedesktop.org)

[**Portable build from source**](BUILD.md) · [**Documentation**](site/src/content/docs/en/getting-started/index.mdx) · [**Upstream and history**](UPSTREAM.md)

</div>

---

## What it does

Plug your goggles into a Raspberry Pi 4 and everyone else gets to see what you
see. The standard DJI goggles source uses USB-C. A separate experimental
**DJI Goggles 3 - USB-A** source uses a USB-A network connection.
Enable **Sharing live view** in the goggles. Every source feeds the outputs at the same time:

- **HDMI** - the feed on a TV, monitor or projector.
- **Any browser** - live view on phones and laptops, over WebRTC or WebSocket.
- **Streams** - an RTSP server, plus SRT, RTMP (YouTube, Twitch, Facebook) and
  UDP for OBS, VLC, QGroundControl and Mission Planner.
- **Recording** - MP4 or MPEG-TS to the SD card or a USB drive, started by hand,
  automatically when video arrives, or with a physical button.

It also changes the **air unit's camera settings** from the browser, shows the
**goggles' battery and link quality**, offers **remote access** through
Tailscale or WireGuard, and supports **signed manual updates** that roll
back if a new version doesn't start. Check for releases from felizey/Openstreamer
in **System → Updates**, then download and install a signed update file.

DJI Goggles 3 - USB-A currently provides video only. Select it on the **Input** page;
see the [video input guide](site/src/content/docs/en/using/video-input.mdx) for setup.

Everything is set up from the web interface at `http://openstreamer.local`. No
terminal needed.

## Supported inputs

| Source                                        | Port  | Video | Camera settings | Battery & link |
| --------------------------------------------- | :---: | :---: | :-------------: | :------------: |
| DJI Goggles 2, Integra, Goggles 3, Goggles N3 | USB-C |  ✅   |       ✅        |       ✅       |
| DJI Goggles 3                                 | USB-A |  ✅   |       ❌        |       ❌       |
| DJI FPV Goggles V1 / V2 _(experimental)_      | USB-A |  ✅   |       ❌        |       ❌       |
| USB cameras, capture sticks, analog receivers | USB-A |  ✅   |        -        |       -        |

_Which camera settings can be changed depends on the air unit (O3, O4, O4 Pro)._

> [!NOTE]
> Openstreamer is an independent project. It is not affiliated with, endorsed by
> or sponsored by DJI.

## What you need

- **Raspberry Pi 4 Model B** - the target board for this test build.
- **A microSD card**, 16 GB or more.
- **Power through the GPIO header or a PoE HAT** - the USB-C port is reserved
  for the goggles while Openstreamer is installed.
- **A USB-C to USB-C data cable** for the standard goggles mode, or a
  **USB-A to USB-C data cable** for DJI Goggles 3 - USB-A. A USB camera is also supported.

Wi-Fi runs on 2.4 GHz only, on purpose: 5 GHz is where the goggles' video link
lives. See [What you need](site/src/content/docs/en/getting-started/requirements.mdx)
for the details.

## Getting started

**On a blank SD card (recommended):**

1. [Download the image](https://github.com/felizey/Openstreamer/releases)
   from the latest release.
2. Flash it with Raspberry Pi Imager ("Use custom") or balenaEtcher.
3. Boot the Pi, join the `Openstreamer` Wi-Fi network (password `openstreamer`) and
   open `http://10.0.0.1`. An HDMI screen shows the details and a QR code too.

**On Raspberry Pi OS Lite (64-bit, Trixie)** that's already set up:

```sh
sudo apt install ./openstreamer_*_arm64.deb
```

Signed application updates can later be installed from **System → Updates** with
one `.osupd` file; settings and recordings are kept.

See the [full documentation](site/src/content/docs/en/getting-started/index.mdx) for the first
boot, every output, guides for OBS, VLC and ground stations, and common issues.

## Building from source

For a portable build that uses public toolchain images and does not depend on a
developer's local Docker cache, use the commands in [BUILD.md](BUILD.md):

```bash
bash build.sh package   # build the arm64 Debian package
bash build.sh image     # build the Raspberry Pi 4B SD card image
bash build.sh update    # build a signed browser update
```

The last command needs the private update-signing key, which is intentionally
kept outside this public repository. The matching public key is included in the
source.

For interactive development, tasks also run through [mise](https://mise.jdx.dev), which installs
the toolchains (Rust, Node, pnpm):

Tasks run through [mise](https://mise.jdx.dev), which also installs the
toolchains (Rust, Node, pnpm):

```sh
# The web interface and the backend, for development
mise run frontend:dev
mise run backend:run        # as root: the hardware and port 80 need it

# Checks
mise run lint
mise run backend:test

# What ships
mise run device:package     # build and sign the .deb
mise run image:full         # .deb plus the SD card image (needs Docker)
```

On Debian or Raspberry Pi OS, `mise run backend:deps` installs the GStreamer
headers the backend builds against and the plugins it runs with. `mise tasks` lists everything else.

Planning to contribute? See [CONTRIBUTING.md](CONTRIBUTING.md) for the dev setup
and how to submit changes.

## Repository layout

| Path                | What's inside                                                       |
| ------------------- | ------------------------------------------------------------------- |
| `device/backend/`   | The Openstreamer service (Rust, GStreamer): video, outputs, web server |
| `device/frontend/`  | The web interface (Svelte), compiled into the backend               |
| `device/packaging/` | The Debian package, install script and update signing               |
| `image/`            | The ready-made SD card image (pi-gen)                               |
| `site/`             | The website and documentation (Astro)                               |

## Acknowledgements

**Inspiration**

- [**SquirrelCast**](https://xnuclearsquirrel.github.io/SquirrelCast-Public): a paid Android app that does something similar, and the push to start this project.

**DJI protocol documentation**

- [**samuelsadok/dji_protocol**](https://github.com/samuelsadok/dji_protocol): documentation of the DJI DUML protocol.
- [**wllm-rbnt/third-eye**](https://github.com/wllm-rbnt/third-eye): detailed documentation of the Goggles 3 USB link.

**Built with**

- [**GStreamer**](https://gstreamer.freedesktop.org) and its [Rust bindings](https://gitlab.freedesktop.org/gstreamer/gstreamer-rs): all of the video handling.
- [**webrtc-rs**](https://github.com/webrtc-rs/webrtc): the low-latency browser view.
- [**Tokio**](https://tokio.rs) and [**Axum**](https://github.com/tokio-rs/axum): the web server.
- [**Svelte**](https://svelte.dev), [**Tailwind CSS**](https://tailwindcss.com) and [**Lucide**](https://lucide.dev): the web interface.
- [**xterm.js**](https://xtermjs.org): the in-browser terminal.
- [**pi-gen**](https://github.com/RPi-Distro/pi-gen): building the SD card image on Raspberry Pi OS.

## License

Openstreamer is free software, licensed under the **GNU General Public License
v3.0**. See [`LICENSE`](LICENSE) for the full text.

Copyright © 2026 YarosFPV
