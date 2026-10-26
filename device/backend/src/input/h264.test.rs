use super::*;

const AUD_NAL: &[u8] = &[0, 0, 0, 1, 0x09, 0xf0];
const SPS_NAL: &[u8] = &[0, 0, 0, 1, 0x67, 1, 2, 3];
const IDR_NAL: &[u8] = &[0, 0, 0, 1, 0x65, 0xb8, 9, 9];
const P_NAL: &[u8] = &[0, 0, 0, 1, 0x41, 0xe0, 7];
// first_mb_in_slice = 1, slice_type = I/P, pic_parameter_set_id = 0.
const IDR_CONTINUATION: &[u8] = &[0, 0, 1, 0x65, 0x4e, 9];
const P_CONTINUATION: &[u8] = &[0, 0, 0, 1, 0x41, 0x58, 7];

fn pictures(stream: &[&[u8]]) -> Vec<Vec<u8>> {
    let mut a = AccessUnitAssembler::new();
    let mut out = Vec::new();
    for part in stream {
        a.push(part);
        while let Some(au) = a.next_au() {
            out.push(au);
        }
    }
    out
}

fn has_picture(au: &[u8]) -> bool {
    nals(au).any(|n| n.is_vcl())
}

#[test]
fn goggles_layout_delimiter_at_the_end() {
    let out = pictures(&[SPS_NAL, IDR_NAL, AUD_NAL, P_NAL, AUD_NAL, P_NAL, AUD_NAL]);
    assert_eq!(out.len(), 3);
    assert!(out.iter().all(|au| has_picture(au)));
}

#[test]
fn doubled_delimiters_yield_no_empty_pictures() {
    // An encoder's leading delimiter plus one appended at the end
    let frame = |slice: &[u8]| [AUD_NAL, slice, AUD_NAL].concat();
    let (a, b, c) = (frame(IDR_NAL), frame(P_NAL), frame(P_NAL));
    let out = pictures(&[&a, &b, &c]);
    assert_eq!(out.len(), 3);
    assert!(out.iter().all(|au| has_picture(au)));
}

#[test]
fn no_delimiters_cuts_at_the_next_slice() {
    let out = pictures(&[SPS_NAL, IDR_NAL, P_NAL, P_NAL]);
    assert_eq!(out.len(), 2);
}

#[test]
fn several_slices_before_a_delimiter_are_one_picture() {
    let out = pictures(&[
        SPS_NAL,
        IDR_NAL,
        IDR_CONTINUATION,
        IDR_CONTINUATION,
        AUD_NAL,
        P_NAL,
        P_CONTINUATION,
        AUD_NAL,
    ]);
    assert_eq!(
        out,
        vec![
            [SPS_NAL, IDR_NAL, IDR_CONTINUATION, IDR_CONTINUATION].concat(),
            [AUD_NAL, P_NAL, P_CONTINUATION].concat(),
        ]
    );
}

#[test]
fn without_delimiters_only_a_first_slice_starts_the_next_picture() {
    let out = pictures(&[
        SPS_NAL,
        IDR_NAL,
        IDR_CONTINUATION,
        P_NAL,
        P_CONTINUATION,
        P_NAL,
    ]);
    assert_eq!(
        out,
        vec![
            [SPS_NAL, IDR_NAL, IDR_CONTINUATION].concat(),
            [P_NAL, P_CONTINUATION].concat(),
        ]
    );
}

#[test]
fn a_first_slice_header_split_between_chunks_waits_for_its_payload() {
    let mut assembler = AccessUnitAssembler::new();
    let picture = [SPS_NAL, IDR_NAL, IDR_CONTINUATION].concat();
    assembler.push(&picture);
    assembler.push(&[0, 0, 1, 0x41]);
    assert!(assembler.next_au().is_none());

    assembler.push(&[0xe0]);
    assert_eq!(assembler.next_au().unwrap(), picture);
    assert!(assembler.next_au().is_none());
}

#[test]
fn a_multibyte_first_mb_field_split_between_chunks_keeps_the_same_picture() {
    let mut assembler = AccessUnitAssembler::new();
    let picture = [SPS_NAL, P_NAL].concat();
    assembler.push(&picture);
    // first_mb_in_slice = 255 is the 17-bit code 00000000100000000.
    let continuation = &[0, 0, 0, 1, 0x41, 0x00, 0x80, 0x00];
    assembler.push(&continuation[..6]);
    assert!(assembler.next_au().is_none());
    assembler.push(&continuation[6..7]);
    assert!(assembler.next_au().is_none());
    assembler.push(&continuation[7..]);
    assert!(assembler.next_au().is_none());
    assembler.push(P_NAL);
    assert_eq!(
        assembler.next_au().unwrap(),
        [picture.as_slice(), continuation].concat()
    );
}

#[test]
fn data_partitions_b_and_c_do_not_start_another_picture() {
    let partition_a = &[0, 0, 0, 1, 0x42, 0xe0];
    let partition_b = &[0, 0, 1, 0x43, 0x80];
    let partition_c = &[0, 0, 0, 1, 0x44, 0x80];
    let out = pictures(&[SPS_NAL, partition_a, partition_b, partition_c, P_NAL]);
    assert_eq!(
        out,
        vec![[SPS_NAL, partition_a, partition_b, partition_c].concat()]
    );
}

#[test]
fn first_mb_reader_handles_fragments_escaping_and_bounded_malformed_input() {
    assert_eq!(first_mb_in_slice(&[0x80]), Ok(0));
    assert_eq!(first_mb_in_slice(&[0x40]), Ok(1));
    assert_eq!(first_mb_in_slice(&[0]), Err(SliceHeaderError::Incomplete));
    assert_eq!(
        first_mb_in_slice(&[0, 0x80]),
        Err(SliceHeaderError::Incomplete)
    );
    assert_eq!(first_mb_in_slice(&[0, 0x80, 0]), Ok(255));
    assert_eq!(
        first_mb_in_slice(&[0, 0, 3]),
        Err(SliceHeaderError::Incomplete)
    );
    assert_eq!(
        first_mb_in_slice(&[0, 0, 3, 0, 0x80, 0, 0, 3, 0]),
        Ok(16_777_215)
    );
    assert_eq!(first_mb_in_slice(&[0; 100]), Err(SliceHeaderError::Invalid));
    assert_eq!(
        first_mb_in_slice(&[0, 0, 3, 4]),
        Err(SliceHeaderError::Invalid)
    );
}

#[test]
fn a_truncated_complete_slice_does_not_wait_past_a_following_delimiter() {
    let truncated = &[0, 0, 1, 0x41, 0];
    let out = pictures(&[SPS_NAL, P_NAL, truncated, AUD_NAL]);
    assert_eq!(out, vec![[SPS_NAL, P_NAL, truncated].concat()]);
}

#[test]
fn strips_only_a_leading_delimiter() {
    let au = [AUD_NAL, SPS_NAL, IDR_NAL].concat();
    assert_eq!(strip_leading_aud(&au), [SPS_NAL, IDR_NAL].concat());
    let plain = [SPS_NAL, IDR_NAL].concat();
    assert_eq!(strip_leading_aud(&plain), plain);
    assert_eq!(strip_leading_aud(AUD_NAL), AUD_NAL);
}
