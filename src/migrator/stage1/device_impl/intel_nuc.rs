use log::{error, info, warn};

use crate::common::get_os_name;
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

        let os_name = get_os_name()?;
        info!("Detected OS name is {}", os_name);

        let os_supported = if let Some(_) = SUPPORTED_OSSES.iter().position(|&r| r == os_name) {
            true
        } else {
            false
        };

        if !os_supported {
            if opts.is_os_check() {
                error!(
                    "The OS '{}' has not been tested with {} for device type IntelNuc, to override this check use the no-os-check option on the command line",
                    os_name,
                    env!("CARGO_PKG_NAME")
                );
                return Err(MigError::displayed());
            } else {
                warn!(
                    "The OS '{}' has not been tested with {} for device type IntelNuc, prodeeding due to no-os-check option", os_name, env!("CARGO_PKG_NAME"));
            }
        }

        // **********************************************************************
        // ** AMD64 specific initialisation/checksget_
        // **********************************************************************

        // TODO: determine boot device
        // use config.migrate.flash_device
        // if EFI boot look for EFI partition
        // else look for /boot

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
