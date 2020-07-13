use std::path::PathBuf;

use crate::common::Result;

pub(crate) trait ImageFile {
    fn fill(&mut self, offset: u64, buffer: &mut [u8]) -> Result<()>;
    fn get_path(&self) -> PathBuf;
}
