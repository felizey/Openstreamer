use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Tree(std::path::PathBuf);
impl Tree {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "openstreamer-hdmi-hotplug-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn screen(&self, port: u32, connected: bool, edid: &[u8]) {
        let path = self.0.join(format!("card1-HDMI-A-{port}"));
        fs::create_dir_all(&path).unwrap();
        fs::write(
            path.join("status"),
            if connected {
                "connected\n"
            } else {
                "disconnected\n"
            },
        )
        .unwrap();
        fs::write(
            path.join("modes"),
            if connected { "1920x1080\n" } else { "" },
        )
        .unwrap();
        fs::write(path.join("edid"), edid).unwrap();
    }
    fn scan(&self, ports: HdmiPorts) -> Topology {
        Topology::read(&self.0, ports).unwrap()
    }
}
impl Drop for Tree {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn discovers_a_second_screen_and_recovers_after_all_screens_were_unplugged() {
    let tree = Tree::new();
    tree.screen(1, true, b"screen0");
    tree.screen(2, false, b"");
    let first = tree.scan(HdmiPorts::Both);
    tree.screen(2, true, b"screen1");
    let both = tree.scan(HdmiPorts::Both);
    assert_ne!(first, both);
    assert_eq!(both.0.iter().filter(|port| port.connected).count(), 2);
    tree.screen(1, false, b"");
    tree.screen(2, false, b"");
    assert!(!tree.scan(HdmiPorts::Both).has_screen());
    tree.screen(2, true, b"screen1");
    let restored = tree.scan(HdmiPorts::Both);
    assert!(restored.has_screen());
    assert_ne!(restored, first);
}

#[test]
fn ignores_unselected_ports_and_detects_replaced_monitors_or_ready_edid() {
    let tree = Tree::new();
    tree.screen(1, true, b"first");
    tree.screen(2, false, b"");
    let selected = tree.scan(HdmiPorts::Hdmi0);
    tree.screen(2, true, b"other");
    assert_eq!(selected, tree.scan(HdmiPorts::Hdmi0));
    assert_eq!(tree.scan(HdmiPorts::Hdmi1).0.len(), 1);
    tree.screen(1, true, b"replacement");
    assert_ne!(selected, tree.scan(HdmiPorts::Hdmi0));
}
