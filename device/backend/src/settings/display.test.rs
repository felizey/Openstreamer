use super::*;

#[test]
fn existing_settings_enable_both_ports_without_losing_display_options() {
    let cfg: DisplaySettings =
        serde_json::from_str(r#"{"hdmi_output":true,"mode":"1280x720@60.00","mirror":true}"#)
            .unwrap();
    assert_eq!(cfg.ports, HdmiPorts::Both);
    assert_eq!(cfg.mode, "1280x720@60.00");
    assert!(cfg.mirror);
    assert!(!cfg.rotate_180);
}

#[test]
fn selecting_the_second_port_does_not_depend_on_the_first_being_connected() {
    let cfg: DisplaySettings = serde_json::from_str(r#"{"ports":"hdmi1"}"#).unwrap();
    assert!(!cfg.ports.includes(1));
    assert!(cfg.ports.includes(2));
    assert!(!cfg.ports.includes(3));
    assert!(HdmiPorts::Hdmi0.includes(1));
    assert!(!HdmiPorts::Hdmi0.includes(2));
    assert!(serde_json::from_str::<DisplaySettings>(r#"{"ports":"unknown"}"#).is_err());
}
