use super::*;

#[test]
fn removed_input_options_are_ignored_when_loading_saved_settings() {
    let cfg: Goggles3Settings = serde_json::from_str(
        r#"{"transport":"wifi","interface":"wlan0","boost":true,"boost_kbps":30000}"#,
    )
    .unwrap();
    assert!(cfg.interface.is_empty());
    assert_eq!(
        serde_json::to_value(cfg).unwrap(),
        serde_json::json!({"interface":""})
    );
    let usb: Goggles3Settings =
        serde_json::from_str(r#"{"interface":"usb0","boost":true}"#).unwrap();
    assert_eq!(usb.interface, "usb0");
    assert!(usb.validate().is_ok());
}

fn uvc(mode: &str, bitrate_kbps: u32, keyframe_secs: u32) -> UvcSettings {
    UvcSettings {
        mode: mode.into(),
        bitrate_kbps,
        keyframe_secs,
        ..Default::default()
    }
}

#[test]
fn reads_a_camera_mode() {
    assert_eq!(parse_uvc_mode("1280x720@30"), Some((1280, 720, 30)));
    assert_eq!(parse_uvc_mode("1920x1080@60"), Some((1920, 1080, 60)));
}

#[test]
fn rejects_malformed_camera_modes() {
    for mode in [
        "",
        "1280x720",
        "1280@30",
        "1280x@30",
        "x720@30",
        "1280x720@",
        "1280X720@30",
        "1280x720@30fps",
        "-1x720@30",
    ] {
        assert_eq!(parse_uvc_mode(mode), None, "{mode:?} got through");
    }
}

#[test]
fn the_defaults_are_valid() {
    assert!(UvcSettings::default().validate().is_ok());
}

#[test]
fn checks_mode_bitrate_and_keyframe_interval() {
    assert!(uvc("", 6000, 2).validate().is_ok());
    assert!(uvc("1280x720@30", 500, 1).validate().is_ok());
    assert!(uvc("1280x720@30", 20_000, 10).validate().is_ok());

    assert!(uvc("720p", 6000, 2).validate().is_err());
    assert!(uvc("", 499, 2).validate().is_err());
    assert!(uvc("", 20_001, 2).validate().is_err());
    assert!(uvc("", 6000, 0).validate().is_err());
    assert!(uvc("", 6000, 11).validate().is_err());
}

#[test]
fn goggles3_settings_are_backward_compatible_and_round_trip() {
    let old: InputSettings = serde_json::from_str(r#"{"mode":"dji_fpv"}"#).unwrap();
    assert_eq!(old.goggles3, Goggles3Settings::default());
    let next: InputSettings = serde_json::from_str(
        r#"{"mode":"dji_goggles3","goggles3":{"interface":"enx001122334455"}}"#,
    )
    .unwrap();
    assert_eq!(next.mode, InputMode::DjiGoggles3);
    assert_eq!(next.goggles3.interface, "enx001122334455");
    assert!(
        serde_json::to_value(&next.goggles3)
            .unwrap()
            .get("transport")
            .is_none()
    );
    assert_eq!(
        serde_json::from_str::<InputSettings>(&serde_json::to_string(&next).unwrap()).unwrap(),
        next
    );
}

#[test]
fn goggles3_rejects_invalid_interface_names() {
    for interface in ["", "usb0", "enx001122334455", "wlan0", "wlan1.2"] {
        assert!(
            Goggles3Settings {
                interface: interface.into(),
            }
            .validate()
            .is_ok()
        );
    }
    for interface in [
        "lo",
        "../eth0",
        "-eth0",
        "eth0;reboot",
        "a b",
        "wlan0\n",
        "abcdefghijklmnop",
    ] {
        assert!(
            Goggles3Settings {
                interface: interface.into(),
            }
            .validate()
            .is_err(),
            "{interface}"
        );
    }
}

#[test]
fn removed_wifi_input_migrates_to_usb_autodetection() {
    let old: Goggles3Settings =
        serde_json::from_str(r#"{"transport":"wifi","interface":"wlan1"}"#).unwrap();
    assert_eq!(old, Goggles3Settings::default());
    let usb: Goggles3Settings =
        serde_json::from_str(r#"{"transport":"usb","interface":"usb0"}"#).unwrap();
    assert_eq!(usb.interface, "usb0");
}
