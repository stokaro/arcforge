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
fn multiple_file_arguments_fail_before_desktop_initialization() {
    let output = Command::new(env!("CARGO_BIN_EXE_arcforge"))
        .args(["first.zip", "second.zip"])
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .expect("launch desktop binary with invalid arguments");
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("one archive path"));
}

#[test]
fn help_explains_archive_opening_without_a_display() {
    let output = Command::new(env!("CARGO_BIN_EXE_arcforge"))
        .arg("--help")
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .expect("launch desktop binary in help mode");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[ARCHIVE]"));
    assert!(stdout.contains("Double-click a file to open it in the default app"));
    assert!(stdout.contains("staged deletion and Save As"));
    assert!(output.stderr.is_empty());
}
