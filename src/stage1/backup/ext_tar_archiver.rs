use log::{debug, warn};
use std::fs::{create_dir_all, remove_dir_all};
use std::path::{Path, PathBuf};

use crate::common::system::symlink;
use crate::stage1::utils::mktemp;
use crate::{
    common::{
        call,
        defs::{BACKUP_ARCH_NAME, TAR_CMD},
        dir_exists,
        error::{Error, ErrorKind, Result, ToError},
        path_append,
    },
    stage1::backup::archiver::Archiver,
};

// use external tar / gzip for archiving
// strategy is to link  (ln -s ) all files / directories to a temporary directory
// and tar/gizip that directory on finish
#[cfg(target_os = "linux")]
pub(crate) struct ExtTarArchiver {
    tmp_dir: PathBuf,
    archive: PathBuf,
}

#[cfg(target_os = "linux")]
impl ExtTarArchiver {
    pub fn new<P: AsRef<Path>>(file: P) -> Result<ExtTarArchiver> {
        const NO_PATH: Option<&Path> = None;
        Ok(ExtTarArchiver {
            tmp_dir: mktemp(true, None, None, NO_PATH)?,
            archive: PathBuf::from(file.as_ref()),
        })
    }
}

#[cfg(target_os = "linux")]
impl Archiver for ExtTarArchiver {
    fn add_file(&mut self, target: &Path, source: &Path) -> Result<()> {
        debug!(
            "ExtTarArchiver::add_file: '{}' , '{}'",
            target.display(),
            source.display()
        );
        if let Some(parent_dir) = target.parent() {
            let parent_dir = path_append(&self.tmp_dir, parent_dir);
            if !dir_exists(&parent_dir).upstream_with_context(&format!(
                "Failed to access directory '{}'",
                parent_dir.display()
            ))? {
                debug!(
                    "ExtTarArchiver::add_file: create directory '{}'",
                    parent_dir.display()
                );
                create_dir_all(&parent_dir).upstream_with_context(&format!(
                    "Failed to create directory '{}'",
                    parent_dir.display()
                ))?;
            }
        }

        let lnk_target = path_append(&self.tmp_dir, &target);

        debug!(
            "ExtTarArchiver::add_file: link '{}' to '{}'",
            source.display(),
            lnk_target.display()
        );

        symlink(source, &lnk_target).upstream_with_context(&format!(
            "Failed to link '{}' to '{}'",
            source.display(),
            lnk_target.display()
        ))?;
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        let _res = call_command!(
            TAR_CMD,
            &[
                "-h",
                "-czf",
                BACKUP_ARCH_NAME,
                "-C",
                &*self.tmp_dir.to_string_lossy(),
                ".",
            ],
            &format!("Failed to create archive in '{}'", self.archive.display(),)
        )?;

        if let Err(why) = remove_dir_all(&self.tmp_dir) {
            warn!(
                "Failed to delete temporary directory '{}' error: {:?}",
                self.tmp_dir.display(),
                why
            );
        }

        Ok(())
    }
}
