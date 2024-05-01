use log::error;

use crate::common::{is_admin, Error, Options, Result};

/// Performs checks to ensure that the program can run properly with the
/// provided command-line options. Returns an error if the program cannot run
/// for some reason.
pub(crate) fn do_early_checks(_opts: &Options) -> Result<()> {
    if !is_admin()? {
        error!("please run this program as root");
        return Err(Error::displayed());
    }

    Ok(())
}
