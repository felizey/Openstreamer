use crate::settings::HdmiPorts;
use std::{fs, path::Path};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Connector {
    name: String,
    connected: bool,
    modes: Vec<u8>,
    edid: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Topology(Vec<Connector>);

impl Topology {
    pub fn scan(ports: HdmiPorts) -> Result<Self, String> {
        Self::read(Path::new("/sys/class/drm"), ports)
    }

    fn read(root: &Path, ports: HdmiPorts) -> Result<Self, String> {
        let mut connectors = Vec::new();
        for entry in fs::read_dir(root).map_err(|error| format!("HDMI discovery: {error}"))? {
            let entry = entry.map_err(|error| error.to_string())?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some((_, index)) = name.rsplit_once("-HDMI-A-") else {
                continue;
            };
            if !index
                .parse()
                .ok()
                .is_some_and(|index| ports.includes(index))
            {
                continue;
            }
            let path = entry.path();
            let status = fs::read_to_string(path.join("status"))
                .map_err(|error| format!("{name}: {error}"))?;
            let connected = status.trim() == "connected";
            connectors.push(Connector {
                name,
                connected,
                modes: if connected {
                    fs::read(path.join("modes")).unwrap_or_default()
                } else {
                    Vec::new()
                },
                edid: if connected {
                    fs::read(path.join("edid")).unwrap_or_default()
                } else {
                    Vec::new()
                },
            });
        }
        connectors.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(Self(connectors))
    }

    pub fn has_screen(&self) -> bool {
        self.0.iter().any(|connector| connector.connected)
    }

    pub fn connected_count(&self) -> usize {
        self.0
            .iter()
            .filter(|connector| connector.connected)
            .count()
    }
}

#[cfg(test)]
#[path = "hotplug.test.rs"]
mod tests;
