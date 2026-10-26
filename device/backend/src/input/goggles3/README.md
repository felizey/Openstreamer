# DJI Goggles 3 - USB-A

This is an independent Rust implementation of the observed UDP video transport.
It uses the existing Openstreamer H.264 outputs. USB means USB Ethernet with the
Pi as host. Enable **Sharing live view** in the goggles. The standard USB-C
accessory/DUML transport remains a separate source, `dji_fpv`.

## Browser startup

Watch creates a separate neutral IDR for each new viewer of a supported stream
without IDR. It does not re-prime the shared HDMI stream. The WebSocket MP4
configuration retains both the original live PPS and the generated PPS. WebRTC
waits for connection before starting its private primer. A WebSocket viewer
whose playback queue overflows can request another private keyframe.

## Protocol behavior

This is an independent Rust implementation of the observed USB network
transport. It parses the 20-byte UDP header and forwards the Annex B H.264
payload. Goggles 3 discovery, acknowledgements and video use separate message
channels; USB video is received from `192.168.60.2:9003`.

Relevant evidence functions:

| Function | Observed behavior |
| --- | --- |
| `FUN_000229fc` | 20-byte receive header; channel byte 6, echo word at 2 and receive ID at 10, little endian; channels 1 and 2 |
| `FUN_00022ccc` | 48-byte discovery/start datagram |
| `FUN_00022dac` | 34-byte ACK with echoed word and receive ID |
| `FUN_00023220` | Periodic discovery until control data arrives, ACK maintenance afterwards |
| `FUN_00021294` | Video is a continuous Annex B stream, NAL boundaries can cross UDP packets |
| `FUN_0002a570` | Wi-Fi peer 192.168.2.1 and USB peer 192.168.60.2 |
| `FUN_0002a760` | Local USB Ethernet address 192.168.60.1/24 |

Openstreamer does not include the proprietary receiver, activation code,
licence checks or prerecorded video arrays. Camera commands and telemetry are
not implemented for this source. This input supports only USB; the historical
Wi-Fi peer in the evidence table is not an available Openstreamer connection mode.

Tests exercise the datagrams, stream boundaries, startup without an IDR gate,
bounded buffering, interface identity and a local simulated UDP peer. Actual
Goggles 3 compatibility and UDP loss/reordering behavior require hardware tests.
The receive ID is echoed rather than assumed to increment by one: that meaning
is not established by the supplied evidence.

Each new UDP socket starts a new live-stream generation. HDMI discards the
previous partial access unit and replaces its parser/decoder on a generation or
live/splash boundary; warnings alone do not trigger that recovery. Actual bus or
flow errors are recovered at fresh SPS headers. Tests cover the byte boundaries
and GStreamer lifecycle through appsrc/appsink; they do not emulate the Pi's
V4L2 firmware or DRM display.

The journal records each session's UDP port, the first real SPS/PPS/IDR NAL and
one input summary. If video slices arrive without an IDR for five seconds, one
diagnostic notice is logged. Slice/NAL counts are not displayed frame counts.

When a supported source has supplied no IDR, Openstreamer generates a small
neutral IDR from the live SPS and a separate unused PPS ID before the first P
picture. The CAVLC picture uses I16x16 DC prediction with zero residuals
(about 8 KiB for the captured 1080p format). Live pictures and their original
PPS are unchanged. No captured picture is embedded in the app.

This currently supports High profile, 8-bit 4:2:0, progressive pictures, POC type
2 and one reference picture within the Pi 4's dimensions and 8192-MB limit.
Unsupported parameters are logged and passed through without injection. A
source IDR suppresses injection. Session/gap resets discard all bootstrap state;
source diagnostics exclude generated data. This addresses stream startup, not
requests from viewers joining an already-running IDR-less stream later.

Software replay of the user's capture produced 0 frames without priming and
268 with it (one generated picture plus 267 live pictures). After picture 19,
decoded pixels matched the reference replay using the real source IDR. This
does not establish V4L2/firmware compatibility; test reconnects on the Pi.
