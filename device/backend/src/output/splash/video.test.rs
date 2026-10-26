use super::*;
use serde_json::json;

fn source(container: &str, codec: &str, duration: &str) -> Probe {
    serde_json::from_value(json!({
        "streams": [{
            "codec_name": codec, "width": 1920, "height": 1080,
            "profile": "High", "has_b_frames": 0, "r_frame_rate": "30/1"
        }],
        "format": {"format_name": container, "duration": duration}
    }))
    .unwrap()
}

#[test]
fn mp4_brand_distinguishes_quicktime_and_3gp() {
    assert!(mp4_brand(b"\0\0\0\x18ftypisom\0\0\0\0isommp42"));
    assert!(!mp4_brand(b"\0\0\0\x18ftypqt  \0\0\0\0qt  qt  "));
    assert!(!mp4_brand(b"\0\0\0\x18ftyp3gp4\0\0\0\0isom3gp4"));
    assert!(!mp4_brand(b"not a video"));
    assert!(
        source("mov,mp4,m4a,3gp,3g2,mj2", "hevc", "1.5")
            .validate_source(false)
            .is_ok()
    );
    let mut probe = source("mov,mp4,m4a,3gp,3g2,mj2", "hevc", "1.5");
    probe.format.as_mut().unwrap().tags = Some(FormatTags {
        major_brand: Some("isom".into()),
    });
    assert!(
        probe.validate_source(false).is_err(),
        "ffprobe's brand covers a distant ftyp box"
    );
    probe.format.as_mut().unwrap().tags = Some(FormatTags {
        major_brand: Some("qt  ".into()),
    });
    assert!(probe.validate_source(false).is_ok());
}

#[test]
fn mp4_requires_h264_but_other_containers_can_be_transcoded() {
    assert!(
        source("mov,mp4,m4a,3gp,3g2,mj2", "h264", "1.5")
            .validate_source(true)
            .is_ok()
    );
    let error = source("mov,mp4,m4a,3gp,3g2,mj2", "hevc", "1.5")
        .validate_source(true)
        .unwrap_err()
        .to_string();
    assert!(error.contains("MP4 source must contain H.264"));
    assert!(
        source("matroska,webm", "vp9", "1.5")
            .validate_source(false)
            .is_ok()
    );
    assert!(source("avi", "mpeg4", "1.5").validate_source(false).is_ok());
}

#[test]
fn refuses_unbounded_or_invalid_input() {
    for duration in ["0", "-1", "301", "NaN", "inf", "N/A"] {
        assert!(
            source("matroska,webm", "vp9", duration)
                .validate_source(false)
                .is_err()
        );
    }
    let mut probe = source("matroska,webm", "vp9", "1");
    probe.streams[0].width = Some(100_000);
    assert!(probe.validate_source(false).is_err());
    probe.streams.clear();
    assert!(
        probe
            .validate_source(false)
            .unwrap_err()
            .to_string()
            .contains("no video track")
    );
}

#[test]
fn converted_output_must_be_high_profile_1080p30_without_b_frames() {
    let mut probe = source("mov,mp4,m4a,3gp,3g2,mj2", "h264", "2");
    assert!(probe.validate_output().is_ok());
    probe.streams[0].has_b_frames = Some(2);
    assert!(probe.validate_output().is_err());
    probe.streams[0].has_b_frames = Some(0);
    probe.streams[0].r_frame_rate = Some("60/1".into());
    assert!(probe.validate_output().is_err());
    probe.streams[0].r_frame_rate = Some("30/1".into());
    probe.streams[0].profile = Some("Constrained Baseline".into());
    assert!(probe.validate_output().is_err());
}

#[test]
fn manifest_cannot_point_outside_splash_files() {
    for name in [
        "../settings.json",
        "/etc/passwd",
        "splash-video-../etc/passwd.mp4",
        "splash-video-id.mp4",
    ] {
        assert!(
            Saved {
                video: name.into(),
                preview: "splash-video-0123456789abcdef.png".into()
            }
            .validate()
            .is_err()
        );
    }
    assert!(
        Saved {
            video: "splash-video-0123456789abcdef.mp4".into(),
            preview: "splash-video-0123456789abcdef.png".into(),
        }
        .validate()
        .is_ok()
    );
}

#[test]
fn failed_install_preserves_previous_manifest_and_removes_partial_files() {
    let parent = std::env::temp_dir().join(format!(
        "openstreamer-splash-test-{:016x}",
        rand::random::<u64>()
    ));
    let semaphore = Arc::new(tokio::sync::Semaphore::new(1));
    let upload = Upload::new(&parent, Arc::clone(&semaphore).try_acquire_owned().unwrap()).unwrap();
    let temporary = upload.directory.clone();
    let old_manifest = b"previous valid splash manifest";
    std::fs::write(parent.join("splash-video.json"), old_manifest).unwrap();
    // Missing poster simulates a failed conversion/install after the MP4 move.
    std::fs::write(upload.directory.join("converted.mp4"), b"video").unwrap();
    assert!(upload.install(&parent).is_err());
    assert_eq!(
        std::fs::read(parent.join("splash-video.json")).unwrap(),
        old_manifest
    );
    assert!(
        !parent
            .join(format!("splash-video-{}.mp4", upload.id))
            .exists()
    );
    assert!(Arc::clone(&semaphore).try_acquire_owned().is_err());
    drop(upload);
    assert!(!temporary.exists());
    assert!(Arc::clone(&semaphore).try_acquire_owned().is_ok());
    std::fs::remove_dir_all(parent).unwrap();
}

