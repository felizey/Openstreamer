#!/usr/bin/env bash
# Capture the existing RTSP output on a Linux PC; do not decode or re-encode it.
set -euo pipefail
HERE=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
URL=${1:-rtsp://10.0.0.1:554/stream}
LABEL=${2:-reconnect}
if [ "$#" -gt 2 ] || [[ ! "$URL" =~ ^rtsps?:// ]] || [[ ! "$LABEL" =~ ^[a-zA-Z0-9_-]{1,32}$ ]]; then
    echo "Usage: bash $0 [rtsp://PI_ADDRESS:554/stream] [reconnect|working]" >&2
    exit 2
fi
for command in ffmpeg timeout mktemp; do
    command -v "$command" >/dev/null || {
        echo "Missing $command. On Ubuntu install ffmpeg and coreutils." >&2
        exit 1
    }
done
OUTPUT=$(mktemp -d "./openstreamer-${LABEL}-$(date +%Y%m%d-%H%M%S).XXXXXX")
OUTPUT=$(cd -- "$OUTPUT" && pwd)
echo "Capturing 5 seconds to $OUTPUT"
STATUS=0
timeout --signal=INT --kill-after=3s 20s \
    ffmpeg -hide_banner -nostdin -loglevel warning -n \
    -rtsp_transport tcp -i "$URL" -map 0:v:0 -an \
    -c:v copy -copyinkf -t 5 -fs 33554432 -f h264 "$OUTPUT/stream.h264" \
    > "$OUTPUT/ffmpeg.log" 2>&1 || STATUS=$?
printf '%s\n' "$STATUS" > "$OUTPUT/ffmpeg-exit-status.txt"
if [ ! -s "$OUTPUT/stream.h264" ]; then
    echo "No video bytes captured. Keep $OUTPUT/ffmpeg.log for diagnosis." >&2
    tail -n 20 "$OUTPUT/ffmpeg.log" >&2
    exit 1
fi
if [ "$STATUS" != 0 ]; then
    echo "FFmpeg exited with status $STATUS; the captured stream may be incomplete." >&2
fi
if command -v python3 >/dev/null; then
    if python3 "$HERE/inspect-h264.py" "$OUTPUT/stream.h264" \
        > "$OUTPUT/summary.json" 2> "$OUTPUT/inspect.log"; then
        echo "Header analysis: $OUTPUT/summary.json"
    else
        echo "Header analysis failed; keep the raw stream and inspect.log." >&2
    fi
fi
echo "Capture: $OUTPUT/stream.h264"
echo "Log: $OUTPUT/ffmpeg.log"
