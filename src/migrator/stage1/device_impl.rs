use log::{error, info, warn};
use std::fs::read_to_string;

use crate::common::ToError;
use crate::{
    common::{get_os_name, Error, ErrorKind, Options, Result},
    stage1::{defs::OSArch, device::Device, utils::get_os_arch},
};

// mod beaglebone;
mod beaglebone;
mod intel_nuc;
mod raspberrypi;

const DEVICE_TREE_MODEL: &str = "/proc/device-tree/model";

pub(crate) fn check_os(supported: &[&str], opts: &Options, dev_type: &str) -> Result<bool> {
    let os_name = get_os_name()?;
    info!("Detected OS name is {}", os_name);

    let os_supported = supported.iter().any(|&r| r == os_name);

    if !os_supported {
        if opts.os_check() {
            error!(
                "The OS '{}' has not been tested with {} for device type {}, to override this check use the no-os-check option on the command line",
                os_name,
                dev_type,
                env!("CARGO_PKG_NAME")
            );
            Ok(false)
        } else {
            warn!(
                "The OS '{}' has not been tested with {} for device type IntelNuc, prodeeding due to no-os-check option", os_name, env!("CARGO_PKG_NAME"));
            Ok(true)
        }
    } else {
        Ok(true)
    }
}

pub(crate) fn get_device(opts: &Options) -> Result<Box<dyn Device>> {
    let os_arch = get_os_arch()?;
    info!("Detected OS Architecture is {:?}", os_arch);

    match os_arch {
        OSArch::ARMHF => {
            let dev_tree_model = String::from(
                read_to_string(DEVICE_TREE_MODEL)
                    .upstream_with_context(&format!(
                        "get_device: unable to determine model due to inaccessible file '{}'",
                        DEVICE_TREE_MODEL
                    ))?
                    .trim_end_matches('\0')
                    .trim_end(),
            );

            if let Some(device) = raspberrypi::is_rpi(opts, &dev_tree_model)? {
                return Ok(device);
            }

            if let Some(device) = beaglebone::is_bb(opts, &dev_tree_model)? {
                return Ok(device);
            }

            let message = format!(
                "Your device type: '{}' is not supported by balena-migrate.",
                dev_tree_model
            );
            error!("{}", message);
            Err(Error::with_context(ErrorKind::InvState, &message))
        }
        OSArch::AMD64 => Ok(Box::new(intel_nuc::IntelNuc::from_config(opts)?)),
        /*            OSArch::I386 => {
                    migrator.init_i386()?;
                },
        */
        _ => Err(Error::with_context(
            ErrorKind::InvParam,
            &format!("get_device: unexpected OsArch encountered: {:?}", os_arch),
        )),
    }
}
