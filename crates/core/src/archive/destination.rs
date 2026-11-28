use std::ffi::OsString;
use std::path::{Path, PathBuf};

use cap_fs_ext::DirExt;
use cap_std::ambient_authority;
use cap_std::fs::{Dir, DirBuilder};
use same_file::Handle;

use super::ArchiveError;

pub(super) struct DestinationParent {
    pub path: PathBuf,
    pub parent_path: PathBuf,
    pub leaf: OsString,
    pub parent: Dir,
    identity: Handle,
}

impl DestinationParent {
    pub fn open(destination: &Path, description: &str) -> Result<Self, ArchiveError> {
        let invalid = || ArchiveError::InvalidDestination(description.into());
        if destination.as_os_str().is_empty() {
            return Err(invalid());
        }
        let path = if destination.is_absolute() {
            destination.to_owned()
        } else {
            std::env::current_dir()?.join(destination)
        };
        let leaf = path.file_name().ok_or_else(invalid)?.to_owned();
        let parent_path = path.parent().ok_or_else(invalid)?.to_owned();
        let parent = Dir::open_ambient_dir(&parent_path, ambient_authority())?;
        let identity = directory_identity(&parent)?;
        Ok(Self {
            path,
            parent_path,
            leaf,
            parent,
            identity,
        })
    }

    pub fn exists_error(&self) -> ArchiveError {
        ArchiveError::DestinationExists(self.path.to_string_lossy().into_owned())
    }

    pub fn verify_parent(&self) -> Result<(), ArchiveError> {
        let current = Dir::open_ambient_dir(&self.parent_path, ambient_authority())
            .map_err(|_| ArchiveError::DestinationChanged)?;
        if directory_identity(&current)? != self.identity {
            return Err(ArchiveError::DestinationChanged);
        }
        Ok(())
    }

    pub fn verify_directory(&self, leaf: &Path, identity: &Handle) -> Result<(), ArchiveError> {
        self.verify_parent()?;
        let current = self
            .parent
            .open_dir_nofollow(leaf)
            .map_err(|_| ArchiveError::DestinationChanged)?;
        if directory_identity(&current)? != *identity {
            return Err(ArchiveError::DestinationChanged);
        }
        Ok(())
    }
}

pub(super) fn directory_identity(directory: &Dir) -> Result<Handle, ArchiveError> {
    Ok(Handle::from_file(directory.try_clone()?.into_std_file())?)
}

pub(super) fn private_directory_builder() -> DirBuilder {
    let mut builder = DirBuilder::new();
    #[cfg(unix)]
    {
        use cap_std::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.recursive(false);
    builder
}
