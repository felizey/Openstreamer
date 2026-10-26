//! The observed Goggles 3 USB UDP video transport.
//!
//! The receiver strips a 20-byte UDP header and concatenates the video payload
//! as Annex B H.264. Incoming RX IDs are echoed in acknowledgements; their
//! sequence rules are not established, so the receiver does not assume they
//! increment contiguously.
//!
//! The meaning of the incoming length/checksum fields has not been established.
//! Inventing sequence rules here would risk discarding a working stream. UDP
//! loss/reordering recovery still
//! needs validation with real goggles; reset drops partial data. Complete NALs
//! are forwarded as received, including pictures before the first IDR. Decoder
//! synchronization is supplemented by the separate bootstrap module, which
//! generates a neutral IDR from supported live parameters. No prerecorded
//! keyframe arrays are used by this transport.

use crate::input::h264::find_start_code;

const HEADER_LEN: usize = 20;
const MAX_DATAGRAM: usize = 65_507;
const MAX_BUFFER: usize = 4 * 1024 * 1024;
const START_CODE: [u8; 4] = [0, 0, 0, 1];

/// Complete start/discovery datagram, repeated until the control channel answers.
pub const DISCOVERY: [u8; 48] = [
    0x30, 0x80, 0xe9, 0x30, 0x00, 0x00, 0x00, 0x69, 0xd0, 0xe9, 0x64, 0x00, 0x64, 0x00, 0xc0, 0x05,
    0x14, 0x00, 0x00, 0x0a, 0x00, 0x64, 0x00, 0x64, 0x00, 0xc0, 0x05, 0x14, 0x00, 0x00, 0x64, 0x00,
    0x14, 0x00, 0x64, 0x00, 0xc0, 0x05, 0x14, 0x00, 0x00, 0x64, 0x00, 0x01, 0x01, 0x04, 0x0a, 0x02,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Channel {
    Control,
    Video,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Header {
    pub channel: Channel,
    pub echo_word: u16,
    pub rx_id: u16,
}

#[derive(Debug, Eq, PartialEq)]
pub struct Packet<'a> {
    pub header: Header,
    pub payload: &'a [u8],
}

/// The reference dispatcher uses the actual datagram length. All fields we
/// inspect are inside the fixed header; unknown fields are deliberately ignored.
pub fn parse_packet(data: &[u8]) -> Option<Packet<'_>> {
    if !(HEADER_LEN..=MAX_DATAGRAM).contains(&data.len()) {
        return None;
    }
    let channel = match data[6] {
        1 => Channel::Control,
        2 => Channel::Video,
        _ => return None,
    };
    Some(Packet {
        header: Header {
            channel,
            echo_word: u16::from_le_bytes([data[2], data[3]]),
            rx_id: u16::from_le_bytes([data[10], data[11]]),
        },
        payload: &data[HEADER_LEN..],
    })
}

/// ACKs have a fixed byte 7 (0x7f); it is not the outgoing DUML XOR checksum.
pub fn ack(header: Header) -> [u8; 34] {
    let mut data = [0; 34];
    data[..2].copy_from_slice(&[0x22, 0x80]);
    data[2..4].copy_from_slice(&header.echo_word.to_le_bytes());
    data[6..8].copy_from_slice(&[0x04, 0x7f]);
    data[8..10].copy_from_slice(&header.rx_id.to_le_bytes());
    data[10..12].copy_from_slice(&header.rx_id.to_le_bytes());
    data
}

/// Reassembles NALs split across UDP packets. A NAL is complete only when the
/// next start code arrives, just as in the observed DJI Goggles 3 receiver. Memory is
/// bounded even if a sender never provides a start code or end of NAL.
pub struct VideoAssembler {
    buf: Vec<u8>,
    /// Already searched bytes inside the final, incomplete NAL. Only the
    /// trailing three bytes can become a prefix when another packet arrives.
    search_from: usize,
}

impl VideoAssembler {
    pub fn new() -> Self {
        Self {
            buf: Vec::with_capacity(128 * 1024),
            search_from: 0,
        }
    }

    /// A new peer/session must never inherit a partial NAL.
    pub fn reset(&mut self) {
        self.buf.clear();
        self.search_from = 0;
    }

