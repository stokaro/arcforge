use std::process::{Command, Output};

fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_arcforge-cli"))
        .args(args)
        .output()
        .expect("launch the built CLI")
}

#[test]
fn version_does_not_require_a_display() {
    let output = Command::new(env!("CARGO_BIN_EXE_arcforge-cli"))
        .arg("--version")
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .expect("launch CLI without display");
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains(env!("CARGO_PKG_VERSION")));
    assert!(output.stderr.is_empty());
}

#[test]
fn doctor_reports_zip_extraction_without_archive_modification() {
    let output = cli(&["doctor", "--json"]);
    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid JSON");
    assert_eq!(report["kind"], "doctor");
    assert_eq!(report["data"]["providers"][0]["write"], false);
    assert_eq!(report["data"]["providers"][0]["encrypted_content"], false);
    assert_eq!(report["data"]["ready_for_archive_operations"], true);
    assert_eq!(report["data"]["archive_provider_count"], 1);
    assert_eq!(
        report["data"]["providers"][0]["operations"],
        serde_json::json!(["list", "info", "preview", "extract", "delete"])
    );
    assert_eq!(
        report["data"]["providers"][0]["extraction_compression"],
        serde_json::json!(["stored", "deflate"])
    );
    assert_eq!(
        report["data"]["supported_archive_formats"],
        serde_json::json!(["zip", "zip64"])
    );
}

#[test]
fn discovery_reports_extraction_as_implemented() {
    let output = cli(&["commands", "--json"]);
    assert!(output.status.success());
    let catalog: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid JSON");
    let extract = catalog["data"]
        .as_array()
        .expect("command array")
        .iter()
        .find(|command| command["name"] == "extract")
        .expect("extract listed");
    assert_eq!(extract["availability"], "implemented");
}

#[test]
fn planned_operation_fails_without_success_output() {
    let output = cli(&["create", "does-not-exist.zip"]);
    assert_eq!(output.status.code(), Some(3));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not implemented"));
}

#[test]
fn invalid_usage_and_unknown_commands_have_distinct_failure_code() {
    for args in [
        vec!["unknown"],
        vec!["doctor", "--pretend"],
        vec!["version", "extra"],
        vec!["help", "extra"],
    ] {
        let output = cli(&args);
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }
}

#[cfg(unix)]
#[test]
fn command_failure_survives_a_closed_stderr_reader() {
    use std::os::fd::OwnedFd;
    use std::os::unix::net::UnixStream;
    use std::process::Stdio;

    for (args, expected_code) in [
        (vec!["unknown"], 2),
        (vec!["create", "does-not-exist.zip"], 3),
    ] {
        // Give stderr a real OS stream whose consumer has already closed. The
        // child receives EPIPE when reporting its error, without a timing race.
        let (reader, writer) = UnixStream::pair().expect("create stderr stream");
        drop(reader);
        let writer: OwnedFd = writer.into();
        let output = Command::new(env!("CARGO_BIN_EXE_arcforge-cli"))
            .args(args)
            .stderr(Stdio::from(writer))
            .output()
            .expect("launch CLI with a closed stderr consumer");
        assert_eq!(output.status.code(), Some(expected_code));
        assert!(output.stdout.is_empty());
    }
}
