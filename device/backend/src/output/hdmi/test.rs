use super::*;
use gstreamer_app::AppSink;

// Exercise the production appsrc setup and replacement lifecycle without
// requiring a V4L2 decoder, a DRM display, or a Raspberry Pi.
fn test_running() -> Result<Running, String> {
    gst::init().unwrap();
    let pipeline = gst::parse::launch(
        "appsrc name=src is-live=true format=time ! identity ! \
         appsink name=sink sync=false async=false",
    )
    .map_err(|error| error.to_string())?
    .dynamic_cast::<gst::Pipeline>()
    .unwrap();
    Ok(finish_build(pipeline))
}

fn start() -> Running {
    let mut running = None;
    replace_running(&mut running, test_running).unwrap();
    running.unwrap()
}

fn sample(running: &Running) -> gst::Sample {
    running
        .sink
        .clone()
        .dynamic_cast::<AppSink>()
        .unwrap()
        .try_pull_sample(gst::ClockTime::from_seconds(2))
        .expect("the software HDMI test pipeline must deliver the buffer")
}

fn picture(marker: u8) -> Vec<u8> {
    vec![0, 0, 0, 1, 0x65, marker, 0x84]
}

fn bytes(sample: &gst::Sample) -> Vec<u8> {
    sample
        .buffer()
        .unwrap()
        .map_readable()
        .unwrap()
        .as_slice()
        .to_vec()
}

#[test]
fn appsrc_stamps_live_pictures_and_marks_the_stream_boundary() {
    let running = start();
    assert_eq!(running.pipeline.current_state(), gst::State::Playing);

    running.push_buffer(picture(1), true).unwrap();
    let first = sample(&running);
    let first_buffer = first.buffer().unwrap();
    let first_pts = first_buffer.pts().expect("live buffers need a timestamp");
    assert!(first_buffer.flags().contains(gst::BufferFlags::DISCONT));
    assert_eq!(bytes(&first), picture(1));

    thread::sleep(Duration::from_millis(5));
    running.push_buffer(picture(2), false).unwrap();
    let second = sample(&running);
    let second_buffer = second.buffer().unwrap();
    let second_pts = second_buffer
        .pts()
        .expect("every live buffer needs a timestamp");
    assert!(
        second_pts > first_pts,
        "timestamps must advance within a stream"
    );
    assert!(!second_buffer.flags().contains(gst::BufferFlags::DISCONT));
    assert_eq!(bytes(&second), picture(2));
    assert!(!running.failed.load(Ordering::Acquire));
}

#[test]
fn replacement_closes_the_old_pipeline_before_starting_the_new_one() {
    let mut running = Some(start());
    let old = running.as_ref().unwrap();
    let old_pipeline = old.pipeline.clone();
    let old_appsrc = old.appsrc.clone();
    old.push_buffer(picture(0xaa), true).unwrap();
    // Deliberately leave the old stream's picture queued and unconsumed.

    replace_running(&mut running, || {
        assert_eq!(old_pipeline.current_state(), gst::State::Null);
        test_running()
    })
    .unwrap();

    assert_eq!(old_pipeline.current_state(), gst::State::Null);
    assert!(
        old_appsrc
            .push_buffer(gst::Buffer::from_slice(picture(0xbb)))
            .is_err()
    );
    let fresh = running.as_ref().unwrap();
    assert_ne!(fresh.pipeline, old_pipeline);
    assert_eq!(fresh.pipeline.current_state(), gst::State::Playing);
    fresh.push_buffer(picture(0xcc), true).unwrap();
    let resumed = sample(fresh);
    assert_eq!(bytes(&resumed), picture(0xcc));
    assert!(resumed.buffer().unwrap().pts().is_some());
    assert!(
        resumed
            .buffer()
            .unwrap()
            .flags()
            .contains(gst::BufferFlags::DISCONT)
    );
    assert!(
        fresh
            .sink
            .clone()
            .dynamic_cast::<AppSink>()
            .unwrap()
            .try_pull_sample(gst::ClockTime::ZERO)
            .is_none()
    );
}

#[test]
fn failed_replacement_keeps_no_old_decoder_and_can_be_retried() {
    let mut running = Some(start());
    let old_pipeline = running.as_ref().unwrap().pipeline.clone();
    let error = replace_running(&mut running, || Err("test creation failure".into()));
    assert_eq!(error.unwrap_err(), "test creation failure");
    assert!(running.is_none());
    assert_eq!(old_pipeline.current_state(), gst::State::Null);

    replace_running(&mut running, test_running).unwrap();
    let recovered = running.as_ref().unwrap();
    recovered.push_buffer(picture(3), true).unwrap();
    assert_eq!(bytes(&sample(recovered)), picture(3));
    assert!(!recovered.failed.load(Ordering::Acquire));
}

#[test]
fn only_bus_errors_request_decoder_recovery() {
    let running = start();
    running
        .pipeline
        .post_message(
            gst::message::Warning::builder(gst::CoreError::Failed, "test decoder warning")
                .src(&running.pipeline)
                .build(),
        )
        .unwrap();
    assert!(!running.failed.load(Ordering::Acquire));

    running
        .pipeline
        .post_message(
            gst::message::Error::builder(gst::CoreError::Failed, "test decoder error")
                .src(&running.pipeline)
                .build(),
        )
        .unwrap();
    assert!(running.failed.load(Ordering::Acquire));
}

#[test]
fn a_rejected_appsrc_buffer_requests_decoder_recovery() {
    let running = start();
    running.appsrc.end_of_stream().unwrap();
    assert!(running.push_buffer(picture(4), false).is_err());
    assert!(running.failed.load(Ordering::Acquire));
}
