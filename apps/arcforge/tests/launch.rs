use std::process::Command;

#[test]
fn version_exits_before_desktop_initialization() {
    let output = Command::new(env!("CARGO_BIN_EXE_arcforge"))
        .arg("--version")
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .expect("launch desktop binary in version mode");
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains(env!("CARGO_PKG_VERSION")));
    assert!(output.stderr.is_empty());
}

#[test]
fn unsupported_file_arguments_fail_before_desktop_initialization() {
    let output = Command::new(env!("CARGO_BIN_EXE_arcforge"))
        .arg("does-not-exist.zip")
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .expect("launch desktop binary with unsupported argument");
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not implemented"));
}
