use flate2::read::GzDecoder;
use log::{debug, trace};
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};

const DEF_READ_BUFFER: usize = 1024 * 1024;

use crate::common::{disk_util::image_file::ImageFile, MigError, MigErrorKind};

pub(crate) struct GZipFile {
    path: PathBuf,
    decoder: GzDecoder<File>,
    bytes_read: u64,
}

impl GZipFile {
    pub fn new(path: &Path) -> Result<GZipFile, MigError> {
        trace!("new: entered with '{}'", path.display());
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

        Ok(GZipFile {
            path: path.to_path_buf(),
            decoder: GzDecoder::new(file),
            bytes_read: 0,
        })
    }

    fn reset(&mut self) -> Result<(), MigError> {
        trace!("reset: entered");
        let file = match OpenOptions::new()
            .write(false)
            .read(true)
            .create(false)
            .open(&self.path)
        {
            Ok(file) => file,
            Err(why) => {
                return Err(MigError::from_remark(
                    MigErrorKind::Upstream,
                    &format!(
                        "failed to reopen file for reading: '{}', error {:?}",
                        self.path.display(),
                        why
                    ),
                ));
            }
        };

        self.decoder = GzDecoder::new(file);
        self.bytes_read = 0;
        Ok(())
    }

    fn seek(&mut self, offset: u64) -> Result<(), MigError> {
        trace!(
            "seek: entered with offset {}, bytes_read: {}",
            offset,
            self.bytes_read
        );
        let mut to_read = if offset < self.bytes_read {
            self.reset()?;
            offset
        } else {
            offset - self.bytes_read
        };

        trace!("seek: to_read: {}", to_read);

        if to_read == 0 {
            Ok(())
        } else {
            let mut buffer: [u8; DEF_READ_BUFFER] = [0; DEF_READ_BUFFER];
            if to_read >= (DEF_READ_BUFFER as u64) {
                loop {
                    match self.decoder.read(&mut buffer) {
                        Ok(bytes_read) => {
                            to_read -= bytes_read as u64;
                            // debug!("bytes_read: {}, to_read:{}", bytes_read, to_read);
                            if to_read < DEF_READ_BUFFER as u64 {
                                trace!(
                                    "seek: done with DEF_BUFFER, to_read: {}, bytes_read: {}",
                                    to_read,
                                    bytes_read
                                );
                                break;
                            }
                        }
                        Err(why) => {
                            return Err(MigError::from_remark(
                                MigErrorKind::Upstream,
                                &format!(
                                    "seek: failed to reopen file for reading: '{}', error {:?}",
                                    self.path.display(),
                                    why
                                ),
                            ));
                        }
                    }
                }
            }

            if to_read > 0 {
                trace!("seek: last buffer, to_read: {}", to_read);
                match self.decoder.read_exact(&mut buffer[0..to_read as usize]) {
                    Ok(_) => {
                        trace!("seek: read, got {} bytes", to_read);
                        self.bytes_read = offset;
                        Ok(())
                    }
                    Err(why) => Err(MigError::from_remark(
                        MigErrorKind::Upstream,
                        &format!(
                            "seek: failed to read from file  file '{}', error {:?}",
                            self.path.display(),
                            why
                        ),
                    )),
                }
            } else {
                debug!("seek: nothing more to_read: {}", to_read);
                self.bytes_read = offset;
                Ok(())
            }
        }
    }
}

impl ImageFile for GZipFile {
    fn fill(&mut self, offset: u64, buffer: &mut [u8]) -> Result<(), MigError> {
        trace!(
            "fill: entered with offset {}, size {}",
            offset,
            buffer.len()
        );
        self.seek(offset)?;

        trace!("fill: bytes_read after seek {}", self.bytes_read);

        match self.decoder.read_exact(buffer) {
            Ok(_) => {
                self.bytes_read = offset + buffer.len() as u64;
                Ok(())
            }
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
