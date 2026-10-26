use super::*;
use std::sync::atomic::AtomicBool;

/// Decode the generated picture through the same AU assembly/parser boundary
/// as HDMI, using a software decoder instead of requiring V4L2/DRM hardware.
#[test]
#[ignore = "requires GStreamer h264parse and avdec_h264 runtime plugins"]
fn generated_startup_idr_decodes_after_parameter_only_access_units() {
    use gst::prelude::*;
    use gstreamer as gst;
    use gstreamer_app::{AppSink, AppSrc};

    gst::init().unwrap();
    let sps =
        hex::decode("0000000167640034ac4d00f0044fcb3501010140000003004000001e03c70ca8").unwrap();
    let pps = [0, 0, 0, 1, 0x68, 0xee, 0x3c, 0xb0];
    let p = [0, 0, 0, 1, 0x41, 0xe0];
    struct Stop(gst::Pipeline);
    impl Drop for Stop {
        fn drop(&mut self) {
            let _ = self.0.set_state(gst::State::Null);
        }
    }
    for _ in 0..2 {
        let pipeline = Stop(
            gst::parse::launch(
                "appsrc name=src is-live=true format=time do-timestamp=true ! h264parse ! \
             avdec_h264 ! video/x-raw,format=I420 ! appsink name=sink sync=false async=false",
            )
            .unwrap()
            .dynamic_cast::<gst::Pipeline>()
            .unwrap(),
        );
        let src = pipeline
            .0
            .by_name("src")
            .unwrap()
            .dynamic_cast::<AppSrc>()
            .unwrap();
        let sink = pipeline
            .0
            .by_name("sink")
            .unwrap()
            .dynamic_cast::<AppSink>()
            .unwrap();
        src.set_caps(Some(
            &gst::Caps::builder("video/x-h264")
                .field("stream-format", "byte-stream")
                .field("alignment", "au")
                .build(),
        ));
        pipeline.0.set_state(gst::State::Playing).unwrap();
        let mut boot = bootstrap::Bootstrap::default();
        boot.before(&sps);
        boot.before(&pps);
        let primer = boot.before(&p).unwrap().unwrap();
        let mut assembler = crate::input::h264::AccessUnitAssembler::new();
        assembler.push(&sps);
        assembler.push(&pps);
        assembler.push(&primer.bytes);
        while let Some(au) = assembler.next_au() {
            src.push_buffer(gst::Buffer::from_mut_slice(au)).unwrap();
        }
        src.end_of_stream().unwrap();
        let sample = sink
            .try_pull_sample(gst::ClockTime::from_seconds(5))
            .expect("generated IDR must decode");
        let caps = sample.caps().unwrap().structure(0).unwrap();
        assert_eq!(caps.get::<i32>("width").unwrap(), 1920);
        assert_eq!(caps.get::<i32>("height").unwrap(), 1080);
        let map = sample.buffer().unwrap().map_readable().unwrap();
        assert!(map.len() >= 1920 * 1080);
        assert!(map[..1920].iter().all(|pixel| *pixel == 128));
        assert!(
            sink.try_pull_sample(gst::ClockTime::from_seconds(1))
                .is_none()
        );
        assert!(
            pipeline
                .0
                .bus()
                .unwrap()
                .pop_filtered(&[gst::MessageType::Error])
                .is_none()
        );
    }
}

fn packet(channel: u8, payload: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0; 20];
    bytes[2..4].copy_from_slice(&0x1234u16.to_le_bytes());
    bytes[6] = channel;
    bytes[10..12].copy_from_slice(&0x5678u16.to_le_bytes());
    bytes.extend_from_slice(payload);
    bytes
}

fn local_socket() -> UdpSocket {
    let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    socket.set_read_timeout(Some(POLL)).unwrap();
    socket
}

