//! Portable development commands for the ArcForge workspace.
//!
//! The workspace/xtask split follows ArtCraft; see THIRD_PARTY_NOTICES.md.

mod cargo_env;
mod layers;
mod notices;
mod package;
mod skills;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const HELP: &str = "ArcForge workspace tooling

Usage: cargo xtask <command> [--target TARGET] [--release]

  check    Check formatting, skills, dependency layers, notices, Clippy, tests and builds
  layers   Check workspace layering and headless core/CLI dependencies
  notices  Verify license notice snapshot hashes match the committed inputs
  skills   Verify scoped agent skills and generated Claude entry points
  skills-sync  Update generated Claude entry points from canonical skills
  build    Build the workspace with the committed dependency lockfile
  test     Run workspace tests (requires a runnable target)
  package  Build unsigned release development bundles and SHA-256 checksums

package always uses the release profile. Cross-compilation requires an installed
target and its toolchain; test requires a native target or a configured runner.
Development bundles are portable archives, not signed production installers.";

#[derive(Debug, Default)]
struct Options {
    target: Option<String>,
    release: bool,
}

fn main() -> ExitCode {
    match execute() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xtask: {error}");
            ExitCode::FAILURE
        }
    }
}

fn execute() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let Some(command) = args.next() else {
        println!("{HELP}");
        return Ok(());
    };
    if command == "--help" || command == "-h" || command == "help" {
        println!("{HELP}");
        return Ok(());
    }
    let options = parse_options(args)?;
    let root = workspace_root()?;
    match command.to_str() {
        Some("check") => {
            run(cargo(&root).args(["fmt", "--all", "--", "--check"]))?;
            skills::check(&root)?;
            layers::run(&root)?;
            notices::run(&root)?;
            let mut clippy = cargo(&root);
            clippy.args([
                "clippy",
                "--locked",
                "--workspace",
                "--all-targets",
                "--all-features",
            ]);
            add_options(&mut clippy, &options);
            clippy.args(["--", "-D", "warnings"]);
            run(&mut clippy)?;
            cargo_operation(&root, "test", &options)?;
            cargo_operation(&root, "build", &options)
        }
        Some("layers") => layers::run(&root),
        Some("notices") => notices::run(&root),
        Some("skills") => skills::check(&root),
        Some("skills-sync") => skills::sync(&root),
        Some(operation @ ("build" | "test")) => cargo_operation(&root, operation, &options),
        Some("package") => package::run(&root, &options),
        _ => Err(format!("unknown command {:?}; use cargo xtask --help", command).into()),
    }
}

fn parse_options(args: impl Iterator<Item = OsString>) -> Result<Options> {
    let mut args = args;
    let mut options = Options::default();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--release") => options.release = true,
            Some("--target") => {
                if options.target.is_some() {
                    return Err("--target may be supplied only once".into());
                }
                let target = args
                    .next()
                    .ok_or("--target requires a Rust target triple")?;
                let target = target
                    .into_string()
                    .map_err(|_| "target must be valid UTF-8")?;
                if target.is_empty()
                    || !target
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
                {
                    return Err("target must be a Rust target triple".into());
                }
                options.target = Some(target);
            }
            _ => return Err(format!("unrecognized argument {arg:?}").into()),
        }
    }
    Ok(options)
}

fn workspace_root() -> Result<PathBuf> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "xtask must be a direct child of the workspace root".into())
}

fn cargo(root: &Path) -> Command {
    let mut command = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    command.current_dir(root);
    cargo_env::without_cargo_run_vars(&mut command);
    command
}

fn add_options(command: &mut Command, options: &Options) {
    if let Some(target) = &options.target {
        command.args(["--target", target]);
    }
    if options.release {
        command.arg("--release");
    }
}

fn cargo_operation(root: &Path, operation: &str, options: &Options) -> Result<()> {
    let mut command = cargo(root);
    command.args([operation, "--locked", "--workspace", "--all-features"]);
    add_options(&mut command, options);
    run(&mut command)
}

fn run(command: &mut Command) -> Result<()> {
    eprintln!("$ {command:?}");
    let status = command.status()?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("command failed with {status}: {command:?}").into())
    }
}

fn metadata(root: &Path, resolve_dependencies: bool) -> Result<serde_json::Value> {
    let mut command = cargo(root);
    command.args([
        "metadata",
        "--locked",
        "--format-version",
        "1",
        "--all-features",
    ]);
    if !resolve_dependencies {
        command.arg("--no-deps");
    }
    // Do not filter the platform: rules cover target-specific dependencies too.
    let output = command.output()?;
    if !output.status.success() {
        return Err(format!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(serde_json::from_slice(&output.stdout)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_incomplete_or_ambiguous_targets() {
        for args in [
            vec!["--target"],
            vec!["--target", "../elsewhere"],
            vec!["--target", "x86_64-unknown-linux-gnu", "--target", "other"],
        ] {
            assert!(parse_options(args.into_iter().map(OsString::from)).is_err());
        }
    }
}
