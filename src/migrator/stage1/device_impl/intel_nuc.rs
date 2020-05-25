use log::{error, info};

use crate::stage1::device_impl::check_os;
use crate::{
    common::{MigError, Options},
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
    pub fn from_config(opts: &Options) -> Result<IntelNuc, MigError> {
        const SUPPORTED_OSSES: &[&str] = &[
            "Ubuntu 18.04.3 LTS",
            "Ubuntu 18.04.2 LTS",
            "Ubuntu 16.04.2 LTS",
            "Ubuntu 16.04.6 LTS",
            "Ubuntu 14.04.2 LTS",
            "Ubuntu 14.04.5 LTS",
            "Ubuntu 14.04.6 LTS",
            "Manjaro Linux",
        ];

        if !check_os(SUPPORTED_OSSES, opts, "Generic x86_64/Intel Nuc")? {
            return Err(MigError::displayed());
        }

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
            return Err(MigError::displayed());
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
