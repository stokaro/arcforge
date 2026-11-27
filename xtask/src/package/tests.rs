use super::*;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "arcforge-package-test-{}-{timestamp}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("create isolated test directory");
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn bundles_preserve_names_bytes_permissions_and_repeatable_archives() -> Result<()> {
    let directory = TestDirectory::new();
    let staging = directory.0.join("stage");
    fs::create_dir_all(staging.join("folder"))?;
    fs::write(staging.join("folder/žluťoučký.txt"), b"archive payload")?;
    fs::write(staging.join("arcforge-cli"), b"executable payload")?;
    let first = directory.0.join("first.zip");
    let second = directory.0.join("second.zip");
    write_zip(&staging, "bundle", &first)?;
    write_zip(&staging, "bundle", &second)?;
    assert_eq!(hash_file(&first)?, hash_file(&second)?);
    let mut zip = zip::ZipArchive::new(File::open(first)?)?;
    let mut content = String::new();
    zip.by_name("bundle/folder/žluťoučký.txt")?
        .read_to_string(&mut content)?;
    assert_eq!(content, "archive payload");
    assert_eq!(
        zip.by_name("bundle/arcforge-cli")?
            .unix_mode()
            .map(|mode| mode & 0o777),
        Some(0o755)
    );

    let first = directory.0.join("first.tar.gz");
    let second = directory.0.join("second.tar.gz");
    write_tar(&staging, "bundle", &first)?;
    write_tar(&staging, "bundle", &second)?;
    assert_eq!(hash_file(&first)?, hash_file(&second)?);
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(File::open(first)?));
    let mut contents = BTreeMap::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_string_lossy().into_owned();
        let mut bytes = String::new();
        entry.read_to_string(&mut bytes)?;
        contents.insert(path, (bytes, entry.header().mode()?));
    }
    assert_eq!(contents["bundle/folder/žluťoučký.txt"].0, "archive payload");
    assert_eq!(contents["bundle/arcforge-cli"].1, 0o755);
    Ok(())
}

#[test]
fn executable_machine_check_rejects_mislabeled_target_artifacts() -> Result<()> {
    let directory = TestDirectory::new();
    let path = directory.0.join("binary");
    for target in TARGETS {
        let arm = target.starts_with("aarch64");
        let mut bytes = [0_u8; 128];
        if target.contains("linux") {
            bytes[..6].copy_from_slice(b"\x7fELF\x02\x01");
            bytes[18..20].copy_from_slice(&(if arm { 183_u16 } else { 62_u16 }).to_le_bytes());
        } else if target.contains("apple") {
            bytes[..4].copy_from_slice(&[0xcf, 0xfa, 0xed, 0xfe]);
            bytes[4..8].copy_from_slice(
                &(if arm {
                    0x0100_000c_u32
                } else {
                    0x0100_0007_u32
                })
                .to_le_bytes(),
            );
        } else {
            bytes[..2].copy_from_slice(b"MZ");
            bytes[60..64].copy_from_slice(&64_u32.to_le_bytes());
            bytes[64..68].copy_from_slice(b"PE\0\0");
            bytes[68..70]
                .copy_from_slice(&(if arm { 0xaa64_u16 } else { 0x8664_u16 }).to_le_bytes());
        }
        fs::write(&path, bytes)?;
        verify_binary(&path, target)?;
        let wrong = if arm {
            target.replacen("aarch64", "x86_64", 1)
        } else {
            target.replacen("x86_64", "aarch64", 1)
        };
        assert!(verify_binary(&path, &wrong).is_err());
    }
    fs::write(&path, b"truncated")?;
    assert!(verify_binary(&path, TARGETS[0]).is_err());
    Ok(())
}
