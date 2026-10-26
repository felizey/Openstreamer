# DJI Goggles 3 USB-A diagnostics

Openstreamer 0.1.9 restores the decoder after USB reconnects. For supported
H.264 streams that resume without an IDR frame, it generates a neutral startup
frame from the live SPS and uses a separate PPS. Watch viewers receive their
own startup frame and retain both PPS entries in the browser MP4 stream.

## Capture a reconnect problem

Run these tools on an Ubuntu/WSL computer that can reach the Pi. They use the
existing RTSP output and require no new microSD image. Install FFmpeg if needed:

```bash
sudo apt install ffmpeg
```

If video fails to return after reconnecting the goggles' USB cable, leave
**Sharing live view** enabled and capture the stream:

```bash
bash device/diagnostics/capture-rtsp.sh \
  rtsp://10.0.0.1:554/stream reconnect
```

Replace the address if the Pi uses Ethernet or another address. RTSP must be
enabled. Then turn Sharing live view off and on. If the picture returns, run
the same command with `working` instead of `reconnect` to collect a comparison.
A unique directory is created for each run. Existing captures are preserved.

The script copies up to five seconds of H.264 without re-encoding, with a
32 MiB output limit and a 20-second timeout (plus three seconds to terminate).
`-copyinkf` includes initial non-key frames; omitting it can hide the problem.
Include both `stream.h264` files with their `summary.json` and `ffmpeg.log` in
the bug report, keeping the two directories separate. If there are no captured
bytes, include the log. These captures contain the application's RTSP output
after its H.264 parser, rather than raw USB UDP packets or V4L2 driver flags.

## Inspect H.264 data

`inspect-h264.py` reads a completed regular Annex B file of up to 64 MiB and
reports JSON counts for NAL types, the first two slice-header fields, and
recovery-point SEI. It does not change the input file or decode video. The
absence of NAL type 5 alone does not establish intra refresh or prove that a
stream can start without previous reference pictures.

```bash
python3 device/diagnostics/inspect-h264.py capture.h264
python3 -m unittest discover -s device/diagnostics -p 'test_*.py' -v
```

## Decoder startup

The generated startup IDR uses a separate CAVLC PPS and contains no recorded
camera image. For the supported DJI 1920x1080 stream it is about 8 KiB. It
initializes decoder reference state while the incoming stream refreshes the
picture. The technique depends on the stream format; software decoding checks
do not replace testing the V4L2 decoder on a Raspberry Pi.

## References

- [Raspberry Pi driver discussion, including MMAL error concealment](https://github.com/raspberrypi/linux/issues/5166)
- [Raspberry Pi 6.18 decoder driver](https://github.com/raspberrypi/linux/blob/rpi-6.18.y/drivers/staging/vc04_services/bcm2835-codec/bcm2835-v4l2-codec.c)
- [GStreamer V4L2 buffer handling](https://github.com/GStreamer/gstreamer/blob/1.26.2/subprojects/gst-plugins-good/sys/v4l2/gstv4l2bufferpool.c)
- [FFmpeg: copy initial non-key frames](https://ffmpeg.org/ffmpeg.html#Advanced-Video-options)
