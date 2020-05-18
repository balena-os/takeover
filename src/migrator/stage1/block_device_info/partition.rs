use std::path::PathBuf;
use std::rc::Rc;

use crate::{
    common::path_append,
    stage1::block_device_info::{block_device::BlockDevice, mount::Mount},
};

#[derive(Clone)]
pub(crate) struct Partition {
    pub name: String,
    pub major: u64,
    pub minor: u64,
    pub mounted: Option<Mount>,
    pub parent: Rc<Box<dyn BlockDevice>>,
}

impl BlockDevice for Partition {
    fn get_major(&self) -> u64 {
        self.major
    }

    fn get_minor(&self) -> u64 {
        self.minor
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
        Some(&self.parent)
    }

    fn is_partition(&self) -> bool {
        true
    }
}
