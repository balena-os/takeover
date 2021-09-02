use log::{error, info};

use crate::{
    common::{Error, Options, Result},
    // linux_common::is_secure_boot,
    stage1::{
        defs::{DeviceType, DEV_TYPE_GEN_X86_64, DEV_TYPE_INTEL_NUC},
        device::Device,
        utils::is_secure_boot,
    },
};

const X86_SLUGS: [&str; 2] = [DEV_TYPE_INTEL_NUC, DEV_TYPE_GEN_X86_64];

pub(crate) struct IntelNuc;

impl IntelNuc {
    pub fn from_config(opts: &Options) -> Result<IntelNuc> {
        if opts.migrate() {
            // **********************************************************************
            // ** AMD64 specific initialisation/checks
            // **********************************************************************

            let secure_boot = is_secure_boot()?;
            info!(
                "Secure boot is {}enabled",
                if secure_boot { "" } else { "not " }
            );

            if secure_boot {
                error!(
                    "{} does not currently support systems with secure boot enabled.",
                    env!("CARGO_PKG_NAME")
                );
                return Err(Error::displayed());
            }
        }
        Ok(IntelNuc)
    }
}

impl<'a> Device for IntelNuc {
    fn supports_device_type(&self, dev_type: &str) -> bool {
        X86_SLUGS.contains(&dev_type)
    }
    fn get_device_type(&self) -> DeviceType {
        DeviceType::IntelNuc
    }
}
