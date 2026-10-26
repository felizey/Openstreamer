use super::*;

const MESSAGE: &[u8] = b"Openstreamer package verification test";

/// A deterministic test-only key, independent of the production release key.
fn signed_fixture() -> (String, String) {
    use ed25519_dalek::{Signer, SigningKey};
    let key = SigningKey::from_bytes(&[42; 32]);
    let encode = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);
    (
        encode(key.verifying_key().as_bytes()),
        encode(&key.sign(MESSAGE).to_bytes()),
    )
}

#[test]
fn accepts_a_valid_signature_and_its_trailing_newline() {
    let (key, signature) = signed_fixture();
    assert_eq!(verify(&key, MESSAGE, &signature), Ok(()));
    assert_eq!(verify(&key, MESSAGE, &format!("{signature}\n")), Ok(()));
}

#[test]
fn changed_package_is_refused() {
    let (key, signature) = signed_fixture();
    assert!(verify(&key, b"changed package", &signature).is_err());
}

#[test]
fn signature_from_another_key_is_refused() {
    use ed25519_dalek::SigningKey;
    let (_, signature) = signed_fixture();
    let other = SigningKey::from_bytes(&[43; 32]);
    let key = base64::engine::general_purpose::STANDARD.encode(other.verifying_key().as_bytes());
    assert!(verify(&key, MESSAGE, &signature).is_err());
}

#[test]
fn junk_signature_is_refused() {
    let (key, _) = signed_fixture();
    assert!(verify(&key, MESSAGE, "").is_err());
    assert!(verify(&key, MESSAGE, "not base64!").is_err());
    assert!(verify(&key, MESSAGE, "c2hvcnQ=").is_err());
}

fn signed_bundle() -> (String, Vec<u8>) {
    let (key, signature) = signed_fixture();
    let mut bundle = BUNDLE_MAGIC.to_vec();
    bundle.extend(
        base64::engine::general_purpose::STANDARD
            .decode(signature)
            .unwrap(),
    );
    bundle.extend(MESSAGE);
    (key, bundle)
}

#[test]
fn single_file_update_returns_the_original_verified_package() {
    let (key, bundle) = signed_bundle();
    assert_eq!(verified_upload(&key, &bundle, None).unwrap(), MESSAGE);
}

#[test]
fn tampering_with_any_part_of_a_single_file_update_is_refused() {
    let (key, bundle) = signed_bundle();
    for position in [0, BUNDLE_MAGIC.len(), bundle.len() - 1] {
        let mut damaged = bundle.clone();
        damaged[position] ^= 1;
        assert!(verified_upload(&key, &damaged, None).is_err());
    }
}

#[test]
fn incomplete_single_file_updates_are_refused_without_panicking() {
    let (key, bundle) = signed_bundle();
    for length in 0..bundle.len() {
        assert!(verified_upload(&key, &bundle[..length], None).is_err());
    }
}

#[test]
fn single_file_update_from_another_signer_is_refused() {
    use ed25519_dalek::SigningKey;
    let (_, bundle) = signed_bundle();
    let other = SigningKey::from_bytes(&[43; 32]);
    let key = base64::engine::general_purpose::STANDARD.encode(other.verifying_key().as_bytes());
    assert!(verified_upload(&key, &bundle, None).is_err());
}

#[test]
fn separate_package_and_signature_remain_compatible() {
    let (key, signature) = signed_fixture();
    assert_eq!(
        verified_upload(&key, MESSAGE, Some(&signature)).unwrap(),
        MESSAGE
    );
    assert!(verified_upload(&key, MESSAGE, None).is_err());
    let (_, mut bundle) = signed_bundle();
    bundle[BUNDLE_MAGIC.len() - 2] = b'2';
    assert!(verified_upload(&key, &bundle, None).is_err());
}

#[test]
fn packaging_revision_is_dropped_from_the_version() {
    assert_eq!(display_version("0.1.9-1"), "0.1.9");
    assert_eq!(display_version("0.2.0"), "0.2.0");
}

