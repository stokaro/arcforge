use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::Result;

pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug)]
pub(crate) struct Target {
    pub target: String,
    pub runner: String,
    pub archive: String,
    pub cli: String,
    pub gui: String,
}

impl Target {
    pub fn stem(&self) -> String {
        format!("arcforge-{VERSION}-{}", self.target)
    }

    pub fn filename(&self) -> String {
        format!("{}.{}", self.stem(), self.archive)
    }

    pub fn evidence_filename(&self) -> String {
        format!("{}.evidence.json", self.stem())
    }

    pub fn asset_names(&self) -> [String; 3] {
        [
            self.filename(),
            format!("{}.sha256", self.filename()),
            self.evidence_filename(),
        ]
    }
}

pub(crate) struct Manifest {
    pub targets: Vec<Target>,
    pub sha256: String,
}

impl Manifest {
    pub fn load(root: &Path) -> Result<Self> {
        Self::parse(&fs::read(root.join("release/targets.json"))?)
    }

    fn parse(bytes: &[u8]) -> Result<Self> {
        let value: Value = serde_json::from_slice(bytes)?;
        exact_keys(&value, &["schema_version", "targets"])?;
        if value["schema_version"] != 1 {
            return Err("unsupported release manifest schema".into());
        }
        let rows = value["targets"]
            .as_array()
            .ok_or("missing release targets")?;
        if rows.len() != 6 {
            return Err("release manifest must contain all six primary native targets".into());
        }
        let mut names = BTreeSet::new();
        let mut targets = Vec::new();
        for row in rows {
            exact_keys(row, &["target", "runner", "archive", "cli", "gui"])?;
            let target = string(row, "target")?;
            let runner = string(row, "runner")?;
            if !target
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
                || !runner
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'.'))
                || !names.insert(target.to_owned())
            {
                return Err("invalid or duplicate release target/runner".into());
            }
            let (architecture, platform) = target.split_once('-').ok_or("invalid target")?;
            if !matches!(architecture, "x86_64" | "aarch64") {
                return Err(format!("unsupported executable architecture {architecture}").into());
            }
            let expected = match platform {
                "unknown-linux-gnu" => ("tar.gz", "arcforge-cli", "arcforge"),
                "pc-windows-msvc" => ("zip", "arcforge-cli.exe", "arcforge.exe"),
                "apple-darwin" => (
                    "zip",
                    "arcforge-cli",
                    "ArcForge.app/Contents/MacOS/arcforge",
                ),
                _ => return Err(format!("unsupported executable platform {platform}").into()),
            };
            let actual = (
                string(row, "archive")?,
                string(row, "cli")?,
                string(row, "gui")?,
            );
            if actual != expected {
                return Err(format!("invalid archive/executable layout for {target}").into());
            }
            targets.push(Target {
                target: target.to_owned(),
                runner: runner.to_owned(),
                archive: actual.0.to_owned(),
                cli: actual.1.to_owned(),
                gui: actual.2.to_owned(),
            });
        }
        Ok(Self {
            targets,
            sha256: format!("{:x}", Sha256::digest(bytes)),
        })
    }

    pub fn target(&self, target: &str) -> Result<&Target> {
        self.targets
            .iter()
            .find(|row| row.target == target)
            .ok_or_else(|| {
                format!("unsupported packaging target {target}; see release/targets.json").into()
            })
    }

    pub fn matrix(&self) -> Value {
        json!({"include": self.targets.iter().map(|row| {
            json!({"target": row.target, "runner": row.runner})
        }).collect::<Vec<_>>()})
    }
}

fn exact_keys(value: &Value, keys: &[&str]) -> Result<()> {
    let object = value.as_object().ok_or("expected JSON object")?;
    if object.len() != keys.len() || keys.iter().any(|key| !object.contains_key(*key)) {
        return Err(format!("expected exactly these manifest keys: {}", keys.join(", ")).into());
    }
    Ok(())
}

fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .filter(|text| !text.is_empty())
        .ok_or_else(|| format!("manifest {key} must be a nonempty string").into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_rejects_ambiguous_targets_and_incompatible_layouts() -> Result<()> {
        let bytes = include_bytes!("../../../release/targets.json");
        let valid = Manifest::parse(bytes)?;
        assert_eq!(valid.targets.len(), 6);
        assert_eq!(valid.matrix()["include"].as_array().unwrap().len(), 6);
        let mut value: Value = serde_json::from_slice(bytes)?;
        value["targets"][1] = value["targets"][0].clone();
        assert!(Manifest::parse(&serde_json::to_vec(&value)?).is_err());
        value = serde_json::from_slice(bytes)?;
        value["targets"][0]["gui"] = json!("../arcforge");
        assert!(Manifest::parse(&serde_json::to_vec(&value)?).is_err());
        value = serde_json::from_slice(bytes)?;
        value["schema_version"] = json!(2);
        assert!(Manifest::parse(&serde_json::to_vec(&value)?).is_err());
        value = serde_json::from_slice(bytes)?;
        value["targets"].as_array_mut().unwrap().pop();
        assert!(Manifest::parse(&serde_json::to_vec(&value)?).is_err());
        Ok(())
    }
}
