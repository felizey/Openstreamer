import assert from "node:assert/strict";
import test from "node:test";
import { describeTrack, initSegment, fragment } from "../src/lib/fmp4.ts";

const sps = Uint8Array.from(
  Buffer.from(
    "67640034ac4d00f0044fcb3501010140000003004000001e03c70ca8",
    "hex",
  ),
);
const pps = Uint8Array.from(Buffer.from("68ee3cb0", "hex"));
const temporary = Uint8Array.from(Buffer.from("68538f20", "hex"));

function parameterSets(bytes) {
  const marker = Buffer.from(bytes).indexOf("avcC");
  assert.ok(marker > 0);
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  let offset = marker + 10;
  const spsLength = view.getUint16(offset);
  offset += 2 + spsLength;
  const count = bytes[offset++];
  const result = [];
  for (let i = 0; i < count; i++) {
    const size = view.getUint16(offset);
    offset += 2;
    result.push(bytes.subarray(offset, offset + size));
    offset += size;
  }
  return result;
}

test("MP4 initialization keeps both live and generated-IDR PPS", () => {
  const track = describeTrack(sps, [pps, temporary]);
  assert.equal(track.width, 1920);
  assert.equal(track.height, 1080);
  assert.deepEqual(parameterSets(initSegment(track)), [pps, temporary]);
});

test("a normal single-PPS stream keeps its configuration", () => {
  assert.deepEqual(parameterSets(initSegment(describeTrack(sps, [pps]))), [
    pps,
  ]);
});

test("an IDR and a live P picture remain separate MP4 samples", () => {
  const first = fragment(1, [new Uint8Array([0x65, 1, 2])], true, 0, 1500);
  const second = fragment(2, [new Uint8Array([0x41, 3, 4])], false, 1500, 1500);
  for (const data of [first, second]) {
    const marker = Buffer.from(data).indexOf("trun");
    const view = new DataView(data.buffer, data.byteOffset, data.byteLength);
    assert.equal(view.getUint32(marker + 8), 1);
  }
});