#[test]
fn successful_install_roundtrips_and_releases_temporary_files() {
    let parent = std::env::temp_dir().join(format!(
        "openstreamer-splash-test-{:016x}",
        rand::random::<u64>()
    ));
    let semaphore = Arc::new(tokio::sync::Semaphore::new(1));
    let upload = Upload::new(&parent, semaphore.try_acquire_owned().unwrap()).unwrap();
    std::fs::write(upload.directory.join("converted.mp4"), b"video").unwrap();
    std::fs::write(upload.directory.join("preview.png"), b"poster").unwrap();
    let video = upload.install(&parent).unwrap();
    let loaded = Video::load(&parent).unwrap().unwrap();
    assert_eq!(loaded.path, video.path);
    assert_eq!(std::fs::read(&loaded.preview).unwrap(), b"poster");
    drop(upload);
    assert!(loaded.path.is_file());
    Video::remove_manifest(&parent).unwrap();
    loaded.remove_files();
    assert!(Video::load(&parent).unwrap().is_none());
    std::fs::remove_dir_all(parent).unwrap();
}

/// Run explicitly on the Linux build host after installing FFmpeg and the
/// GStreamer good/bad plugins. This exercises the production converter and
/// player, including the transition back to the first keyframe at EOF.
#[tokio::test]
#[ignore = "requires FFmpeg and GStreamer runtime plugins"]
async fn transcodes_and_loops_a_real_clip_with_paced_access_units() {
    use crate::input::h264;
    use std::time::Instant;

    let parent = std::env::temp_dir().join(format!(
        "openstreamer-splash-integration-{:016x}",
        rand::random::<u64>()
    ));
    let semaphore = Arc::new(tokio::sync::Semaphore::new(1));
    let upload = Upload::new(&parent, semaphore.try_acquire_owned().unwrap()).unwrap();
    let mut generate = limited_command("ffmpeg");
    generate.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-nostdin",
        "-y",
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=320x180:rate=24",
        "-t",
        "2",
        "-c:v",
        "ffv1",
        "-threads",
        "1",
        "-f",
        "matroska",
    ]);
    generate.arg(upload.source());
    run(generate, Duration::from_secs(30)).await.unwrap();
    upload.convert().await.unwrap();
    let converted = probe(&upload.directory.join("converted.mp4"))
        .await
        .unwrap();
    converted.validate_output().unwrap();
    assert!(
        std::fs::read(upload.directory.join("preview.png"))
            .unwrap()
            .starts_with(b"\x89PNG")
    );

    let video = upload.install(&parent).unwrap();
    drop(upload);
    let mut player = Player::new(Arc::clone(&video)).unwrap();
    let started = Instant::now();
    let mut received = 0;
    let mut keyframes = Vec::new();
    while received < 75 {
        assert!(
            started.elapsed() < Duration::from_secs(8),
            "Playback stalled at {received} frames"
        );
        if let Some(frame) = player.next_frame().unwrap() {
            let idr = h264::nals(&frame).any(|nal| nal.kind == h264::IDR);
            if idr {
                // New viewers and the decoder can enter at every keyframe,
                // including the first picture of the second playback loop.
                assert!(h264::nals(&frame).any(|nal| nal.kind == h264::SPS));
                keyframes.push(received);
            }
            if received == 0 {
                assert!(idr);
            }
            received += 1;
            assert!(
                player.next_frame().unwrap().is_none(),
                "Frames must be paced"
            );
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    assert_eq!(
        keyframes,
        [0, 30, 60],
        "One-second GOPs and a new loop keyframe"
    );
    assert!(started.elapsed() >= Duration::from_millis(2400));
    assert!(started.elapsed() < Duration::from_secs(5));
    drop(player);
    video.remove_files();
    Video::remove_manifest(&parent).unwrap();
    std::fs::remove_dir_all(parent).unwrap();
}

#[tokio::test]
#[ignore = "requires FFmpeg"]
async fn rejects_a_real_mp4_with_a_non_h264_video_track() {
    let parent = std::env::temp_dir().join(format!(
        "openstreamer-splash-integration-{:016x}",
        rand::random::<u64>()
    ));
    let semaphore = Arc::new(tokio::sync::Semaphore::new(1));
    let upload = Upload::new(&parent, semaphore.try_acquire_owned().unwrap()).unwrap();
    let mut generate = limited_command("ffmpeg");
    generate.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-nostdin",
        "-y",
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=160x90:rate=10",
        "-t",
        "0.2",
        "-c:v",
        "mpeg4",
        "-threads",
        "1",
        "-f",
        "mp4",
    ]);
    generate.arg(upload.source());
    run(generate, Duration::from_secs(30)).await.unwrap();
    let error = upload.convert().await.unwrap_err().to_string();
    assert!(error.contains("MP4 source must contain H.264"), "{error}");
    assert!(!upload.directory.join("converted.mp4").exists());
    drop(upload);
    std::fs::remove_dir_all(parent).unwrap();
}
