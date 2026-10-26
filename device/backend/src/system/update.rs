use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::Serialize;

use crate::net::network::run;

/// The private half signs releases and never comes near the repository
const PUBLIC_KEY: &str = include_str!("../../../packaging/update-key.pub");
/// One file: magic, the 64-byte Ed25519 signature, then the unchanged .deb.
const BUNDLE_MAGIC: &[u8] = b"OPENSTREAMER-UPDATE/1\n";
const PACKAGE: &str = "openstreamer";
const DIR: &str = "/var/lib/openstreamer/update";
/// Runs the install outside this service, which the package restarts
/// partway through
const UNIT: &str = "openstreamer-update";

const STAGED: &str = "staged.deb";
const CURRENT: &str = "current.deb";
const PREVIOUS: &str = "previous.deb";
const LOG: &str = "log";
const RESULT: &str = "result";
const ATTEMPT: &str = "attempt";
/// In `result` when the new version never answered: the old one is back,
/// or there was none to go back to
const REVERTED: &str = "reverted";
const UNANSWERED: &str = "unanswered";
const HEALTH_TRIES: u32 = 30;

/// Where the web interface listens, for asking the new version if it's up
pub static PORT: std::sync::OnceLock<u16> = std::sync::OnceLock::new();

#[derive(Serialize)]
pub struct Status {
    current: &'static str,
    /// Uploaded and verified, waiting to be installed
    staged: Option<String>,
    /// What "roll back" would install
    previous: Option<String>,
    installing: bool,
    failure: Option<Failure>,
}

#[derive(Serialize)]
struct Failure {
    version: String,
    /// It installed but never answered, and the version before is back on
    reverted: bool,
    log: String,
}

async fn architecture() -> Result<String, String> {
    run("dpkg", &["--print-architecture"])
        .await
        .map(|arch| arch.trim().to_string())
        .map_err(|e| format!("Couldn't tell what kind of device this is: {e}"))
}

pub async fn status() -> Status {
    let installing = run("systemctl", &["is-active", UNIT])
        .await
        .is_ok_and(|state| matches!(state.trim(), "active" | "activating"));
    let result = fs::read_to_string(file(RESULT)).unwrap_or_default();
    let failed = !matches!(result.trim(), "" | "0");
    Status {
        current: env!("CARGO_PKG_VERSION"),
        staged: version_of(&file(STAGED)).await,
        previous: version_of(&file(PREVIOUS)).await,
        installing,
        failure: (failed && !installing).then(|| Failure {
            version: fs::read_to_string(file(ATTEMPT)).unwrap_or_default(),
            reverted: result.trim() == REVERTED,
            log: fs::read_to_string(file(LOG)).unwrap_or_default(),
        }),
    }
}

/// Keep an uploaded package for installing, once it's proven to be ours
pub async fn stage(upload: &[u8], signature: Option<&str>) -> Result<(), String> {
    let deb = verified_upload(PUBLIC_KEY, upload, signature)?;
    fs::create_dir_all(DIR).map_err(|e| format!("Couldn't create {DIR}: {e}"))?;
    let upload = file("upload.deb");
    fs::write(&upload, deb).map_err(|e| format!("Couldn't store the package: {e}"))?;
    let checked = check_package(&upload).await;
    if checked.is_err() {
        let _ = fs::remove_file(&upload);
    }
    checked?;
    fs::rename(&upload, file(STAGED)).map_err(|e| format!("Couldn't store the package: {e}"))?;
    let _ = fs::remove_file(file(RESULT));
    Ok(())
}

pub fn discard() {
    let _ = fs::remove_file(file(STAGED));
    let _ = fs::remove_file(file(RESULT));
}

pub async fn install() -> Result<(), String> {
    let version = version_of(&file(STAGED))
        .await
        .ok_or("Upload a package first")?;
    // What ran before stays as the one to roll back to
    start(
        &version,
        STAGED,
        &format!("[ ! -f {CURRENT} ] || mv -f {CURRENT} {PREVIOUS}; mv -f {STAGED} {CURRENT}"),
    )
    .await
}

pub async fn rollback() -> Result<(), String> {
    let version = version_of(&file(PREVIOUS))
        .await
        .ok_or("There's no earlier version to go back to")?;
    // The two swap places, so rolling back twice returns to the newer one
    start(
        &version,
        PREVIOUS,
        &format!(
            "mv -f {PREVIOUS} swap.deb; [ ! -f {CURRENT} ] || mv -f {CURRENT} {PREVIOUS}; mv -f swap.deb {CURRENT}"
        ),
    )
    .await
}