/// The install script in a scratch folder, with apt-get, curl and sleep
/// replaced: `apt` is the exit code of each apt-get call in turn, `health`
/// what the device answers
fn run_script(name: &str, apt: &[u8], health: &str, before: &[(&str, &str)]) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let dir =
        std::env::temp_dir().join(format!("openstreamer-update-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let bin = dir.join("bin");
    fs::create_dir_all(&bin).unwrap();
    for (file, contents) in before {
        fs::write(dir.join(file), contents).unwrap();
    }
    let codes: Vec<String> = apt.iter().map(u8::to_string).collect();
    fs::write(dir.join("apt-codes"), codes.join("\n") + "\n").unwrap();
    fs::write(dir.join("health"), health).unwrap();
    let stubs = [
        (
            "apt-get",
            "echo \"$@\" >>apt-calls\ncode=$(head -1 apt-codes); sed -i 1d apt-codes; exit ${code:-0}",
        ),
        ("curl", "cat health"),
        ("sleep", "true"),
    ];
    for (stub, body) in stubs {
        let path = bin.join(stub);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let keep =
        format!("[ ! -f {CURRENT} ] || mv -f {CURRENT} {PREVIOUS}; mv -f {STAGED} {CURRENT}");
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(script(&dir.to_string_lossy(), STAGED, "0.2.0", 80, &keep))
        .current_dir(&dir)
        .env(
            "PATH",
            format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
        )
        .status()
        .unwrap();
    assert!(status.success());
    dir
}

fn read(dir: &Path, name: &str) -> String {
    fs::read_to_string(dir.join(name))
        .unwrap_or_default()
        .trim()
        .to_string()
}

const RUNNING: &[(&str, &str)] = &[(STAGED, "new"), (CURRENT, "old")];

#[test]
fn version_that_answers_is_kept() {
    let dir = run_script("ok", &[0], r#"{"status":"ok","version":"0.2.0"}"#, RUNNING);
    assert_eq!(read(&dir, RESULT), "0");
    assert_eq!(read(&dir, CURRENT), "new");
    assert_eq!(read(&dir, PREVIOUS), "old");
    assert!(!dir.join(STAGED).exists());
    assert_eq!(read(&dir, "apt-calls").lines().count(), 1);
}

#[test]
fn version_that_never_answers_is_replaced_by_the_one_before() {
    let dir = run_script("silent", &[0, 0], "", RUNNING);
    assert_eq!(read(&dir, RESULT), REVERTED);
    assert_eq!(read(&dir, CURRENT), "old");
    assert_eq!(read(&dir, STAGED), "new");
    assert!(!dir.join(PREVIOUS).exists());
    let calls = read(&dir, "apt-calls");
    assert!(calls.lines().nth(1).unwrap().ends_with(CURRENT), "{calls}");
}

#[test]
fn old_version_still_answering_is_not_taken_for_the_new_one() {
    let dir = run_script(
        "stale",
        &[0, 0],
        r#"{"status":"ok","version":"0.1.9"}"#,
        RUNNING,
    );
    assert_eq!(read(&dir, RESULT), REVERTED);
}

#[test]
fn nothing_to_go_back_to_is_reported() {
    let dir = run_script("first", &[0], "", &[(STAGED, "new")]);
    assert_eq!(read(&dir, RESULT), UNANSWERED);
    assert_eq!(read(&dir, "apt-calls").lines().count(), 1);
}

#[test]
fn failed_install_changes_nothing() {
    let dir = run_script("failed", &[100, 0, 100], "", RUNNING);
    assert_eq!(read(&dir, RESULT), "100");
    assert_eq!(read(&dir, CURRENT), "old");
    assert_eq!(read(&dir, STAGED), "new");
    assert_eq!(read(&dir, "apt-calls").lines().count(), 3);
}

#[test]
fn missing_package_lists_are_fetched_and_the_install_retried() {
    let dir = run_script(
        "lists",
        &[100, 0, 0],
        r#"{"status":"ok","version":"0.2.0"}"#,
        RUNNING,
    );
    assert_eq!(read(&dir, RESULT), "0");
    assert_eq!(read(&dir, CURRENT), "new");
    let calls = read(&dir, "apt-calls");
    assert_eq!(calls.lines().nth(1), Some("update"), "{calls}");
    assert_eq!(calls.lines().count(), 3);
}

#[test]
fn install_script_checks_against_the_same_key() {
    let script = include_str!("../../../packaging/install.sh");
    assert!(script.contains(&format!("PUBLIC_KEY=\"{}\"", PUBLIC_KEY.trim())));
}
