use super::*;
use std::os::unix::fs::symlink;

struct Tree(PathBuf);

impl Tree {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "openstreamer-goggles3-sysfs-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        fs::create_dir_all(path.join("class/net")).unwrap();
        Self(path)
    }

    fn root(&self) -> PathBuf {
        self.0.join("class/net")
    }

    fn interface(&self, name: &str, index: u32, vendor: &str, product: &str) {
        let device = self.0.join(format!("devices/{name}/usb/1-1"));
        let function = device.join("1-1:1.0");
        fs::create_dir_all(&function).unwrap();
        fs::write(device.join("idVendor"), vendor).unwrap();
        fs::write(device.join("idProduct"), product).unwrap();
        let net = self.root().join(name);
        fs::create_dir_all(&net).unwrap();
        fs::write(net.join("ifindex"), index.to_string()).unwrap();
        symlink(function, net.join("device")).unwrap();
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn autodetection_follows_usb_ancestry_and_skips_unrelated_usb0() {
    let tree = Tree::new();
    tree.interface("usb0", 3, "0525", "a4a2");
    tree.interface("enx1234", 8, "2CA3\n", "0020\n");
    let interface = resolve_in(&tree.root(), &Goggles3Settings::default())
        .unwrap()
        .unwrap();
    assert_eq!(interface.name, "enx1234");
    assert!(interface.present());
    let cfg = Goggles3Settings {
        interface: "usb0".into(),
    };
    assert!(
        resolve_in(&tree.root(), &cfg)
            .unwrap_err()
            .contains("2ca3:0020")
    );
}

#[test]
fn reused_interface_name_or_changed_usb_identity_cannot_receive_cleanup() {
    let tree = Tree::new();
    tree.interface("usb0", 3, "2ca3", "0020");
    let interface = Interface::read(&tree.root(), "usb0").unwrap();
    fs::write(tree.root().join("usb0/ifindex"), "4").unwrap();
    assert!(!interface.present());
    fs::write(tree.root().join("usb0/ifindex"), "3").unwrap();
    let device = tree.0.join("devices/usb0/usb/1-1");
    fs::write(device.join("idProduct"), "001f").unwrap();
    assert!(!interface.present());
    fs::write(device.join("idProduct"), "0020").unwrap();
    assert!(interface.present());
    fs::remove_file(tree.root().join("usb0/device")).unwrap();
    let replacement = tree.0.join("devices/replacement");
    fs::create_dir_all(&replacement).unwrap();
    fs::write(replacement.join("idVendor"), "2ca3").unwrap();
    fs::write(replacement.join("idProduct"), "0020").unwrap();
    symlink(replacement, tree.root().join("usb0/device")).unwrap();
    assert!(!interface.present());
}

#[test]
fn no_usb_interface_waits_and_a_wireless_adapter_is_never_selected() {
    let tree = Tree::new();
    assert!(
        resolve_in(&tree.root(), &Goggles3Settings::default())
            .unwrap()
            .is_none()
    );
    tree.interface("wlan0", 7, "1234", "5678");
    assert!(
        resolve_in(&tree.root(), &Goggles3Settings::default())
            .unwrap()
            .is_none()
    );
    let cfg = Goggles3Settings {
        interface: "wlan0".into(),
    };
    assert!(
        resolve_in(&tree.root(), &cfg)
            .unwrap_err()
            .contains("2ca3:0020")
    );
}

#[test]
fn only_the_exact_usb_address_can_be_retained() {
    assert!(validate_addresses(&[]).is_ok());
    assert!(validate_addresses(&[USB_ADDRESS.into()]).is_ok());
    for addresses in [
        vec!["10.0.0.1/24".into()],
        vec!["192.168.60.1/16".into()],
        vec!["192.168.60.2/24".into()],
        vec![USB_ADDRESS.into(), "169.254.1.2/16".into()],
    ] {
        assert!(validate_addresses(&addresses).is_err());
    }
    assert_eq!(
        parse_addresses(r#"[{"addr_info":[{"local":"192.168.60.1","prefixlen":24}]}]"#).unwrap(),
        vec![USB_ADDRESS.to_string()]
    );
    for invalid in [
        r#"[{"unexpected":[]}]"#,
        r#"[{"addr_info":[{"local":"10.0.0.2"}]}]"#,
    ] {
        assert!(parse_addresses(invalid).is_err());
    }
}

#[test]
fn command_output_is_drained_with_a_bounded_retained_buffer() {
    let bytes = vec![b'a'; OUTPUT_LIMIT * 5];
    assert_eq!(drain(bytes.as_slice()).unwrap().len(), OUTPUT_LIMIT);
}

#[test]
fn transient_profile_identity_survives_restart_but_not_interface_reuse() {
    let tree = Tree::new();
    tree.interface("usb0", 3, "2ca3", "0020");
    let first = Interface::read(&tree.root(), "usb0").unwrap();
    let restarted = Interface::read(&tree.root(), "usb0").unwrap();
    assert_eq!(first.uuid(), restarted.uuid());
    assert!(marker_matches(&tree.root(), &first.marker(), "usb0"));
    assert!(!marker_matches(&tree.root(), &first.marker(), "eth0"));
    assert!(!marker_matches(
        &tree.root(),
        "another-application|3|usb0|/sys/device",
        "usb0"
    ));
    fs::write(tree.root().join("usb0/ifindex"), "4").unwrap();
    let reused = Interface::read(&tree.root(), "usb0").unwrap();
    assert_ne!(first.uuid(), reused.uuid());
    assert!(!marker_matches(&tree.root(), &first.marker(), "usb0"));
    assert!(marker_matches(&tree.root(), &reused.marker(), "usb0"));
}
