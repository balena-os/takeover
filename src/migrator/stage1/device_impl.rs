use failure::ResultExt;
use log::{error, info, warn};
use std::fs::read_to_string;

use crate::common::get_os_name;
use crate::stage1::utils::get_os_arch;
use crate::{
    common::{MigErrCtx, MigError, MigErrorKind},
    stage1::defs::OSArch,
    stage1::device::Device,
    Options,
};

// mod beaglebone;
mod intel_nuc;
mod raspberrypi;

const DEVICE_TREE_MODEL: &str = "/proc/device-tree/model";

pub(crate) fn check_os(
    supported: &[&str],
    opts: &Options,
    dev_type: &str,
) -> Result<bool, MigError> {
    let os_name = get_os_name()?;
    info!("Detected OS name is {}", os_name);

    let os_supported = if let Some(_) = supported.iter().position(|&r| r == os_name) {
        true
    } else {
        false
    };

    if !os_supported {
        if opts.is_os_check() {
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

pub(crate) fn get_device(opts: &Options) -> Result<Box<dyn Device>, MigError> {
    let os_arch = get_os_arch()?;
    info!("Detected OS Architecture is {:?}", os_arch);

    match os_arch {
        OSArch::ARMHF => {
            let dev_tree_model = String::from(
                read_to_string(DEVICE_TREE_MODEL)
                    .context(MigErrCtx::from_remark(
                        MigErrorKind::Upstream,
                        &format!(
                            "get_device: unable to determine model due to inaccessible file '{}'",
                            DEVICE_TREE_MODEL
                        ),
                    ))?
                    .trim_end_matches('\0')
                    .trim_end(),
            );

            if let Some(device) = raspberrypi::is_rpi(opts, &dev_tree_model)? {
                return Ok(device);
            }
            /*
            if let Some(device) = beaglebone::is_bb(mig_info, config, s2_cfg, &dev_tree_model)? {
                return Ok(device);
            }

             */

            let message = format!(
                "Your device type: '{}' is not supported by balena-migrate.",
                dev_tree_model
            );
            error!("{}", message);
            Err(MigError::from_remark(MigErrorKind::InvState, &message))
        }
        OSArch::AMD64 => Ok(Box::new(intel_nuc::IntelNuc::from_config(opts)?)),
        /*            OSArch::I386 => {
                    migrator.init_i386()?;
                },
        */
        _ => Err(MigError::from_remark(
            MigErrorKind::InvParam,
            &format!("get_device: unexpected OsArch encountered: {:?}", os_arch),
        )),
    }
}
