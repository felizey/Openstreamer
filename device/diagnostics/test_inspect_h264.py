import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("inspect-h264.py")
SPEC = importlib.util.spec_from_file_location("inspect_h264", SCRIPT)
INSPECTOR = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(INSPECTOR)
PREFIX = b"\x00\x00\x00\x01"


def ue(value):
    binary = f"{value + 1:b}"
    return "0" * (len(binary) - 1) + binary


def packed(bits):
    bits += "1"
    bits += "0" * (-len(bits) % 8)
    return int(bits, 2).to_bytes(len(bits) // 8, "big")


def escaped(rbsp):
    result = bytearray()
    zeroes = 0
    for byte in rbsp:
        if zeroes >= 2 and byte <= 3:
            result.append(3)
            zeroes = 0
        result.append(byte)
        zeroes = zeroes + 1 if byte == 0 else 0
    return bytes(result)


def slice_nal(first_mb, slice_type, nal_type=1):
    return PREFIX + bytes([0x60 | nal_type]) + escaped(packed(ue(first_mb) + ue(slice_type)))


def sei_nal(messages, trailing=b"\x80"):
    rbsp = bytearray()
    for kind, payload in messages:
        rbsp.extend(b"\xff" * (kind // 255))
        rbsp.append(kind % 255)
        rbsp.extend(b"\xff" * (len(payload) // 255))
        rbsp.append(len(payload) % 255)
        rbsp.extend(payload)
    rbsp.extend(trailing)
    return PREFIX + b"\x06" + escaped(rbsp)


class InspectorTests(unittest.TestCase):
    def test_non_idr_intra_and_all_slice_type_aliases_are_counted_as_slices(self):
        stream = b"".join(slice_nal(index, kind, (1, 2, 5)[index % 3])
                          for index, kind in enumerate(range(10)))
        result = INSPECTOR.inspect_annex_b(stream)
        self.assertEqual(result["nal_units_total"], 10)
        self.assertEqual(result["slices"]["headers_parsed"], 10)
        self.assertEqual(result["slices"]["slice_type_modulo_5"], dict.fromkeys(INSPECTOR.SLICE_NAMES, 2))
        self.assertEqual(result["slices"]["first_mb_in_slice_zero"], 1)
        self.assertEqual(result["slices"]["first_mb_in_slice_nonzero"], 9)
        self.assertEqual(result["slices"]["first_mb_in_slice_max"], 9)
        self.assertIn("not decoded frames or FPS", result["count_units"])

    def test_slice_header_emulation_prevention_is_removed(self):
        stream = slice_nal(0xFFFFFF, 7)
        self.assertIn(b"\x00\x00\x03", stream)
        result = INSPECTOR.inspect_annex_b(stream)
        self.assertEqual(result["slices"]["first_mb_in_slice_max"], 0xFFFFFF)
        self.assertEqual(result["slices"]["slice_type_modulo_5"]["I"], 1)
        for following in range(4):
            self.assertEqual(INSPECTOR.unescape_rbsp(b"\x00\x00\x03" + bytes([following])),
                             b"\x00\x00" + bytes([following]))

    def test_malformed_truncated_slice_and_forbidden_nal_headers_are_separate(self):
        stream = (PREFIX + b"\xe1\xff" + PREFIX + b"\x60\xff"
                  + PREFIX + b"\x61" + slice_nal(0, 10)
                  + PREFIX + b"\x61\x00\x00\x03\x04"
                  + PREFIX + b"\x61\x00\x00\x03")
        result = INSPECTOR.inspect_annex_b(stream)
        self.assertEqual(result["invalid_nal_headers"], 2)
        self.assertEqual(result["slices"]["malformed_headers"], 2)
        self.assertEqual(result["slices"]["truncated_headers"], 2)
        with self.assertRaisesRegex(INSPECTOR.HeaderError, "exceeds 32 bits"):
            INSPECTOR.Bits(bytes(5)).ue()

    def test_mixed_start_codes_padding_and_empty_nals(self):
        stream = b"garbage" + PREFIX + b"\x67\x64\x80\x00\x00" + PREFIX + b"\x00\x00\x01" + b"\x68\x80\x00\x00"
        result = INSPECTOR.inspect_annex_b(stream)
        self.assertEqual(result["bytes_before_first_start_code"], 7)
        self.assertEqual(result["nal_units_total"], 3)
        self.assertEqual(result["empty_nal_units"], 1)
        self.assertEqual(result["nal_types"], {"7": 1, "8": 1})

    def test_multiple_sei_payloads_extended_lengths_and_rbsp_trailing(self):
        point = packed(ue(17) + "10" + "11")
        stream = sei_nal([(260, bytes(260)), (6, point), (5, b"\x80")])
        result = INSPECTOR.inspect_annex_b(stream)
        sei = result["sei"]
        self.assertEqual(sei["messages_parsed"], 3)
        self.assertEqual(sei["recovery_points_parsed"], 1)
        self.assertEqual(sei["truncated_nal_units"], 0)
        preview = sei["recovery_points_preview"][0]
        self.assertEqual(preview["recovery_frame_cnt"], 17)
        self.assertTrue(preview["exact_match_flag"])
        self.assertFalse(preview["broken_link_flag"])
        self.assertEqual(preview["changing_slice_group_idc"], 3)

    def test_recovery_point_truncation_and_malformed_sei_are_counted(self):
        stream = (sei_nal([(6, b"")])
                  + sei_nal([(6, b"\x80")], trailing=b"")
                  + PREFIX + b"\x06\x06\x03\x80"
                  + PREFIX + b"\x06\xff"
                  + PREFIX + b"\x06\x06"
                  + PREFIX + b"\x06\x00\x00\x03\x04")
        result = INSPECTOR.inspect_annex_b(stream)
        self.assertEqual(result["sei"]["truncated_recovery_points"], 1)
        self.assertEqual(result["sei"]["recovery_points_parsed"], 1)
        self.assertEqual(result["sei"]["truncated_nal_units"], 4)
        self.assertEqual(result["sei"]["malformed_nal_units"], 1)

    def test_recovery_preview_is_bounded_while_counts_include_all_messages(self):
        messages = [(6, packed(ue(index) + "01" + "00")) for index in range(32)]
        result = INSPECTOR.inspect_annex_b(sei_nal(messages))
        self.assertEqual(result["sei"]["recovery_points_parsed"], 32)
        self.assertEqual(len(result["sei"]["recovery_points_preview"]), 16)
        self.assertEqual(result["sei"]["recovery_frame_cnt_min"], 0)
        self.assertEqual(result["sei"]["recovery_frame_cnt_max"], 31)

    def test_non_annex_b_empty_missing_and_oversize_files_fail_clearly(self):
        for data in (b"", b"\x00\x00\x00\x18ftypmp42"):
            with self.assertRaisesRegex(INSPECTOR.InspectionError, "no Annex B start code"):
                INSPECTOR.inspect_annex_b(data)
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "capture.h264"
            with self.assertRaisesRegex(INSPECTOR.InspectionError, "cannot read input"):
                INSPECTOR.inspect_file(path)
            with path.open("wb") as source:
                source.truncate(INSPECTOR.MAX_BYTES + 1)
            with self.assertRaisesRegex(INSPECTOR.InspectionError, "64 MiB"):
                INSPECTOR.inspect_file(path)

    def test_cli_json_and_source_unchanged(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "capture.h264"
            contents = slice_nal(0, 2)
            path.write_bytes(contents)
            before = path.stat()
            completed = subprocess.run([sys.executable, str(SCRIPT), str(path)],
                                       capture_output=True, text=True, check=False)
            self.assertEqual(completed.returncode, 0, completed.stderr)
            self.assertEqual(json.loads(completed.stdout)["slices"]["slice_type_modulo_5"]["I"], 1)
            self.assertEqual(path.read_bytes(), contents)
            self.assertEqual(path.stat().st_mtime_ns, before.st_mtime_ns)
            bad = subprocess.run([sys.executable, str(SCRIPT), str(path.with_name("missing"))],
                                 capture_output=True, text=True, check=False)
            self.assertEqual(bad.returncode, 1)
            self.assertEqual(bad.stdout, "")
            self.assertIn("cannot read input", bad.stderr)


if __name__ == "__main__":
    unittest.main()
