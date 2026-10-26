use super::*;

const SPS: &[u8] = &[0, 0, 0, 1, 0x67, 0x42, 0, 0x1e];
const PPS: &[u8] = &[0, 0, 0, 1, 0x68, 0xce, 0x3c];
const IDR: &[u8] = &[0, 0, 0, 1, 0x65, 0x88, 0x84];
const AUD: &[u8] = &[0, 0, 0, 1, 0x09, 0xf0];

fn fresh_picture() -> Vec<u8> {
    [SPS, PPS, IDR].concat()
}

fn push_picture(ingress: &mut Ingress) -> Vec<u8> {
    let picture = fresh_picture();
    ingress.assembler.push(&picture);
    ingress.assembler.push(AUD);
    let actual = ingress.assembler.next_au().unwrap();
    assert!(ingress.assembler.next_au().is_none());
    actual
}

#[test]
fn immediate_new_session_drops_the_previous_unfinished_access_unit() {
    let mut ingress = Ingress::new();
    assert!(!ingress.select(Feed::Live(InputMode::DjiGoggles3, 1)));
    let old_picture = [SPS, &[0, 0, 0, 1, 0x41, 0xaa, 0xbb]].concat();
    ingress.assembler.push(&old_picture);
    assert!(ingress.assembler.next_au().is_none());

    // A new session is a boundary even when no inactivity timeout elapsed.
    assert!(ingress.select(Feed::Live(InputMode::DjiGoggles3, 2)));
    assert_eq!(push_picture(&mut ingress), fresh_picture());
}

#[test]
fn new_session_drops_a_nal_split_during_disconnect() {
    let mut ingress = Ingress::new();
    ingress.select(Feed::Live(InputMode::DjiFpv, 5));
    ingress
        .assembler
        .push(&[0, 0, 0, 1, 0x67, 0xaa, 0xbb, 0, 0]);
    assert!(ingress.assembler.next_au().is_none());

    assert!(ingress.select(Feed::Live(InputMode::DjiFpv, 6)));
    assert_eq!(push_picture(&mut ingress), fresh_picture());
}

#[test]
fn live_splash_and_live_need_one_rebuild_per_transition() {
    let mut ingress = Ingress::new();
    assert!(!ingress.select(Feed::Live(InputMode::DjiFpv, 1)));
    ingress.assembler.push(&[0, 0, 1, 0x41, 0xaa]);

    assert!(ingress.select(Feed::Splash));
    assert!(ingress.assembler.next_au().is_none());
    assert!(ingress.take_discont());
    assert!(!ingress.select(Feed::Splash));
    assert!(!ingress.take_discont());

    // Any incomplete bytes must be discarded when live video takes over.
    ingress.assembler.push(&[0, 0, 1, 0x65, 0xbb]);
    assert!(ingress.select(Feed::Live(InputMode::DjiFpv, 2)));
    assert_eq!(push_picture(&mut ingress), fresh_picture());
    assert!(ingress.take_discont());
    assert!(!ingress.select(Feed::Live(InputMode::DjiFpv, 2)));
    assert!(!ingress.take_discont());
}

#[test]
fn switching_sources_is_a_boundary_even_with_the_same_session_number() {
    let mut ingress = Ingress::new();
    assert!(!ingress.select(Feed::Live(InputMode::Uvc, 1)));
    ingress.assembler.push(&[0, 0, 0, 1, 0x41, 0xaa]);

    assert!(ingress.select(Feed::Live(InputMode::DjiFpv, 1)));
    assert_eq!(push_picture(&mut ingress), fresh_picture());
}

#[test]
fn repeated_feed_selection_keeps_a_picture_split_across_chunks() {
    let mut ingress = Ingress::new();
    let live = Feed::Live(InputMode::DjiGoggles3, 7);
    assert!(!ingress.select(live));
    assert!(ingress.take_discont());
    let picture = fresh_picture();
    ingress.assembler.push(&picture[..10]);
    assert!(ingress.assembler.next_au().is_none());

    assert!(!ingress.select(live));
    assert!(!ingress.take_discont());
    ingress.assembler.push(&picture[10..]);
    ingress.assembler.push(AUD);
    assert_eq!(ingress.assembler.next_au().unwrap(), picture);
}

#[test]
fn reset_starts_a_fresh_pipeline_and_marks_only_its_first_buffer() {
    let mut ingress = Ingress::new();
    let live = Feed::Live(InputMode::DjiFpv, 2);
    assert!(ingress.take_discont());
    assert!(!ingress.take_discont());
    assert!(!ingress.select(live));
    ingress.assembler.push(&[0, 0, 0, 1, 0x41, 0xaa]);
    ingress.take_discont();

    ingress.reset();
    assert!(ingress.take_discont());
    assert!(!ingress.take_discont());
    assert!(ingress.assembler.next_au().is_none());
    assert!(!ingress.select(live));
    assert!(ingress.take_discont());
    assert!(!ingress.take_discont());
    assert_eq!(push_picture(&mut ingress), fresh_picture());
}

#[test]
fn discarding_failed_decoder_data_keeps_the_current_feed() {
    let mut ingress = Ingress::new();
    let live = Feed::Live(InputMode::DjiGoggles3, 9);
    ingress.select(live);
    ingress.take_discont();
    ingress.assembler.push(&[0, 0, 1, 0x41, 0xaa]);

    ingress.discard_partial();
    assert!(ingress.take_discont());
    assert!(!ingress.take_discont());
    assert!(!ingress.select(live));
    assert_eq!(push_picture(&mut ingress), fresh_picture());
}
