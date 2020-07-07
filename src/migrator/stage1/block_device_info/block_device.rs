use std::fmt::{self, Debug};
use std::path::PathBuf;
use std::rc::Rc;

use crate::stage1::block_device_info::mount::Mount;
use crate::stage1::block_device_info::partition::PartitionInfo;
use crate::stage1::block_device_info::DeviceNum;

pub(crate) trait BlockDevice {
    fn get_device_num(&self) -> &DeviceNum;
    fn get_mountpoint(&self) -> &Option<Mount>;
    fn get_name(&self) -> &str;
    fn get_dev_path(&self) -> PathBuf;
    fn get_parent(&self) -> Option<&Rc<Box<dyn BlockDevice>>>;
    fn is_partition(&self) -> bool;
    fn set_mountpoint(&mut self, mountpoint: Mount);
    fn get_partition_info(&self) -> Option<&PartitionInfo>;
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
            .field("device_num", &self.get_device_num())
            .field("mounted", &self.get_mountpoint())
            .field("parent", &parent_val)
            .finish()
    }
}
