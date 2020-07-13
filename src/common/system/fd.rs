use std::ffi::CString;
use std::io;
use std::os::raw::c_int;

use libc::{self, close, open, ENODEV, ENOENT};
use std::path::Path;

use log::debug;
use nix::errno::errno;

use crate::common::{
    error::{Error, ErrorKind, Result},
    path_to_cstring,
};

pub(crate) struct Fd {
    fd: c_int,
}

impl Fd {
    pub fn get_fd(&self) -> c_int {
        self.fd
    }

    pub fn open<P: AsRef<Path>>(file: P, mode: c_int) -> Result<Fd> {
        let file_name = path_to_cstring(&file)?;
        let fname_ptr = file_name.into_raw();
        let fd = unsafe { open(fname_ptr, mode) };
        let _file_name = unsafe { CString::from_raw(fname_ptr) };
        if fd >= 0 {
            debug!(
                "Fd::open: opened path: '{}' as fd {}",
                file.as_ref().display(),
                fd
            );
            Ok(Fd { fd })
        } else {
            let err_no = errno();
            debug!(
                "Fd:open: failed to open file '{}', error {}, ",
                file.as_ref().display(),
                err_no,
            );

            if (err_no == ENOENT) || (err_no == ENODEV) {
                Err(Error::with_context(
                    ErrorKind::FileNotFound,
                    &format!(
                        "Fd::open: Failed to open file '{}', error {}",
                        file.as_ref().display(),
                        io::Error::last_os_error()
                    ),
                ))
            } else {
                Err(Error::with_context(
                    ErrorKind::Upstream,
                    &format!(
                        "Fd::open: Failed to open file '{}', error {}",
                        file.as_ref().display(),
                        io::Error::last_os_error()
                    ),
                ))
            }
        }
    }
}

impl Drop for Fd {
    fn drop(&mut self) {
        unsafe { close(self.fd) };
    }
}
