use gstreamer::glib::{self, LogLevel};

// This V4L2 message uses GLib g_warning(), not the GStreamer bus/debug logger.
// Keep the decoder's other diagnostics and all actual errors unchanged.
pub fn init() {
    glib::log_set_default_handler(|domain, level, message| {
        if !suppressed(domain, level, message) {
            glib::log_default_handler(domain, level, Some(message));
        }
    });
}

fn suppressed(domain: Option<&str>, level: LogLevel, message: &str) -> bool {
    if level != LogLevel::Warning || !matches!(domain, None | Some("GStreamer")) {
        return false;
    }
    message
        .strip_suffix(": Too old frames, bug in decoder -- please file a bug")
        .and_then(|source| source.strip_prefix("v4l2h264dec"))
        .is_some_and(|index| !index.is_empty() && index.bytes().all(|byte| byte.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;
    const NOISE: &str = "v4l2h264dec17: Too old frames, bug in decoder -- please file a bug";

    #[test]
    fn suppresses_only_the_repeated_v4l2_warning() {
        assert!(suppressed(None, LogLevel::Warning, NOISE));
        assert!(suppressed(Some("GStreamer"), LogLevel::Warning, NOISE));
        assert!(suppressed(
            None,
            LogLevel::Warning,
            &NOISE.replace("dec17", "dec0")
        ));
    }

    #[test]
    fn preserves_errors_and_other_decoder_diagnostics() {
        assert!(!suppressed(None, LogLevel::Error, NOISE));
        assert!(!suppressed(None, LogLevel::Critical, NOISE));
        assert!(!suppressed(Some("OtherLibrary"), LogLevel::Warning, NOISE));
        assert!(!suppressed(
            None,
            LogLevel::Warning,
            "v4l2h264dec17: Failed to process frame"
        ));
        assert!(!suppressed(
            None,
            LogLevel::Warning,
            &NOISE.replace("v4l2h264dec17", "other_decoder")
        ));
    }

    #[test]
    fn glib_warning_filter_forwards_unrelated_messages() {
        const CHILD: &str = "OPENSTREAMER_TEST_GLIB_LOG_CHILD";
        if std::env::var_os(CHILD).is_some() {
            init();
            glib::g_warning!(
                "GStreamer",
                "v4l2h264dec17: Too old frames, bug in decoder -- please file a bug"
            );
            glib::g_warning!(
                "GStreamer",
                "test unrelated decoder warning remains visible"
            );
            return;
        }
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "gst_log::tests::glib_warning_filter_forwards_unrelated_messages",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(result.status.success());
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(!stderr.contains("Too old frames"), "{stderr}");
        assert!(
            stderr.contains("test unrelated decoder warning remains visible"),
            "{stderr}"
        );
    }
}
