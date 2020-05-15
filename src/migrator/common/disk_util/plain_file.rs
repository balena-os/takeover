use failure::ResultExt;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::common::{disk_util::image_file::ImageFile, MigErrCtx, MigError, MigErrorKind};

pub(crate) struct PlainFile {
    path: PathBuf,
    file: File,
}

impl PlainFile {
    pub fn new(path: &Path) -> Result<PlainFile, MigError> {
        let file = match OpenOptions::new()
            .write(false)
            .read(true)
            .create(false)
            .open(path)
        {
            Ok(file) => file,
            Err(why) => {
                return Err(MigError::from_remark(
                    MigErrorKind::Upstream,
                    &format!(
                        "failed to open file for reading: '{}', error {:?}",
                        path.display(),
                        why
                    ),
                ));
            }
        };

        Ok(PlainFile {
            path: path.to_path_buf(),
            file,
        })
    }
}

impl ImageFile for PlainFile {
    fn fill(&mut self, offset: u64, buffer: &mut [u8]) -> Result<(), MigError> {
        self.file
            .seek(SeekFrom::Start(offset))
            .context(MigErrCtx::from_remark(
                MigErrorKind::Upstream,
                &format!("failed to seek to offset {}", offset),
            ))?;
        match self.file.read_exact(buffer) {
            Ok(_) => Ok(()),
            Err(why) => Err(MigError::from_remark(
                MigErrorKind::Upstream,
                &format!(
                    "failed to read from file: '{}', error {:?}",
                    self.path.display(),
                    why
                ),
            )),
        }
    }
    fn get_path(&self) -> PathBuf {
        self.path.clone()
    }
}
