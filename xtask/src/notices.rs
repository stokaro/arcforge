//! Offline verification that committed notices match their source inputs.

use std::fs;
use std::path::{Component, Path};

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::Result;

const SNAPSHOT: &str = "licenses/dependencies.lock.json";

pub fn run(root: &Path) -> Result<()> {
    verify(root).map_err(|error| {
        format!(
            "license notice snapshot check failed: {error}\nRegenerate notices and {SNAPSHOT} using the instructions in docs/development.md."
        )
    })?;
    println!("License notice snapshot matches all recorded inputs and outputs.");
    Ok(())
}

fn verify(root: &Path) -> Result<()> {
    let snapshot: Value = serde_json::from_slice(
        &fs::read(root.join(SNAPSHOT)).map_err(|error| format!("{SNAPSHOT}: {error}"))?,
    )?;
    if snapshot["schema"].as_u64() != Some(1) {
        return Err(format!("{SNAPSHOT}: expected schema 1").into());
    }
    if snapshot["generator"]
        .as_str()
        .is_none_or(|value| value.trim().is_empty())
    {
        return Err(format!("{SNAPSHOT}: generator must be a nonempty string").into());
    }
    let files = snapshot["files"]
        .as_object()
        .ok_or("snapshot files must be an object")?;
    if files.is_empty() || !files.contains_key("Cargo.lock") {
        return Err("snapshot files must include Cargo.lock and must not be empty".into());
    }
    let canonical_root = root.canonicalize()?;
    for (name, expected) in files {
        if !is_relative_file(name) {
            return Err(
                format!("snapshot path must be a normalized relative file path: {name:?}").into(),
            );
        }
        let expected = expected
            .as_str()
            .ok_or_else(|| format!("{name}: expected a SHA-256 string"))?;
        if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(format!("{name}: expected 64 hexadecimal SHA-256 digits").into());
        }
        let path = root.join(name);
        let resolved = path
            .canonicalize()
            .map_err(|error| format!("{name}: {error}"))?;
        if !resolved.starts_with(&canonical_root) || !resolved.is_file() {
            return Err(
                format!("{name}: snapshot input must be a file inside the workspace").into(),
            );
        }
        let actual = format!("{:x}", Sha256::digest(fs::read(&resolved)?));
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(
                format!("{name}: SHA-256 mismatch; expected {expected}, found {actual}").into(),
            );
        }
    }
    Ok(())
}

fn is_relative_file(name: &str) -> bool {
    // Manifest paths use forward slashes on every OS. Reject Windows drive/UNC
    // syntax even when the verifier itself is running on Unix.
    !name.is_empty()
        && !name.contains(['\\', ':'])
        && name
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
        && Path::new(name)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Result<Self> {
            let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
            let root = std::env::temp_dir().join(format!(
                "arcforge-notices-test-{}-{timestamp}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&root)?;
            fs::create_dir(root.join("licenses"))?;
            fs::write(root.join("Cargo.lock"), b"version = 4\n")?;
            fs::write(
                root.join("licenses/notices.txt"),
                b"fixture license notice\n",
            )?;
            let snapshot = serde_json::json!({
                "schema": 1,
                "generator": "cargo-about 0.9.3",
                "files": {
                    "Cargo.lock": format!("{:x}", Sha256::digest(b"version = 4\n")),
                    "licenses/notices.txt": format!("{:x}", Sha256::digest(b"fixture license notice\n"))
                }
            });
            fs::write(root.join(SNAPSHOT), serde_json::to_vec(&snapshot)?)?;
            Ok(Self(root))
        }

        fn update(&self, edit: impl FnOnce(&mut Value)) -> Result<()> {
            let mut snapshot = serde_json::from_slice(&fs::read(self.0.join(SNAPSHOT))?)?;
            edit(&mut snapshot);
            fs::write(self.0.join(SNAPSHOT), serde_json::to_vec(&snapshot)?)?;
            Ok(())
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn accepts_current_snapshot_and_rejects_changed_lockfile() -> Result<()> {
        let fixture = Fixture::new()?;
        verify(&fixture.0)?;
        fs::write(fixture.0.join("Cargo.lock"), b"changed dependency graph")?;
        let error = verify(&fixture.0)
            .expect_err("changed lockfile must fail")
            .to_string();
        assert!(error.contains("Cargo.lock") && error.contains("mismatch"));
        Ok(())
    }

    #[test]
    fn rejects_incorrect_digests_and_missing_notices() -> Result<()> {
        let fixture = Fixture::new()?;
        fixture
            .update(|snapshot| snapshot["files"]["Cargo.lock"] = Value::String("0".repeat(64)))?;
        assert!(verify(&fixture.0).is_err());
        let fixture = Fixture::new()?;
        fs::remove_file(fixture.0.join("licenses/notices.txt"))?;
        assert!(
            verify(&fixture.0)
                .expect_err("missing notice must fail")
                .to_string()
                .contains("licenses/notices.txt")
        );
        Ok(())
    }

    #[test]
    fn requires_supported_schema_and_lockfile_entry() -> Result<()> {
        let fixture = Fixture::new()?;
        fixture.update(|snapshot| snapshot["schema"] = Value::from(2))?;
        assert!(verify(&fixture.0).is_err());
        let fixture = Fixture::new()?;
        fixture.update(|snapshot| snapshot["files"] = serde_json::json!({}))?;
        assert!(verify(&fixture.0).is_err());
        let fixture = Fixture::new()?;
        fixture.update(|snapshot| {
            snapshot["files"]
                .as_object_mut()
                .expect("object fixture")
                .remove("Cargo.lock");
        })?;
        assert!(verify(&fixture.0).is_err());
        Ok(())
    }

    #[test]
    fn rejects_paths_outside_portable_relative_manifest_syntax() -> Result<()> {
        for path in [
            "../outside",
            "/absolute",
            "C:/absolute",
            "C:\\absolute",
            "\\\\server\\file",
            "dir/../outside",
            "./Cargo.lock",
            "dir//file",
        ] {
            let fixture = Fixture::new()?;
            fixture.update(|snapshot| snapshot["files"][path] = Value::String("0".repeat(64)))?;
            assert!(
                verify(&fixture.0)
                    .expect_err("unsafe path must fail")
                    .to_string()
                    .contains("normalized relative"),
                "{path}"
            );
        }
        Ok(())
    }
}