#[test]
fn simulated_goggles_discovery_ack_fragmented_video_and_source_cancellation() {
    let client = local_socket();
    let server = local_socket();
    server
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let address = server.local_addr().unwrap();
    let cancelled = Arc::new(AtomicBool::new(false));
    let stop = Arc::clone(&cancelled);
    let worker = thread::spawn(move || {
        let mut frames = Vec::new();
        let mut states = Vec::new();
        stream(
            &client,
            address,
            || !stop.load(Ordering::Acquire),
            |state| states.push(state),
            |nal| {
                frames.push(nal.to_vec());
                if frames.len() == 4 {
                    stop.store(true, Ordering::Release);
                }
            },
        )
        .unwrap();
        (frames, states)
    });
    let mut buffer = [0; 256];
    let (length, client) = server.recv_from(&mut buffer).unwrap();
    assert_eq!(&buffer[..length], &protocol::DISCOVERY);
    server.send_to(&packet(1, &[]), client).unwrap();
    // An unrelated sender must not be able to feed the selected source.
    let stranger = UdpSocket::bind((Ipv4Addr::new(127, 0, 0, 2), 0)).unwrap();
    stranger
        .send_to(
            &packet(2, &[0, 0, 0, 1, 0x41, 0xff, 0, 0, 0, 1, 0x09, 0xf0]),
            client,
        )
        .unwrap();
    // The goggles' reply source port is not part of their protocol identity.
    let video_sender = local_socket();
    let sps = [0, 0, 0, 1, 0x67, 0x64, 0, 0x28];
    let pps = [0, 0, 0, 1, 0x68, 0xee, 0x3c, 0x80];
    let idr = [0, 0, 0, 1, 0x65, 0x88, 0x84];
    let p = [0, 0, 0, 1, 0x41, 0x9a];
    let aud = [0, 0, 0, 1, 0x09, 0xf0];
    let bytes = [&sps[..], &pps, &idr, &p, &aud].concat();
    for fragment in bytes.chunks(5) {
        video_sender.send_to(&packet(2, fragment), client).unwrap();
    }
    let (length, _) = server.recv_from(&mut buffer).unwrap();
    assert_eq!(length, 34);
    assert_eq!(&buffer[..4], &[0x22, 0x80, 0x34, 0x12]);
    assert_eq!(&buffer[8..12], &[0x78, 0x56, 0x78, 0x56]);
    let (frames, states) = worker.join().unwrap();
    assert_eq!(
        frames,
        vec![sps.to_vec(), pps.to_vec(), idr.to_vec(), p.to_vec()]
    );
    assert_eq!(states, vec![Goggles3State::Waiting, Goggles3State::Live]);
    assert!(cancelled.load(Ordering::Acquire));
}

#[test]
fn an_unselected_source_sends_nothing_and_a_new_session_rediscovers() {
    let server = local_socket();
    let client = local_socket();
    stream(
        &client,
        server.local_addr().unwrap(),
        || false,
        |_| {},
        |_| panic!("An unselected source cannot forward video"),
    )
    .unwrap();
    assert!(server.recv_from(&mut [0; 64]).is_err());
    let mut selected = true;
    stream(
        &client,
        server.local_addr().unwrap(),
        || std::mem::take(&mut selected),
        |_| {},
        |_| {},
    )
    .unwrap();
    let mut bytes = [0; 64];
    let (length, _) = server.recv_from(&mut bytes).unwrap();
    assert_eq!(&bytes[..length], &protocol::DISCOVERY);
}

#[test]
fn metadata_alone_cannot_mark_the_source_live() {
    let client = local_socket();
    let server = local_socket();
    server
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let peer = server.local_addr().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = Arc::clone(&stop);
    let worker = thread::spawn(move || {
        let mut states = Vec::new();
        let mut count = 0;
        stream(
            &client,
            peer,
            || !worker_stop.load(Ordering::Acquire),
            |state| states.push(state),
            |_| {
                count += 1;
                if count == 3 {
                    worker_stop.store(true, Ordering::Release);
                }
            },
        )
        .unwrap();
        (states, count)
    });
    let (_, client) = server.recv_from(&mut [0; 64]).unwrap();
    server.send_to(&packet(1, &[]), client).unwrap();
    let metadata = [
        0, 0, 0, 1, 0x67, 0x64, 0, 0x28, 0, 0, 0, 1, 0x68, 0xee, 0x3c, 0x80, 0, 0, 0, 1, 0x09,
        0xf0, 0, 0, 0, 1, 0x09, 0xf0,
    ];
    server.send_to(&packet(2, &metadata), client).unwrap();
    let (states, count) = worker.join().unwrap();
    assert_eq!(count, 3);
    assert_eq!(states, vec![Goggles3State::Waiting]);
}

#[test]
fn control_arriving_after_video_does_not_replace_the_live_state() {
    let client = local_socket();
    let server = local_socket();
    server
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let peer = server.local_addr().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = Arc::clone(&stop);
    let worker = thread::spawn(move || {
        let mut states = Vec::new();
        let mut count = 0;
        stream(
            &client,
            peer,
            || !worker_stop.load(Ordering::Acquire),
            |state| states.push(state),
            |_| {
                count += 1;
                if count == 2 {
                    worker_stop.store(true, Ordering::Release);
                }
            },
        )
        .unwrap();
        (states, count)
    });
    let (_, client) = server.recv_from(&mut [0; 64]).unwrap();
    server
        .send_to(
            &packet(2, &[0, 0, 0, 1, 0x41, 0x9a, 0, 0, 0, 1, 0x09, 0xf0]),
            client,
        )
        .unwrap();
    server.send_to(&packet(1, &[]), client).unwrap();
    server
        .send_to(&packet(2, &[0, 0, 0, 1, 0x09, 0xf0]), client)
        .unwrap();
    let (states, count) = worker.join().unwrap();
    assert_eq!(count, 2);
    assert_eq!(states, vec![Goggles3State::Live]);
}

