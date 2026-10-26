use super::*;

// Parameter sets only, from the user's 1080p DJI Goggles 3 capture. No picture data.
const SPS: &[u8] = &[
    0, 0, 0, 1, 0x67, 0x64, 0, 0x34, 0xac, 0x4d, 0, 0xf0, 4, 0x4f, 0xcb, 0x35, 1, 1, 1, 0x40, 0, 0,
    3, 0, 0x40, 0, 0, 0x1e, 3, 0xc7, 0x0c, 0xa8,
];
const PPS: &[u8] = &[0, 0, 0, 1, 0x68, 0xee, 0x3c, 0xb0];
const P: &[u8] = &[0, 0, 0, 1, 0x41, 0xe0]; // first_mb=0, P, PPS 0
const IDR: &[u8] = &[0, 0, 0, 1, 0x65, 0x88, 0x84];

fn ready() -> Bootstrap {
    let mut boot = Bootstrap::default();
    assert!(boot.before(SPS).is_none());
    assert!(boot.before(PPS).is_none());
    boot
}

fn units(data: &[u8]) -> Vec<&[u8]> {
    let starts: Vec<_> = data
        .windows(4)
        .enumerate()
        .filter_map(|(i, bytes)| (bytes == PREFIX).then_some(i))
        .collect();
    starts
        .iter()
        .enumerate()
        .map(|(i, &start)| &data[start..starts.get(i + 1).copied().unwrap_or(data.len())])
        .collect()
}

#[test]
fn generates_one_compact_neutral_idr_with_live_parameter_sets_and_separate_pps() {
    let mut boot = ready();
    let primer = boot.before(P).unwrap().unwrap();
    assert_eq!((primer.coded_width, primer.coded_height), (1920, 1088));
    assert!(primer.bytes.len() > 8_000 && primer.bytes.len() < 16_000);
    let nals = units(&primer.bytes);
    assert_eq!(
        nals.iter().map(|n| n[4] & 31).collect::<Vec<_>>(),
        [9, 7, 8, 8, 5, 9]
    );
    assert_eq!(nals[1], SPS);
    assert_eq!(nals[2], PPS);
    assert_eq!(parameter_ids(nals[3]), Some((1, 0)));
    for _ in 0..100 {
        assert!(boot.before(SPS).is_none());
        assert!(boot.before(PPS).is_none());
        assert!(boot.before(P).is_none());
    }
}

#[test]
fn a_real_idr_prevents_injection_and_a_new_connection_primes_again() {
    let mut boot = ready();
    assert!(boot.before(IDR).is_none());
    assert!(boot.before(P).is_none());
    assert!(ready().before(P).unwrap().is_ok());
}

#[test]
fn waits_for_sps_pps_and_the_first_slice_using_the_current_pps() {
    let mut boot = Bootstrap::default();
    assert!(boot.before(P).is_none());
    assert!(boot.before(SPS).is_none());
    assert!(boot.before(P).is_none());
    assert!(boot.before(PPS).is_none());
    let later_slice = [0, 0, 0, 1, 0x41, 0x58]; // first_mb=1, P, PPS0
    assert!(boot.before(&later_slice).is_none());
    let other_pps = [0, 0, 0, 1, 0x41, 0xd0]; // first_mb=0, P, PPS1
    assert!(boot.before(&other_pps).is_none());
    assert!(boot.before(P).unwrap().is_ok());
}

#[test]
fn temporary_pps_does_not_replace_any_seen_live_parameter_id() {
    let primer = ready().before(P).unwrap().unwrap();
    let temporary = units(&primer.bytes)[3];
    let mut boot = ready();
    assert!(boot.before(temporary).is_none());
    assert!(boot.before(PPS).is_none());
    let primer = boot.before(P).unwrap().unwrap();
    assert_eq!(parameter_ids(units(&primer.bytes)[3]), Some((2, 0)));
}

#[test]
fn unsupported_syntax_reports_once_and_does_not_generate_a_picture() {
    let mut boot = ready();
    let mut unsupported = SPS.to_vec();
    unsupported[5] = 66;
    assert!(boot.before(&unsupported).is_none());
    assert!(boot.before(P).unwrap().is_err());
    assert!(boot.before(P).is_none());
    for size in 0..15 {
        assert!(parse_sps(&SPS[..size]).is_err());
    }
    assert!(Bits::new(&[0, 0, 3, 4]).is_err());
    assert!(Bits::new(&[0, 0, 3]).is_err());
    assert!(Bits::new(&[0; 5]).unwrap().ue().is_err());
}

#[test]
fn oversized_parameters_are_not_cached_and_truncated_slices_do_not_panic() {
    let mut boot = ready();
    let mut huge = vec![0; MAX_PARAMETER_BYTES + 1];
    huge[..5].copy_from_slice(&[0, 0, 0, 1, 0x67]);
    boot.before(&huge);
    assert!(boot.before(P).is_none());
    let mut boot = ready();
    for size in 0..P.len() {
        assert!(boot.before(&P[..size]).is_none());
    }
    assert!(boot.before(P).unwrap().is_ok());
}

#[test]
fn generated_picture_has_exactly_the_macroblocks_required_by_the_live_sps() {
    let format = parse_sps(SPS).unwrap();
    assert_eq!(
        (format.frame_bits, format.width_mbs, format.height_mbs),
        (5, 120, 68)
    );
    let primer = ready().before(P).unwrap().unwrap();
    let nals = units(&primer.bytes);
    let mut bits = Bits::new(&nals[4][5..]).unwrap();
    assert_eq!(
        (bits.ue().unwrap(), bits.ue().unwrap(), bits.ue().unwrap()),
        (0, 2, 1)
    );
    assert_eq!(bits.read(5).unwrap(), 0);
    assert_eq!(bits.ue().unwrap(), 0);
    assert_eq!(bits.read(2).unwrap(), 0);
    assert_eq!((bits.ue().unwrap(), bits.ue().unwrap()), (0, 1));
    for _ in 0..8160 {
        assert_eq!(bits.ue().unwrap(), 3);
        assert_eq!(bits.ue().unwrap(), 0);
        assert_eq!(bits.ue().unwrap(), 0);
        assert_eq!(bits.read(1).unwrap(), 1);
    }
    assert_eq!(bits.read(1).unwrap(), 1); // rbsp_stop_one_bit
}
