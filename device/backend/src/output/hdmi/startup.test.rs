use super::*;
const SPS: &[u8] = &[
    0, 0, 0, 1, 0x67, 0x64, 0, 0x34, 0xac, 0x4d, 0, 0xf0, 4, 0x4f, 0xcb, 0x35, 1, 1, 1, 0x40, 0, 0,
    3, 0, 0x40, 0, 0, 0x1e, 3, 0xc7, 0x0c, 0xa8,
];
const PPS: &[u8] = &[0, 0, 0, 1, 0x68, 0xee, 0x3c, 0xb0];
const P: &[u8] = &[0, 0, 0, 1, 0x41, 0xe0];

#[test]
fn headers_survive_dropped_pictures_before_a_natural_idr() {
    let data = [SPS, PPS, P].concat();
    assert_eq!(parameter_sets(&data), [SPS, PPS].concat());
    assert!(parameter_sets(P).is_empty());
    assert_eq!(
        parameter_sets(&[&SPS[1..], &PPS[1..], P].concat()),
        [SPS, PPS].concat()
    );
}

#[test]
fn each_replacement_hdmi_decoder_can_join_an_existing_p_only_stream() {
    let live = [SPS, PPS, P].concat();
    for _ in 0..3 {
        let mut decoder = DecoderStartup::default();
        assert!(decoder.before(P).is_none());
        assert!(!decoder.ready);
        let primer = decoder.before(&live).unwrap();
        assert!(decoder.ready);
        assert!(h264::nals(&primer).any(|nal| nal.kind == h264::IDR));
        assert!(decoder.before(&live).is_none());
    }
}

#[test]
fn a_source_idr_skips_local_injection_and_unsupported_pictures_wait() {
    let mut decoder = DecoderStartup::default();
    assert!(decoder.before(&[0, 0, 0, 1, 0x65, 0x88, 0x84]).is_none());
    assert!(decoder.ready);
    assert!(decoder.before(&[SPS, PPS, P].concat()).is_none());
    let mut baseline = SPS.to_vec();
    baseline[5] = 66;
    let mut decoder = DecoderStartup::default();
    assert!(decoder.before(&[&baseline, PPS, P].concat()).is_none());
    assert!(!decoder.ready);
}