#[test]
fn diagnostics_count_actual_nal_types_and_log_first_occurrences_once() {
    let mut diagnostics = SessionDiagnostics::default();
    let elapsed = Duration::from_millis(20);
    for kind in [0x67, 0x68] {
        let expected = if kind == 0x67 { "SPS" } else { "PPS" };
        assert_eq!(
            diagnostics.observe(&[0, 0, 0, 1, kind], elapsed),
            Some(expected)
        );
        assert_eq!(diagnostics.observe(&[0, 0, 0, 1, kind], elapsed), None);
    }
    // Seeing the parameter sets does not prove that an IDR was sent.
    assert_eq!(diagnostics.counts.idr, 0);
    assert_eq!(diagnostics.first_vcl, None);
    assert_eq!(
        diagnostics.observe(&[0, 0, 0, 1, 0x65], elapsed),
        Some("IDR")
    );
    assert_eq!(diagnostics.observe(&[0, 0, 0, 1, 0x65], elapsed), None);
    assert_eq!(diagnostics.observe(&[0, 0, 0, 1, 0x41], elapsed), None);
    assert_eq!(diagnostics.observe(&[0, 0, 0, 1, 0x09], elapsed), None);
    assert_eq!(
        diagnostics.counts,
        NalCounts {
            total: 8,
            vcl: 3,
            sps: 2,
            pps: 2,
            idr: 2,
        }
    );
    assert_eq!(diagnostics.first_vcl, Some(elapsed));
    for invalid in [&[][..], &[0, 0, 0, 1][..], &[0, 0, 0, 1, 0x80][..]] {
        assert_eq!(diagnostics.observe(invalid, elapsed), None);
    }
    assert_eq!(diagnostics.counts.total, 8);
    assert!(!diagnostics.idr_notice_due(Duration::from_secs(60)));
}

#[test]
fn diagnostics_summary_is_early_and_emitted_once_per_session() {
    let mut diagnostics = SessionDiagnostics::default();
    assert!(!diagnostics.summary_due(Duration::from_millis(1_999)));
    assert!(diagnostics.summary_due(Duration::from_secs(2)));
    assert!(!diagnostics.summary_due(Duration::from_secs(3)));
    let mut with_video = SessionDiagnostics::default();
    with_video.observe(&[0, 0, 0, 1, 0x41], Duration::from_millis(20));
    assert!(with_video.summary_due(Duration::from_millis(20)));
    assert!(!with_video.summary_due(Duration::from_secs(2)));
}

#[test]
fn diagnostics_no_idr_notice_starts_from_actual_video_and_is_not_a_gate() {
    let mut diagnostics = SessionDiagnostics::default();
    for kind in [0x67, 0x68, 0x09] {
        diagnostics.observe(&[0, 0, 0, 1, kind], Duration::ZERO);
    }
    assert!(!diagnostics.idr_notice_due(Duration::from_secs(60)));
    diagnostics.observe(&[0, 0, 0, 1, 0x41], Duration::from_secs(60));
    assert!(!diagnostics.idr_notice_due(Duration::from_millis(64_999)));
    assert!(diagnostics.idr_notice_due(Duration::from_secs(65)));
    assert!(!diagnostics.idr_notice_due(Duration::from_secs(66)));
    // A late real IDR is still observed; the notice does not stop forwarding.
    assert_eq!(
        diagnostics.observe(&[0, 0, 0, 1, 0x65], Duration::from_secs(67)),
        Some("IDR")
    );
    assert_eq!(diagnostics.counts.vcl, 2);
    assert_eq!(diagnostics.counts.idr, 1);
}

#[test]
fn new_udp_session_diagnostics_do_not_inherit_previous_sps_idr_or_timers() {
    let mut previous = SessionDiagnostics::default();
    previous.observe(&[0, 0, 0, 1, 0x67], Duration::ZERO);
    previous.observe(&[0, 0, 0, 1, 0x65], Duration::from_millis(20));
    assert!(previous.summary_due(Duration::from_millis(20)));

    let mut next = SessionDiagnostics::default();
    assert_eq!(next.counts, NalCounts::default());
    assert_eq!(next.first_vcl, None);
    assert!(!next.summary_sent);
    assert_eq!(
        next.observe(&[0, 0, 0, 1, 0x67], Duration::ZERO),
        Some("SPS")
    );
    assert_eq!(next.counts.idr, 0);
    next.observe(&[0, 0, 0, 1, 0x41], Duration::from_millis(100));
    assert!(!next.idr_notice_due(Duration::from_millis(5_099)));
    assert!(next.idr_notice_due(Duration::from_millis(5_100)));
    assert!(!next.idr_notice_due(Duration::from_secs(10)));
}
