use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use crate::settings::Goggles3Settings;

const SYS_NET: &str = "/sys/class/net";
const USB_ADDRESS: &str = "192.168.60.1/24";
const PROFILE_PREFIX: &str = "openstreamer-goggles3";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(3);
const OUTPUT_LIMIT: usize = 16 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Interface {
    pub name: String,
    index: u32,
    device: PathBuf,
    root: PathBuf,
}

impl Interface {
    fn read(root: &Path, name: &str) -> Option<Self> {
        let path = root.join(name);
        let index = fs::read_to_string(path.join("ifindex"))
            .ok()?
            .trim()
            .parse()
            .ok()?;
        let device = fs::canonicalize(path.join("device")).ok()?;
        if !dji_device(&device) {
            return None;
        }
        Some(Self {
            name: name.into(),
            index,
            device,
            root: root.into(),
        })
    }

    /// A reused usb0 name must never inherit cleanup for the unplugged goggles.
    pub fn present(&self) -> bool {
        Self::read(&self.root, &self.name).as_ref() == Some(self)
    }

    fn marker(&self) -> String {
        format!(
            "{PROFILE_PREFIX}|{}|{}|{}",
            self.index,
            self.name,
            self.device.display()
        )
    }

    fn uuid(&self) -> String {
        let digest = hex::encode(Sha256::digest(self.marker().as_bytes()));
        format!(
            "{}-{}-{}-{}-{}",
            &digest[..8],
            &digest[8..12],
            &digest[12..16],
            &digest[16..20],
            &digest[20..32]
        )
    }
}

fn dji_device(device: &Path) -> bool {
    device.ancestors().any(|parent| {
        fs::read_to_string(parent.join("idVendor"))
            .is_ok_and(|id| id.trim().eq_ignore_ascii_case("2ca3"))
            && fs::read_to_string(parent.join("idProduct"))
                .is_ok_and(|id| id.trim().eq_ignore_ascii_case("0020"))
    })
}

pub(super) fn resolve(cfg: &Goggles3Settings) -> Result<Option<Interface>, String> {
    resolve_in(Path::new(SYS_NET), cfg)
}

pub(super) fn is_usb_network_interface(name: &str) -> bool {
    Interface::read(Path::new(SYS_NET), name).is_some()
}

fn resolve_in(root: &Path, cfg: &Goggles3Settings) -> Result<Option<Interface>, String> {
    cfg.validate()?;
    if !cfg.interface.is_empty() {
        let name = &cfg.interface;
        if !root.join(name).exists() {
            return Ok(None);
        }
        return Interface::read(root, name).map(Some).ok_or_else(|| {
            format!("{name} is not a DJI Goggles 3 USB network interface (2ca3:0020)")
        });
    }
    let mut names: Vec<_> = fs::read_dir(root)
        .map_err(|e| format!("Network interfaces: {e}"))?
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    names.sort();
    Ok(names
        .into_iter()
        .find_map(|name| Interface::read(root, &name)))
}

pub(super) fn load_usb_drivers_if_needed() {
    let usb = Path::new("/sys/bus/usb/devices");
    if fs::read_dir(usb).is_ok_and(|mut devices| {
        devices.any(|entry| entry.is_ok_and(|entry| dji_device(&entry.path())))
    }) {
        let _ = run("modprobe", &["--all", "rndis_host", "cdc_ether", "cdc_ncm"]);
    }
}

/// Recover volatile profiles left behind if the process was killed before Drop.
/// The profile's marker identifies its USB device, not just an interface name.
pub(super) fn recover_profiles() -> Result<(), String> {
    let profiles = run(
        "nmcli",
        &["--wait", "2", "-t", "-f", "NAME,UUID", "connection", "show"],
    )?;
    for line in profiles.lines() {
        let Some((name, uuid)) = line.rsplit_once(':') else {
            continue;
        };
        if !name.starts_with(&format!("{PROFILE_PREFIX}-")) {
            continue;
        }
        let marker = run(
            "nmcli",
            &[
                "--wait",
                "2",
                "--escape",
                "no",
                "-g",
                "connection.stable-id",
                "connection",
                "show",
                uuid,
            ],
        )?;
        if marker.trim().starts_with(&format!("{PROFILE_PREFIX}|")) {
            delete_profile(uuid, marker.trim())?;
        }
    }
    Ok(())
}