/// Install `deb`, then give the version it brings a minute to answer. One
/// that doesn't would leave no web interface to fix it from, so the package
/// that was running goes back on. `keep` files the packages once it's
/// settled. The SD card image ships without apt's package lists, so a
/// release that needs a package not yet installed fetches them and tries
/// once more
fn script(dir: &str, deb: &str, version: &str, port: u16, keep: &str) -> String {
    format!(
        r#"apt-get install -y --allow-downgrades {dir}/{deb} >{LOG} 2>&1 || {{
    apt-get update >>{LOG} 2>&1
    apt-get install -y --allow-downgrades {dir}/{deb} >>{LOG} 2>&1
}}
code=$?
if [ $code != 0 ]; then
    echo $code >{RESULT}
    exit 0
fi
tries=0
until curl --fail --silent --max-time 2 http://localhost:{port}/health | grep -q '"version":"{version}"'; do
    tries=$((tries + 1))
    if [ $tries -ge {HEALTH_TRIES} ]; then
        echo "Version {version} didn't start" >>{LOG}
        if [ -f {CURRENT} ] && apt-get install -y --allow-downgrades {dir}/{CURRENT} >>{LOG} 2>&1; then
            echo {REVERTED} >{RESULT}
        else
            echo {UNANSWERED} >{RESULT}
        fi
        exit 0
    fi
    sleep 2
done
{keep}
echo 0 >{RESULT}
"#
    )
}

async fn start(version: &str, deb: &str, keep: &str) -> Result<(), String> {
    if status().await.installing {
        return Err("An update is already being installed".into());
    }
    let _ = fs::remove_file(file(RESULT));
    fs::write(file(ATTEMPT), version).map_err(|e| format!("Couldn't start the update: {e}"))?;
    let port = PORT.get().copied().unwrap_or(80);
    let script = script(DIR, deb, version, port, keep);
    run(
        "systemd-run",
        &[
            "--unit",
            UNIT,
            "--collect",
            "--quiet",
            "--setenv=DEBIAN_FRONTEND=noninteractive",
            &format!("--working-directory={DIR}"),
            "sh",
            "-c",
            &script,
        ],
    )
    .await
    .map(|_| ())
    .map_err(|e| format!("Couldn't start the update: {e}"))
}

fn verified_upload<'a>(
    public_key: &str,
    upload: &'a [u8],
    signature: Option<&str>,
) -> Result<&'a [u8], String> {
    if let Some(signature) = signature {
        verify(public_key, upload, signature)?;
        return Ok(upload);
    }
    let payload = upload
        .strip_prefix(BUNDLE_MAGIC)
        .ok_or("Choose an .osupd update file, or the .deb package and its .sig together")?;
    if payload.len() <= 64 {
        return Err("The update file is incomplete".into());
    }
    let (signature, deb) = payload.split_at(64);
    let signature = base64::engine::general_purpose::STANDARD.encode(signature);
    verify(public_key, deb, &signature)?;
    Ok(deb)
}

fn verify(public_key: &str, deb: &[u8], signature: &str) -> Result<(), String> {
    let decode = |text: &str| base64::engine::general_purpose::STANDARD.decode(text.trim());
    let key = decode(public_key)
        .ok()
        .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
        .and_then(|bytes| VerifyingKey::from_bytes(&bytes).ok())
        .expect("update key is valid");
    let signature = decode(signature)
        .ok()
        .and_then(|bytes| Signature::from_slice(&bytes).ok())
        .ok_or("That isn't a signature file")?;
    key.verify_strict(deb, &signature)
        .map_err(|_| "The signature doesn't match this package".into())
}

/// A signed package for something else, or for another kind of device,
/// would still install
async fn check_package(deb: &Path) -> Result<(), String> {
    let field = async |name: &str| {
        run("dpkg-deb", &["-f", &deb.to_string_lossy(), name])
            .await
            .map(|value| value.trim().to_string())
            .map_err(|_| "That isn't a package file".to_string())
    };
    if field("Package").await? != PACKAGE {
        return Err("That package isn't Openstreamer".into());
    }
    let ours = architecture().await?;
    let theirs = field("Architecture").await?;
    if theirs != ours {
        return Err(format!(
            "That package is for {theirs} devices, this one is {ours}"
        ));
    }
    Ok(())
}

async fn version_of(deb: &Path) -> Option<String> {
    if !deb.exists() {
        return None;
    }
    let version = run("dpkg-deb", &["-f", &deb.to_string_lossy(), "Version"])
        .await
        .ok()?;
    Some(display_version(version.trim()).to_string())
}

/// Without the packaging revision, to compare with what About shows
fn display_version(version: &str) -> &str {
    version
        .rsplit_once('-')
        .map_or(version, |(upstream, _)| upstream)
}

fn file(name: &str) -> PathBuf {
    Path::new(DIR).join(name)
}

#[cfg(test)]
#[path = "update.test.rs"]
mod tests;
