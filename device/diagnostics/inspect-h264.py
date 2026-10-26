#!/usr/bin/env python3
"""Read-only, bounded inspection of selected headers in an Annex B H.264 file.

This is not a decoder or a complete bitstream validator. In particular, a
non-IDR slice is not necessarily a P-slice, and IDR absence does not establish
gradual decoder refresh. Counts describe NAL units and slices, never FPS.
"""

import argparse
import json
import os
from pathlib import Path
import re
import stat
import sys


MAX_BYTES = 64 * 1024 * 1024
MAX_RECOVERY_PREVIEW = 16
START_CODE = re.compile(b"\x00\x00(?:\x00)?\x01")
SLICE_NAMES = ("P", "B", "I", "SP", "SI")


class InspectionError(ValueError):
    """The source cannot be inspected as an Annex B stream."""


class HeaderError(ValueError):
    def __init__(self, kind, detail):
        super().__init__(detail)
        self.kind = kind


class Bits:
    def __init__(self, data):
        self.data = data
        self.position = 0

    def read(self, count):
        if self.position + count > len(self.data) * 8:
            raise HeaderError("truncated", "not enough bits in header")
        value = 0
        for _ in range(count):
            byte, bit = divmod(self.position, 8)
            value = (value << 1) | ((self.data[byte] >> (7 - bit)) & 1)
            self.position += 1
        return value

    def ue(self):
        leading_zeroes = 0
        while self.read(1) == 0:
            leading_zeroes += 1
            if leading_zeroes > 32:
                raise HeaderError("malformed", "Exp-Golomb value exceeds 32 bits")
        value = (1 << leading_zeroes) - 1 + self.read(leading_zeroes)
        if value > 0xFFFFFFFF:
            raise HeaderError("malformed", "Exp-Golomb value exceeds 32 bits")
        return value


def unescape_rbsp(ebsp):
    """Remove legal emulation-prevention bytes; reject illegal 00 00 03 xx."""
    result = bytearray()
    zeroes = 0
    for index, byte in enumerate(ebsp):
        if zeroes >= 2 and byte == 3:
            if index + 1 == len(ebsp):
                raise HeaderError("truncated", "emulation-prevention byte has no following byte")
            if ebsp[index + 1] > 3:
                raise HeaderError("malformed", "invalid emulation-prevention sequence")
            zeroes = 0
            continue
        result.append(byte)
        zeroes = zeroes + 1 if byte == 0 else 0
    return result


def slice_header(ebsp):
    bits = Bits(unescape_rbsp(ebsp))
    first_mb = bits.ue()
    slice_type = bits.ue()
    if slice_type > 9:
        raise HeaderError("malformed", "slice_type is outside 0..9")
    return first_mb, slice_type


def sei_messages(ebsp):
    """Yield complete payloads and require the SEI RBSP trailing-bits byte."""
    rbsp = unescape_rbsp(ebsp)
    offset = 0
    while offset < len(rbsp):
        if len(rbsp) - offset == 1 and rbsp[offset] == 0x80:
            return
        payload_type = 0
        while True:
            if offset == len(rbsp):
                raise HeaderError("truncated", "incomplete SEI payload type")
            byte = rbsp[offset]
            offset += 1
            payload_type += byte
            if byte != 255:
                break
        payload_size = 0
        while True:
            if offset == len(rbsp):
                raise HeaderError("truncated", "incomplete SEI payload size")
            byte = rbsp[offset]
            offset += 1
            payload_size += byte
            if byte != 255:
                break
        if payload_size > len(rbsp) - offset:
            raise HeaderError("truncated", "SEI payload exceeds remaining RBSP bytes")
        yield payload_type, rbsp[offset:offset + payload_size]
        offset += payload_size
    raise HeaderError("truncated", "missing SEI RBSP trailing bits")


def recovery_point(payload):
    bits = Bits(payload)
    return {
        "recovery_frame_cnt": bits.ue(),
        "exact_match_flag": bool(bits.read(1)),
        "broken_link_flag": bool(bits.read(1)),
        "changing_slice_group_idc": bits.read(2),
    }


def nal_units(data):
    previous = None
    for match in START_CODE.finditer(data):
        if previous is not None:
            yield previous, data[previous:match.start()].rstrip(b"\x00")
        previous = match.end()
    if previous is None:
        raise InspectionError("no Annex B start code found; MP4/AVCC input is not supported")
    yield previous, data[previous:].rstrip(b"\x00")