    pub fn push(&mut self, data: &[u8]) -> Vec<Vec<u8>> {
        if data.len() > MAX_BUFFER {
            self.reset();
            return Vec::new();
        }
        if self.buf.len() + data.len() > MAX_BUFFER {
            self.reset();
        }
        self.buf.extend_from_slice(data);

        let Some((mut start, mut prefix_len)) = find_start_code(&self.buf, 0) else {
            // Three trailing zeros might be the start of the next packet's
            // four-byte prefix. Everything before them cannot start a NAL.
            if self.buf.len() > 3 {
                self.buf.drain(..self.buf.len() - 3);
            }
            self.search_from = 0;
            return Vec::new();
        };

        let mut result = Vec::new();
        let mut consumed = start;
        let mut search_from = self.search_from.max(start + prefix_len);
        while let Some((next, next_prefix_len)) = find_start_code(&self.buf, search_from) {
            let header = start + prefix_len;
            if header < next {
                let mut nal = Vec::with_capacity(START_CODE.len() + next - header);
                nal.extend_from_slice(&START_CODE);
                nal.extend_from_slice(&self.buf[header..next]);
                Self::complete_nal(nal, &mut result);
            }
            consumed = next;
            start = next;
            prefix_len = next_prefix_len;
            search_from = start + prefix_len;
        }
        self.search_from = self.buf.len().saturating_sub(3).max(start + prefix_len) - consumed;
        if consumed > 0 {
            self.buf.drain(..consumed);
        }
        result
    }

