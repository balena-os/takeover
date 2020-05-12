use std::path::PathBuf;

use crate::common::MigError;

pub(crate) trait ImageFile {
    fn fill(&mut self, offset: u64, buffer: &mut [u8]) -> Result<(), MigError>;
    fn get_path(&self) -> PathBuf;
}