/// NetworkManager owns the address/link. Only this unsaved, non-autoconnecting
/// profile belongs to the source; management flags and other profiles stay intact.
pub(super) struct UsbNetwork {
    interface: Interface,
    uuid: String,
}

impl UsbNetwork {
    pub fn prepare(
        interface: Interface,
        mut current: impl FnMut() -> bool,
    ) -> Result<Self, String> {
        if !current() || !interface.present() {
            return Err("Goggles 3 connection changed".into());
        }
        let name = interface.name.clone();
        validate_addresses(&addresses(&name)?)?;
        let managed = run(
            "nmcli",
            &[
                "--wait",
                "2",
                "-g",
                "GENERAL.NM-MANAGED",
                "device",
                "show",
                &name,
            ],
        )?;
        if !matches!(managed.trim(), "yes" | "true") {
            return Err(format!(
                "{name} is not managed by NetworkManager; enable management for this DJI USB interface first"
            ));
        }
        let uuid = interface.uuid();
        let marker = interface.marker();
        let network = Self { interface, uuid };
        let check = |current: &mut dyn FnMut() -> bool| {
            if current() && network.interface.present() {
                Ok(())
            } else {
                Err("Goggles 3 connection changed".to_string())
            }
        };
        check(&mut current)?;
        let profiles = run(
            "nmcli",
            &["--wait", "2", "-g", "UUID", "connection", "show"],
        )?;
        if profiles.lines().any(|uuid| uuid.trim() == network.uuid) {
            let existing = run(
                "nmcli",
                &[
                    "--wait",
                    "2",
                    "--escape",
                    "no",
                    "-g",
                    "connection.stable-id",
                    "connection",
                    "show",
                    &network.uuid,
                ],
            )?;
            if existing.trim() != marker {
                return Err("DJI USB profile UUID is already used by another connection".into());
            }
        } else {
            run(
                "nmcli",
                &[
                    "--wait",
                    "2",
                    "connection",
                    "add",
                    "save",
                    "no",
                    "type",
                    "ethernet",
                    "ifname",
                    &name,
                    "con-name",
                    &format!("{PROFILE_PREFIX}-{}", network.interface.index),
                    "connection.uuid",
                    &network.uuid,
                    "connection.stable-id",
                    &marker,
                    "connection.autoconnect",
                    "no",
                    "ipv4.method",
                    "manual",
                    "ipv4.addresses",
                    USB_ADDRESS,
                    "ipv4.never-default",
                    "yes",
                    "ipv4.ignore-auto-dns",
                    "yes",
                    "ipv6.method",
                    "disabled",
                ],
            )?;
        }
        check(&mut current)?;
        // A foreign address appearing during setup also cancels activation.
        validate_addresses(&addresses(&name)?)?;
        run(
            "nmcli",
            &[
                "--wait",
                "2",
                "connection",
                "up",
                "uuid",
                &network.uuid,
                "ifname",
                &name,
            ],
        )?;
        check(&mut current)?;
        Ok(network)
    }
}

impl Drop for UsbNetwork {
    fn drop(&mut self) {
        if let Err(error) = delete_profile(&self.uuid, &self.interface.marker()) {
            eprintln!("[goggles3] USB profile cleanup: {error}");
        }
    }
}

fn marker_matches(root: &Path, marker: &str, active: &str) -> bool {
    let mut parts = marker.splitn(4, '|');
    if parts.next() != Some(PROFILE_PREFIX) {
        return false;
    }
    let Some(index) = parts.next().and_then(|index| index.parse::<u32>().ok()) else {
        return false;
    };
    let Some(name) = parts.next() else {
        return false;
    };
    let Some(device) = parts.next() else {
        return false;
    };
    let cfg = Goggles3Settings {
        interface: name.into(),
    };
    if active != name || cfg.validate().is_err() {
        return false;
    }
    Interface::read(root, name)
        .is_some_and(|interface| interface.index == index && interface.device == Path::new(device))
}

