use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum InputMode {
    #[default]
    // G2, Integra, G3, N3: DUML
    DjiFpv,
    // USB cameras, analog receivers
    Uvc,
    /// DJI Goggles V1 and V2: video only
    DjiFpvLegacy,
    /// DJI Goggles 3 over USB Ethernet.
    DjiGoggles3,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct InputSettings {
    pub mode: InputMode,
    pub uvc: UvcSettings,
    pub goggles3: Goggles3Settings,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Goggles3Settings {
    /// Empty selects the DJI USB network interface automatically.
    pub interface: String,
}

impl<'de> Deserialize<'de> for Goggles3Settings {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize, Default)]
        #[serde(default)]
        struct Saved {
            interface: String,
            transport: Option<String>,
        }
        let saved = Saved::deserialize(deserializer)?;
        // The removed Wi-Fi mode may have saved wlan0/wlan1. Return to USB
        // autodetection instead of trying to claim that wireless interface.
        Ok(Self {
            interface: if saved.transport.as_deref() == Some("wifi") {
                String::new()
            } else {
                saved.interface
            },
        })
    }
}

impl Goggles3Settings {
    pub fn validate(&self) -> Result<(), String> {
        if self.interface.len() > 15
            || (!self.interface.is_empty()
                && (!self
                    .interface
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"_-.:".contains(&c))
                    || self.interface.starts_with(['-', '.'])
                    || self.interface == "lo"))
        {
            return Err(
                "Goggles 3 interface must be a Linux network interface name (up to 15 characters)"
                    .into(),
            );
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct UvcSettings {
    /// /dev/v4l/by-id path, which survives reboots unlike /dev/videoN.
    /// Empty: the first USB video device found
    pub device: String,
    /// `WxH@fps`, empty for the best the device offers up to 1080p30
    pub mode: String,
    /// The device's video gets encoded, unlike the goggles' H.264
    pub bitrate_kbps: u32,
    pub keyframe_secs: u32,
}

impl Default for UvcSettings {
    fn default() -> Self {
        Self {
            device: String::new(),
            mode: String::new(),
            bitrate_kbps: 6000,
            keyframe_secs: 1,
        }
    }
}

impl UvcSettings {
    pub fn validate(&self) -> Result<(), String> {
        if !(500..=20_000).contains(&self.bitrate_kbps) {
            return Err("USB camera bitrate must be 500-20000 kbit/s".into());
        }
        if !(1..=10).contains(&self.keyframe_secs) {
            return Err("USB camera keyframe interval must be 1-10 seconds".into());
        }
        if !self.mode.is_empty() && parse_uvc_mode(&self.mode).is_none() {
            return Err(format!(
                "USB camera mode \"{}\" isn't like 1280x720@30",
                self.mode
            ));
        }
        Ok(())
    }
}

/// `1280x720@30` -> (1280, 720, 30)
pub fn parse_uvc_mode(mode: &str) -> Option<(u32, u32, u32)> {
    let (size, fps) = mode.split_once('@')?;
    let (w, h) = size.split_once('x')?;
    Some((w.parse().ok()?, h.parse().ok()?, fps.parse().ok()?))
}

#[cfg(test)]
#[path = "input.test.rs"]
mod tests;
