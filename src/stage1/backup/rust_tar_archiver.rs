use crate::{
    common::error::{Result, ToError},
    stage1::backup::archiver::Archiver,
};

use flate2::{write::GzEncoder, Compression};
use std::fs::File;
use std::path::Path;
use tar::Builder;

pub(crate) struct RustTarArchiver {
    archive: Builder<GzEncoder<File>>,
}

// use rust internal tar / gzip for archiving

impl RustTarArchiver {
    pub fn new<P: AsRef<Path>>(file: P) -> Result<RustTarArchiver> {
        Ok(RustTarArchiver {
            archive: Builder::new(GzEncoder::new(
                File::create(file.as_ref()).upstream_with_context(&format!(
                    "Failed to create backup in file '{}'",
                    file.as_ref().display()
                ))?,
                Compression::default(),
            )),
        })
    }
}

impl Archiver for RustTarArchiver {
    fn add_file(&mut self, target: &Path, source: &Path) -> Result<()> {
        self.archive
            .append_path_with_name(source, target)
            .upstream_with_context(&format!(
                "Failed to append file: '{}' to archive path: '{}'",
                source.display(),
                target.display()
            ))
    }

    fn finish(&mut self) -> Result<()> {
        self.archive
            .finish()
            .upstream_with_context("Failed to create backup archive")
    }
}