def inspect_annex_b(data):
    if len(data) > MAX_BYTES:
        raise InspectionError("input exceeds the 64 MiB limit")
    first = START_CODE.search(data)
    if first is None:
        raise InspectionError("no Annex B start code found; MP4/AVCC input is not supported")
    result = {
        "format": "H.264 Annex B",
        "bytes_read": len(data),
        "bytes_before_first_start_code": first.start(),
        "count_units": "NAL units and slices; these counts are not decoded frames or FPS",
        "nal_units_total": 0,
        "nal_types": {},
        "empty_nal_units": 0,
        "invalid_nal_headers": 0,
        "slices": {
            "nal_units_examined": 0,
            "headers_parsed": 0,
            "slice_type_modulo_5": dict.fromkeys(SLICE_NAMES, 0),
            "slice_type_raw": dict.fromkeys(map(str, range(10)), 0),
            "first_mb_in_slice_zero": 0,
            "first_mb_in_slice_nonzero": 0,
            "first_mb_in_slice_max": None,
            "malformed_headers": 0,
            "truncated_headers": 0,
        },
        "sei": {
            "messages_parsed": 0,
            "recovery_point_messages": 0,
            "recovery_points_parsed": 0,
            "malformed_nal_units": 0,
            "truncated_nal_units": 0,
            "malformed_recovery_points": 0,
            "truncated_recovery_points": 0,
            "recovery_frame_cnt_min": None,
            "recovery_frame_cnt_max": None,
            "recovery_points_preview": [],
            "preview_limit": MAX_RECOVERY_PREVIEW,
        },
        "limitations": [
            "Only selected headers are inspected; SPS/PPS, pictures and reference dependencies are not validated.",
            "NAL type 5 is IDR; non-IDR NALs may contain I slices. IDR absence alone does not establish gradual decoder refresh.",
            "A parsed slice header does not prove that its remaining payload, including the final NAL, is complete.",
            "Malformed or truncated data may result from an incomplete capture as well as an invalid stream.",
        ],
    }
    slices = result["slices"]
    sei = result["sei"]
    for offset, nal in nal_units(data):
        result["nal_units_total"] += 1
        if not nal:
            result["empty_nal_units"] += 1
            continue
        nal_type = nal[0] & 31
        key = str(nal_type)
        result["nal_types"][key] = result["nal_types"].get(key, 0) + 1
        if nal[0] & 128 or nal_type == 0 or nal_type >= 24:
            result["invalid_nal_headers"] += 1
            continue
        if nal_type in (1, 2, 5):
            slices["nal_units_examined"] += 1
            try:
                first_mb, slice_type = slice_header(nal[1:])
            except HeaderError as error:
                slices[error.kind + "_headers"] += 1
                continue
            slices["headers_parsed"] += 1
            slices["slice_type_raw"][str(slice_type)] += 1
            slices["slice_type_modulo_5"][SLICE_NAMES[slice_type % 5]] += 1
            bucket = "first_mb_in_slice_zero" if first_mb == 0 else "first_mb_in_slice_nonzero"
            slices[bucket] += 1
            current_max = slices["first_mb_in_slice_max"]
            slices["first_mb_in_slice_max"] = first_mb if current_max is None else max(current_max, first_mb)
        elif nal_type == 6:
            try:
                for payload_type, payload in sei_messages(nal[1:]):
                    sei["messages_parsed"] += 1
                    if payload_type != 6:
                        continue
                    sei["recovery_point_messages"] += 1
                    try:
                        point = recovery_point(payload)
                    except HeaderError as error:
                        sei[error.kind + "_recovery_points"] += 1
                        continue
                    sei["recovery_points_parsed"] += 1
                    count = point["recovery_frame_cnt"]
                    for suffix, operation in (("min", min), ("max", max)):
                        key = "recovery_frame_cnt_" + suffix
                        sei[key] = count if sei[key] is None else operation(sei[key], count)
                    if len(sei["recovery_points_preview"]) < MAX_RECOVERY_PREVIEW:
                        sei["recovery_points_preview"].append({
                            "nal_index": result["nal_units_total"],
                            "nal_header_offset": offset,
                            **point,
                        })
            except HeaderError as error:
                sei[error.kind + "_nal_units"] += 1
    result["nal_types"] = dict(sorted(result["nal_types"].items(), key=lambda item: int(item[0])))
    return result


def inspect_file(path):
    try:
        with Path(path).open("rb") as source:
            before = os.fstat(source.fileno())
            if not stat.S_ISREG(before.st_mode):
                raise InspectionError("input must be a regular file")
            if before.st_size > MAX_BYTES:
                raise InspectionError("input exceeds the 64 MiB limit")
            data = source.read(MAX_BYTES)
            after = os.fstat(source.fileno())
            if (len(data) != before.st_size or after.st_size != before.st_size
                    or after.st_mtime_ns != before.st_mtime_ns):
                raise InspectionError("input changed while being read; retry with a completed capture")
    except OSError as error:
        raise InspectionError(f"cannot read input: {error}") from error
    return inspect_annex_b(data)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("file", help="completed Annex B .h264 capture, maximum 64 MiB")
    args = parser.parse_args()
    try:
        summary = inspect_file(args.file)
    except InspectionError as error:
        print(f"inspect-h264: {error}", file=sys.stderr)
        return 1
    print(json.dumps(summary, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
