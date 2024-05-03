use log::error;

use super::{block_device_info::BlockDeviceInfo, get_block_dev_info, get_log_device};
use crate::common::{is_admin, Error, Options, Result};

/// Performs checks to ensure that the program can run properly with the
/// provided command-line options. Returns an error if the program cannot run
/// for some reason.
pub(crate) fn do_early_checks(opts: &Options) -> Result<()> {
    if !is_admin()? {
        error!("please run this program as root");
        return Err(Error::displayed());
    }

    let block_dev_info = get_block_dev_info()?;

    if !check_log_device(opts, &block_dev_info) {
        error!("the requested log device is not suitable for writing stage2 logs");
        return Err(Error::displayed());
    }

    Ok(())
}

/// Checks if the log device requested with `--log-device` is suitable for
/// writing stage2 logs.
fn check_log_device(opts: &Options, block_dev_info: &BlockDeviceInfo) -> bool {
    if opts.log_to().is_none() {
        // No log device requested: that's fine!
        return true;
    };

    // But if the user requested a log device, we must be able to get it. If we
    // can't, *that* is a problem!
    get_log_device(opts, block_dev_info).is_some()
}
