use std::io::Read;
use std::path::PathBuf;

use flate2::read::GzDecoder;
use log::{debug, trace};

use crate::{
    common::{MigError, MigErrorKind},
    stage2::disk_util::image_file::ImageFile,
};

const DEF_READ_BUFFER: usize = 1024 * 1024;

pub(crate) struct GZipStream<R> {
    decoder: GzDecoder<R>,
    bytes_read: u64,
}

impl<R: Read> GZipStream<R> {
    pub fn new(stream: R) -> Result<GZipStream<R>, MigError> {
        trace!("new: entered ");

        Ok(GZipStream {
            decoder: GzDecoder::new(stream),
            bytes_read: 0,
        })
    }

    fn seek(&mut self, offset: u64) -> Result<(), MigError> {
        trace!(
            "seek: entered with offset {}, bytes_read: {}",
            offset,
            self.bytes_read
        );

        let mut to_read = if offset < self.bytes_read {
            return Err(MigError::from_remark(
                MigErrorKind::InvState,
                "cannot seek backwards on stream",
            ));
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
                                &format!("seek: read from stream, error {:?}", why),
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
                        &format!("seek: failed to read from stream, error {:?}", why),
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

impl<R: Read> ImageFile for GZipStream<R> {
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
                &format!("failed to read from stream, error {:?}", why),
            )),
        }
    }
    fn get_path(&self) -> PathBuf {
        PathBuf::from("STREAM")
    }
}
