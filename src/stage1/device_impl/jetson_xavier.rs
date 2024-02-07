use log::{debug, error, trace, info};

use crate::stage1::device_impl::check_os;
use crate::{
    common::{Error, Options, Result},
    // linux_common::is_secure_boot,
    stage1::{
        defs::{DeviceType, DEV_TYPE_JETSON_XAVIER},
        device::Device,
        utils::is_secure_boot,
    },
};

pub(crate) fn is_jetson_xavier(opts: &Options, model_string: &str) -> Result<Option<Box<dyn Device>>> {
    trace!(
        "JetsonXavier::is_jetson_xavier: entered with model string: '{}'",
        model_string
    );

    if model_string.eq("Jetson-AGX") {
        // TODO: found this device model on AGX Xavier running L4T 35.4.1
        debug!("match found for Xavier AGX");
        Ok(Some(Box::new(JetsonXavier::from_config(opts)?)))
    } else {
        debug!("no match for Jetson-AGX on: <{}>", model_string);
        Ok(None)
    }
}

const XAVIER_AGX_SLUGS: [&str; 1] = [DEV_TYPE_JETSON_XAVIER];

pub(crate) struct JetsonXavier;

impl JetsonXavier {
    pub fn from_config(opts: &Options) -> Result<JetsonXavier> {
        const SUPPORTED_OSSES: &[&str] = &[
                   "balenaOS 5.1.20",
                   "balenaOS 3.1.3+rev1",
        ];

        if opts.migrate() {
            if !check_os(SUPPORTED_OSSES, opts, "balenaOS 5.1.20")? {
                return Err(Error::displayed());
            }

            // **********************************************************************
            // ** Xavier AGX specific initialisation/checks
            // **********************************************************************


        }
        Ok(JetsonXavier)
    }
}

impl<'a> Device for JetsonXavier {
    fn supports_device_type(&self, dev_type: &str) -> bool {
        XAVIER_AGX_SLUGS.contains(&dev_type)
    }
    fn get_device_type(&self) -> DeviceType {
        DeviceType::JetsonXavier
    }
}