# Changelog

## 0.1.9

This release improves DJI USB video recovery, HDMI output, custom splash screens,
and browser-based maintenance.

### Video input and viewing

- Add an experimental DJI Goggles 3 USB-A input for video only. Show a waiting
  state until video arrives after enabling **Sharing live view**. Keep the
  standard DJI USB-C input, USB cameras, capture devices and analog receivers.
- Rebuild video sessions after USB reconnects and clear stale partial H.264
  data. When a supported stream resumes without an IDR frame, generate a neutral
  startup frame from its live SPS with a separate PPS ID, without re-encoding
  the video.
- Improve Watch playback with a viewer-specific startup keyframe and recovery
  when a browser viewer falls behind. Retain the required parameter sets and
  keep each viewer's startup frame separate from HDMI and other viewers.
- Bound USB UDP parsing and diagnostics, distinguish incoming SPS/PPS/IDR data
  from generated frames, and avoid decoder restarts caused only by repeated
  stale-frame warnings.
- Remove the Boost bitrate option.

### HDMI and splash screen

- Detect HDMI hot-plug events and restore video output after changing cables.
  Send video to either or both HDMI connectors using one decoder, preserving
  scaling, rotation, mirroring and overlays on each selected output.
- Expand the manual HDMI mode list with standard progressive and interlaced
  timings, including 1920×1080 at 60 Hz, SD formats and supported 4K modes, even
  when a screen does not advertise those modes through EDID. Add modes offered
  by the connected display and label the scan types.
- Allow video uploads for the splash screen. Convert supported source formats
  to silent MP4 with H.264 High Profile at 1920×1080, 30 FPS, and about
  4 Mbit/s, without B-frames and with a keyframe about every second. Keep the
  previous splash screen if conversion fails. Source MP4 files must contain an
  H.264 video track; image splash screens remain available.

### Web interface and updates

- Apply monochrome Openstreamer styling and simplify setup, viewing, recording,
  and maintenance from the browser. Use Openstreamer consistently in the
  application, service, packages, settings and data paths, and remove author
  and project-credit links from the device interface.
- Check the latest stable Openstreamer release in the public GitHub repository.
  Install updates from one signed `.osupd` file or a `.deb` with its `.sig`;
  verify signatures, preserve settings and recordings, and roll back if the new
  application fails to start.

### System and builds

- Build application updates separately from the operating-system image. Provide
  portable image-build scripts and reuse completed base and runtime stages
  when rebuilding the application image.
- Keep recording, stream output, network configuration, local access-point
  setup and USB-drive handling available from the web interface.
- Include HDMI hot-plug, simultaneous HDMI outputs, goggles reconnects and
  splash playback in Raspberry Pi hardware validation.
