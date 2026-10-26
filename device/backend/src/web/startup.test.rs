use super::*;
use std::time::Instant;

const SPS: &[u8] = &[
    0, 0, 0, 1, 0x67, 0x64, 0, 0x34, 0xac, 0x4d, 0, 0xf0, 4, 0x4f, 0xcb, 0x35, 1, 1, 1, 0x40, 0, 0,
    3, 0, 0x40, 0, 0, 0x1e, 3, 0xc7, 0x0c, 0xa8,
];
const PPS: &[u8] = &[0, 0, 0, 1, 0x68, 0xee, 0x3c, 0xb0];
const P: &[u8] = &[0, 0, 0, 1, 0x41, 0xe0];

fn au(bytes: &[u8], keyframe: bool) -> AccessUnit {
    AccessUnit {
        data: bytes.to_vec().into(),
        keyframe,
        assembled: Instant::now(),
    }
}

#[test]
fn a_late_viewer_gets_a_private_idr_and_live_frames_are_unchanged() {
    let mut viewer = Startup::default();
    assert!(viewer.before(&au(P, false)).is_none());
    assert!(!viewer.ready);
    assert!(viewer.before(&au(SPS, false)).is_none());
    assert!(viewer.before(&au(PPS, false)).is_none());
    let live = au(P, false);
    let original = live.data.clone();
    let primer = viewer.before(&live).unwrap();
    assert!(viewer.ready && primer.keyframe);
    assert_eq!(live.data, original);
    assert_eq!(
        h264::nals(&primer.data)
            .map(|nal| nal.kind)
            .collect::<Vec<_>>(),
        [9, 7, 8, 8, 5, 9]
    );
    assert!(primer.assembled < live.assembled);
    assert!(viewer.before(&live).is_none());
}

#[test]
fn viewers_start_independently_without_repriming_existing_viewers() {
    let input = au(&[SPS, PPS, P].concat(), false);
    let mut first = Startup::default();
    let mut second = Startup::default();
    assert!(first.before(&input).is_some());
    assert!(second.before(&input).is_some());
    assert!(first.before(&input).is_none());
    assert!(first.ready && second.ready);
}

#[test]
fn a_real_idr_is_forwarded_without_a_synthetic_picture() {
    let mut viewer = Startup::default();
    assert!(
        viewer
            .before(&au(&[0, 0, 0, 1, 0x65, 0x88, 0x84], true))
            .is_none()
    );
    assert!(viewer.ready);
    assert!(viewer.before(&au(&[SPS, PPS, P].concat(), false)).is_none());
}
