use crate::{stage1::defs::DeviceType, stage1::device::Device};

/// The Dummy device skips all compatibility checks. This is useful when the
/// user's actual device type is not supported by takeover, but it is
/// technically capable of running the migration. This device type is used when
/// the user passes the `--no-dt-check` CLI option.
pub(crate) struct Dummy;

impl Dummy {
    pub fn new() -> Dummy {
        Dummy
    }
}

impl Device for Dummy {
    fn supports_device_type(&self, _dev_type: &str) -> bool {
        true
    }
    fn get_device_type(&self) -> DeviceType {
        DeviceType::Dummy
    }
}
