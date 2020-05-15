use std::fmt::{self, Display, Debug};

use crate::{
    stage1::{defs::DeviceType, },
};

pub(crate) trait Device {
    fn supports_device_type(&self, dev_type: &str) -> bool;
    fn get_device_type(&self) -> DeviceType;
}

impl Display for dyn Device {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{},", self.get_device_type() )
    }
}

impl Debug for dyn Device {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Device")
            .field("type", &self.get_device_type())
            .finish()
    }
}
