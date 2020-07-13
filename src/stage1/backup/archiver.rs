use crate::common::error::Result;
use std::path::Path;

pub trait Archiver {
    fn add_file(&mut self, target: &Path, source: &Path) -> Result<()>;
    fn finish(&mut self) -> Result<()>;
}
