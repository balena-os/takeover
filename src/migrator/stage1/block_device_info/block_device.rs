use std::path::PathBuf;
use std::rc::Rc;
use std::fmt::{self, Debug};

use crate::stage1::block_device_info::mount::Mount;

pub(crate) trait BlockDevice {
    fn get_major(&self) -> u64;
    fn get_minor(&self) -> u64;
    fn get_mountpoint(&self) -> &Option<Mount>;
    fn get_name(&self) -> &str;
    fn get_dev_path(&self) -> PathBuf;
    fn get_parent(&self) -> Option<&Rc<Box<dyn BlockDevice>>>;
    fn is_partition(&self) -> bool;
}

impl Debug for dyn BlockDevice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parent_val = if let Some(parent) = self.get_parent() {
            parent.as_ref().get_dev_path().display().to_string()
        } else {
            "None".to_string()
        };

        f.debug_struct("BlockDevice")
            .field("name", &self.get_name())
            .field("major", &self.get_major())
            .field("minor", &self.get_minor())
            .field("mounted", &self.get_mountpoint())
            .field("parent", &parent_val)
            .finish()
    }
}