fn delete_profile(uuid: &str, expected_marker: &str) -> Result<(), String> {
    let profiles = run(
        "nmcli",
        &["--wait", "2", "-g", "UUID", "connection", "show"],
    )?;
    if !profiles.lines().any(|existing| existing.trim() == uuid) {
        return Ok(());
    }
    let marker = run(
        "nmcli",
        &[
            "--wait",
            "2",
            "--escape",
            "no",
            "-g",
            "connection.stable-id",
            "connection",
            "show",
            uuid,
        ],
    )?;
    if marker.trim() != expected_marker {
        return Err("Refusing to delete a USB profile belonging to another connection".into());
    }
    let active = run(
        "nmcli",
        &[
            "--wait",
            "2",
            "-g",
            "GENERAL.DEVICES",
            "connection",
            "show",
            uuid,
        ],
    )?;
    let active = active.trim();
    if !active.is_empty()
        && active != "--"
        && !marker_matches(Path::new(SYS_NET), expected_marker, active)
    {
        return Err("Refusing to deactivate a DJI USB profile on a different device".into());
    }
    run(
        "nmcli",
        &["--wait", "2", "connection", "delete", "uuid", uuid],
    )
    .map(|_| ())
}

fn addresses(name: &str) -> Result<Vec<String>, String> {
    let output = run("ip", &["-j", "-4", "address", "show", "dev", name])?;
    parse_addresses(&output).map_err(|e| format!("{name} addresses: {e}"))
}

fn parse_addresses(output: &str) -> Result<Vec<String>, serde_json::Error> {
    #[derive(serde::Deserialize)]
    struct Address {
        local: std::net::Ipv4Addr,
        prefixlen: u8,
    }
    #[derive(serde::Deserialize)]
    struct InterfaceAddresses {
        addr_info: Vec<Address>,
    }
    let values: Vec<InterfaceAddresses> = serde_json::from_str(output)?;
    Ok(values
        .iter()
        .flat_map(|value| &value.addr_info)
        .map(|address| format!("{}/{}", address.local, address.prefixlen))
        .collect())
}

fn validate_addresses(addresses: &[String]) -> Result<(), String> {
    if addresses.iter().any(|address| address != USB_ADDRESS) {
        return Err(
            "DJI USB network already has another IPv4 address; refusing to replace it".into(),
        );
    }
    Ok(())
}

fn drain(mut pipe: impl Read) -> io::Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut buffer = [0; 1024];
    loop {
        let length = pipe.read(&mut buffer)?;
        if length == 0 {
            return Ok(output);
        }
        let take = length.min(OUTPUT_LIMIT - output.len());
        output.extend_from_slice(&buffer[..take]);
    }
}

fn run(program: &str, args: &[&str]) -> Result<String, String> {
    let mut child = Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{program}: {e}"))?;
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let stdout = thread::spawn(move || drain(stdout));
    let stderr = thread::spawn(move || drain(stderr));
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if started.elapsed() < COMMAND_TIMEOUT => {
                thread::sleep(Duration::from_millis(20))
            }
            result => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(match result {
                    Err(error) => format!("{program}: {error}"),
                    _ => format!("{program} timed out"),
                });
            }
        }
    };
    let stdout = stdout
        .join()
        .map_err(|_| format!("{program} output reader"))?
        .map_err(|e| format!("{program}: {e}"))?;
    let stderr = stderr
        .join()
        .map_err(|_| format!("{program} error reader"))?
        .map_err(|e| format!("{program}: {e}"))?;
    if status?.success() {
        Ok(String::from_utf8_lossy(&stdout).into_owned())
    } else {
        Err(format!(
            "{program}: {}",
            String::from_utf8_lossy(&stderr).trim()
        ))
    }
}

#[cfg(test)]
#[path = "network.test.rs"]
mod tests;
