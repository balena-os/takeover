use std::path::PathBuf;
use std::rc::Rc;

use crate::stage1::block_device_info::DeviceNum;
use crate::{
    common::path_append,
    stage1::block_device_info::{block_device::BlockDevice, mount::Mount},
    MigError,
};

#[derive(Clone, Debug)]
pub(crate) struct Device {
    pub name: String,
    pub device_num: DeviceNum,
    pub mounted: Option<Mount>,
}

impl BlockDevice for Device {
    fn get_device_num(&self) -> &DeviceNum {
        &self.device_num
    }

    fn get_mountpoint(&self) -> &Option<Mount> {
        &self.mounted
    }

    fn get_name(&self) -> &str {
        self.name.as_str()
    }

    fn get_dev_path(&self) -> PathBuf {
        path_append("/dev", &self.name)
    }

    fn get_parent(&self) -> Option<&Rc<Box<dyn BlockDevice>>> {
        None
    }

    fn is_partition(&self) -> bool {
        false
    }

    fn set_mountpoint(&mut self, mountpoint: Mount) {
        self.mounted = Some(mountpoint);
    }
}
