use crate::common::{file_exists, path_append};

use libc::{close, open, O_RDWR};
use log::{debug, error};
use std::ffi::CString;
use std::io;
use std::path::Path;

pub fn check_loop_control<P: AsRef<Path>>(text: &str, base_path: P) {
    debug!("{}", text);
    debug!(
        "check if /dev/loop-control exists in new /dev: {}",
        file_exists(path_append(base_path.as_ref(), "loop-control"))
    );

    let path = match CString::new("/dev/loop-control") {
        Ok(path) => path,
        Err(why) => {
            error!(
                "Failed to create cstring from path: '/dev/loop-control', error: {:?}",
                why
            );
            return;
        }
    };

    let path_ptr = path.into_raw();
    let fd = unsafe { open(path_ptr, O_RDWR) };
    let _dummy = unsafe { CString::from_raw(path_ptr) };
    if fd >= 0 {
        debug!("open /dev/loop-control succeeded");
        let _res = unsafe { close(fd) };
    } else {
        debug!(
            "open /dev/loop-control failed with error: {}",
            io::Error::last_os_error()
        );
    }
}
