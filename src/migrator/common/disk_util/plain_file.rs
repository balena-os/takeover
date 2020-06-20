use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::common::{disk_util::image_file::ImageFile, Error, ErrorKind, Result, ToError};

pub(crate) struct PlainFile {
    path: PathBuf,
    file: File,
}

impl PlainFile {
    pub fn new(path: &Path) -> Result<PlainFile> {
        let file = match OpenOptions::new()
            .write(false)
            .read(true)
            .create(false)
            .open(path)
        {
            Ok(file) => file,
            Err(why) => {
                return Err(Error::with_context(
                    ErrorKind::Upstream,
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
    fn fill(&mut self, offset: u64, buffer: &mut [u8]) -> Result<()> {
        self.file
            .seek(SeekFrom::Start(offset))
            .upstream_with_context(&format!("failed to seek to offset {}", offset))?;
        match self.file.read_exact(buffer) {
            Ok(_) => Ok(()),
            Err(why) => Err(Error::with_context(
                ErrorKind::Upstream,
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