    fn complete_nal(nal: Vec<u8>, result: &mut Vec<Vec<u8>>) {
        let header = nal[4];
        let kind = header & 0x1f;
        if header & 0x80 != 0 || !(1..=23).contains(&kind) {
            return;
        }
        result.push(nal);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPS: &[u8] = &[0, 0, 0, 1, 0x67, 0x64, 0, 0x28];
    const PPS: &[u8] = &[0, 0, 0, 1, 0x68, 0xee, 0x3c, 0x80];
    const IDR: &[u8] = &[0, 0, 0, 1, 0x65, 0x88, 0x84];
    const P: &[u8] = &[0, 0, 0, 1, 0x41, 0x9a];
    const AUD: &[u8] = &[0, 0, 0, 1, 0x09, 0xf0];

    fn stream(parts: &[&[u8]]) -> Vec<u8> {
        parts.concat()
    }

    fn header(channel: Channel, echo_word: u16, rx_id: u16) -> Header {
        Header {
            channel,
            echo_word,
            rx_id,
        }
    }

    #[test]
    fn discovery_matches_the_documented_complete_wire_vector() {
        assert_eq!(
            hex::encode(DISCOVERY),
            "3080e93000000069d0e964006400c0051400000a0064006400c005140000640014006400c00514000064000101040a02"
        );
    }

    #[test]
    fn ack_matches_wire_vector_and_echoes_wrapping_metadata() {
        assert_eq!(
            hex::encode(ack(header(Channel::Video, 0x1234, 0x5678))),
            "228034120000047f7856785600000000000000000000000000000000000000000000"
        );
        for rx_id in [0xfffe, 0xffff, 0, 1, 32000] {
            let data = ack(header(Channel::Control, 0xabcd, rx_id));
            assert_eq!(&data[2..4], &[0xcd, 0xab]);
            assert_eq!(&data[8..10], &rx_id.to_le_bytes());
            assert_eq!(&data[10..12], &rx_id.to_le_bytes());
            assert_eq!(data[7], 0x7f);
        }
    }

    #[test]
    fn incoming_header_uses_actual_datagram_bounds_and_little_endian_metadata() {
        let mut bytes = [0; HEADER_LEN + 3];
        bytes[2..4].copy_from_slice(&0x1234u16.to_le_bytes());
        bytes[6] = 2;
        bytes[10..12].copy_from_slice(&0xfffeu16.to_le_bytes());
        bytes[20..].copy_from_slice(&[1, 2, 3]);
        assert_eq!(
            parse_packet(&bytes),
            Some(Packet {
                header: header(Channel::Video, 0x1234, 0xfffe),
                payload: &[1, 2, 3],
            })
        );
        for len in 0..HEADER_LEN {
            assert!(parse_packet(&bytes[..len]).is_none());
        }
        for channel in 0..=255 {
            bytes[6] = channel;
            assert_eq!(parse_packet(&bytes).is_some(), channel == 1 || channel == 2);
        }
        let mut largest = vec![0; MAX_DATAGRAM];
        largest[6] = 1;
        assert!(parse_packet(&largest).is_some());
        largest.push(0);
        assert!(parse_packet(&largest).is_none());
    }

    #[test]
    fn fragmented_stream_reassembles_at_every_byte_boundary() {
        let bytes = stream(&[SPS, PPS, IDR, P, AUD]);
        let expected = vec![SPS.to_vec(), PPS.to_vec(), IDR.to_vec(), P.to_vec()];
        for split in 0..=bytes.len() {
            let mut assembler = VideoAssembler::new();
            let mut actual = assembler.push(&bytes[..split]);
            actual.extend(assembler.push(&bytes[split..]));
            assert_eq!(actual, expected, "split at {split}");
        }
        let mut assembler = VideoAssembler::new();
        let actual: Vec<_> = bytes
            .iter()
            .flat_map(|byte| assembler.push(std::slice::from_ref(byte)))
            .collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn large_nal_uses_an_incremental_cursor_and_reset_starts_a_new_stream() {
        let mut assembler = VideoAssembler::new();
        let mut nal = IDR.to_vec();
        nal.resize(100 * 1024, 0x55);
        for fragment in nal.chunks(1200) {
            assert!(assembler.push(fragment).is_empty());
            // A large NAL must not rescan its earlier payload for every
            // datagram. Retain just enough overlap for a split start code.
            assert_eq!(
                assembler.search_from,
                assembler.buf.len().saturating_sub(3).max(4)
            );
        }
        assert_eq!(assembler.push(AUD), vec![nal]);
        assert_eq!(assembler.buf, AUD);
        assembler.reset();
        assert_eq!(assembler.search_from, 0);
        assert!(assembler.buf.is_empty());
        assert_eq!(assembler.push(&stream(&[P, AUD])), vec![P.to_vec()]);
    }

    #[test]
    fn forwards_startup_pictures_without_inventing_an_idr_requirement() {
        let mut assembler = VideoAssembler::new();
        let actual = assembler.push(&stream(&[P, SPS, &PPS[1..], P, AUD]));
        assert_eq!(
            actual,
            vec![P.to_vec(), SPS.to_vec(), PPS.to_vec(), P.to_vec()]
        );
    }

    #[test]
    fn reset_and_overflow_cannot_emit_old_or_partial_video() {
        let mut assembler = VideoAssembler::new();
        assert_eq!(assembler.push(&stream(&[SPS, PPS, IDR, P])).len(), 3);
        assembler.reset();
        assert_eq!(assembler.push(&stream(&[SPS, PPS])), vec![SPS.to_vec()]);
        assert!(assembler.push(&vec![0x55; MAX_BUFFER + 1]).is_empty());
        assert_eq!(assembler.push(&stream(&[IDR, AUD])), vec![IDR.to_vec()]);
        assert!(assembler.buf.len() <= MAX_BUFFER);
    }

    #[test]
    fn incomplete_nal_is_bounded_and_resumes_at_a_new_start_code_after_overflow() {
        let mut assembler = VideoAssembler::new();
        assembler.push(&stream(&[SPS, PPS, IDR, P]));
        assembler.push(&vec![0x55; MAX_BUFFER - P.len()]);
        assert_eq!(assembler.buf.len(), MAX_BUFFER);
        assert!(assembler.push(&[0x55]).is_empty());
        assert!(assembler.buf.len() <= 3);
        assert_eq!(assembler.push(&stream(&[IDR, AUD])), vec![IDR.to_vec()]);
    }

    #[test]
    fn parameter_set_changes_are_forwarded_in_received_order() {
        let mut assembler = VideoAssembler::new();
        let mut changed_sps = SPS.to_vec();
        changed_sps[7] = 0x29;
        let actual = assembler.push(&stream(&[SPS, PPS, IDR, &changed_sps, P, PPS, AUD]));
        assert_eq!(
            actual,
            vec![
                SPS.to_vec(),
                PPS.to_vec(),
                IDR.to_vec(),
                changed_sps,
                P.to_vec(),
                PPS.to_vec()
            ]
        );
    }

    #[test]
    fn skips_invalid_nals_without_poisoning_later_complete_nals() {
        let mut assembler = VideoAssembler::new();
        let forbidden = &[0, 0, 0, 1, 0xe5, 0x88][..];
        let invalid_type = &[0, 0, 0, 1, 0x60, 0x88][..];
        assert_eq!(
            assembler.push(&stream(&[forbidden, invalid_type, P, AUD])),
            vec![P.to_vec()]
        );
    }
}
