use log::{debug, trace};

use crate::stage1::device_impl::check_os;
use crate::{
    common::{Error, Options, Result},
    // linux_common::is_secure_boot,
    stage1::{
        defs::{DeviceType, DEV_TYPE_JETSON_XAVIER, DEV_TYPE_JETSON_XAVIER_NX},
        device::Device,
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
    } else if model_string.eq("NVIDIA Jetson Xavier NX Developer Kit") {
        debug!("match found for Xavier NX Devkit");
        Ok(Some(Box::new(JetsonXavierNX::from_config(opts)?)))
    } else  {
        debug!("no match for Jetson-AGX or NVIDIA Jetson Xavier NX Developer Kit on: <{}>", model_string);
        Ok(None)
    }
}

const XAVIER_AGX_SLUGS: [&str; 1] = [DEV_TYPE_JETSON_XAVIER];
const XAVIER_NX_SLUGS: [&str; 1] = [DEV_TYPE_JETSON_XAVIER_NX];


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

pub(crate) struct JetsonXavierNX;

impl JetsonXavierNX {
    pub fn from_config(opts: &Options) -> Result<JetsonXavierNX> {
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
        Ok(JetsonXavierNX)
    }
}

impl<'a> Device for JetsonXavierNX {
    fn supports_device_type(&self, dev_type: &str) -> bool {
        XAVIER_NX_SLUGS.contains(&dev_type)
    }
    fn get_device_type(&self) -> DeviceType {
        DeviceType::JetsonXavierNX
    }
}
