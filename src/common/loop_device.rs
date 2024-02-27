use std::fmt::{self, Debug, Formatter};
use std::mem::MaybeUninit;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::Duration;

use nix::errno::errno;
use std::cmp::min;
use std::ffi::{CString, OsStr};
use std::io;
use std::os::unix::ffi::OsStrExt;

use crate::common::{
    defs::IoctlReq,
    error::{Error, ErrorKind, Result, ToError},
    hex_dump, path_to_cstring,
    system::fd::Fd,
};
use log::{debug, trace};

use libc::{self, ioctl, makedev, mknod, EAGAIN, ENXIO, O_CLOEXEC, O_RDWR, S_IFBLK};

const MAX_LOOP: u32 = 1024;

const IOCTL_LOOP_SET_FD: IoctlReq = 0x4c00;
const IOCTL_LOOP_CLR_FD: IoctlReq = 0x4c01;
const IOCTL_LOOP_CTL_GET_FREE: IoctlReq = 0x4c82;
const IOCTL_LOOP_GET_STATUS_64: IoctlReq = 0x4c05;
const IOCTL_LOOP_SET_STATUS_64: IoctlReq = 0x4c04;

const LO_NAME_SIZE: usize = 64;
const LO_KEY_SIZE: usize = 32;

#[repr(C)]
pub struct LoopInfo64 {
    lo_device: u64,
    lo_inode: u64,
    lo_rdevice: u64,
    lo_offset: u64,
    lo_sizelimit: u64,
    lo_number: u32,
    lo_encrypt_type: u32,
    lo_encrypt_key_size: u32,
    lo_flags: u32,
    lo_file_name: [u8; LO_NAME_SIZE],
    lo_crypt_name: [u8; LO_NAME_SIZE],
    lo_encrypt_key: [u8; LO_KEY_SIZE],
    lo_init: [u64; 2],
}

impl Debug for LoopInfo64 {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoopInfo64")
            .field("lo_device", &format!("0x{:x}", self.lo_device))
            .field("lo_inode", &self.lo_inode)
            .field("lo_rdevice", &self.lo_rdevice)
            .field("lo_offset", &self.lo_offset)
            .field("lo_sizelimit", &self.lo_sizelimit)
            .field("lo_number", &self.lo_number)
            .field("lo_encrypt_type", &self.lo_encrypt_type)
            .field("lo_encrypt_key_size", &self.lo_encrypt_key_size)
            .field("lo_flags", &self.lo_flags)
            .field("lo_file_name", &cbuffer_to_pathbuf(&self.lo_file_name))
            .field("lo_crypt_name", &hex_dump(&self.lo_crypt_name))
            .field(
                "lo_encrypt_key",
                &key_to_string(
                    &self.lo_encrypt_key,
                    Some(self.lo_encrypt_key_size as usize),
                ),
            )
            .field("lo_init", &self.lo_init)
            .finish()
    }
}

pub struct LoopDevice {
    fd: Fd,
    path: PathBuf,
    file: Option<PathBuf>,
    unset: bool,
}

impl LoopDevice {
    /// create or open loop device for index

    pub fn from_index(loop_index: u32, auto_unset: bool) -> Result<LoopDevice> {
        trace!(
            "from_index: entered with index {}, autounset: {}",
            loop_index,
            auto_unset
        );
        let path = PathBuf::from(&format!("/dev/loop{}", loop_index));

        // try to open device file
        match Fd::open(&path, O_RDWR | O_CLOEXEC) {
            Ok(fd) => {
                // device node exists, check if in use
                let mut loop_dev = LoopDevice {
                    path,
                    fd,
                    file: None,
                    unset: auto_unset,
                };

                match loop_dev.get_loop_info() {
                    Ok(loop_info) => {
                        // valid loop info, grab path
                        loop_dev.file = Some(cbuffer_to_pathbuf(&loop_info.lo_file_name))
                    }
                    Err(why) => {
                        if why.kind() != ErrorKind::DeviceNotFound {
                            return Err(Error::from_upstream(
                                Box::new(why),
                                &format!(
                                    "from_index: failed to retrieve loop info from device'{}'",
                                    loop_dev.path.display()
                                ),
                            ));
                        }
                    }
                }
                Ok(loop_dev)
            }
            Err(why) => {
                // failed to open device
                if why.kind() == ErrorKind::FileNotFound {
                    // file does not exist - try to create device node
                    let c_path = path_to_cstring(&path)?.into_raw();
                    let res = unsafe { mknod(c_path, S_IFBLK | 0o0644, makedev(7, loop_index)) };
                    let _c_path = unsafe { CString::from_raw(c_path) };
                    if res == 0 {
                        let fd = Fd::open(&path, O_RDWR | O_CLOEXEC)?;
                        Ok(LoopDevice {
                            path,
                            fd,
                            file: None,
                            unset: auto_unset,
                        })
                    } else {
                        Err(Error::with_all(
                            ErrorKind::Upstream,
                            &format!(
                                "from_index: Failed to create device node '{}'",
                                path.display()
                            ),
                            Box::new(io::Error::last_os_error()),
                        ))
                    }
                } else {
                    // some other error opening device
                    Err(Error::from_upstream(
                        Box::new(why),
                        &format!("from_index: Failed to open device '{}'", path.display()),
                    ))
                }
            }
        }
    }

    /// create a loop device associated with the given file
    pub fn for_file<P: AsRef<Path>>(
        file: P,
        offset: Option<u64>,
        size_limit: Option<u64>,
        loop_index: Option<u32>,
        auto_unset: bool,
    ) -> Result<LoopDevice> {
        let mut loop_device = if let Some(loop_index) = loop_index {
            LoopDevice::from_index(loop_index, auto_unset)?
        } else {
            LoopDevice::get_free(auto_unset)?
        };
        loop_device.setup(file, offset, size_limit)?;
        Ok(loop_device)
    }

    pub fn get_free(auto_unset: bool) -> Result<LoopDevice> {
        // Find a free loop device, try use /dev/loop-control first
        match Fd::open("/dev/loop-control", O_RDWR | O_CLOEXEC) {
            Ok(file_fd) => {
                let ioctl_res = unsafe { ioctl(file_fd.get_fd(), IOCTL_LOOP_CTL_GET_FREE) };
                if ioctl_res < 0 {
                    Err(Error::with_context(
                        ErrorKind::Upstream,
                        &format!(
                            "get_free: ioctl IOCTL_LOOP_CTL_GET_FREE failed with error: {}",
                            io::Error::last_os_error()
                        ),
                    ))
                } else {
                    // success, return loop number and open device fd
                    Ok(LoopDevice::from_index(ioctl_res as u32, auto_unset)?)
                }
            }
            Err(why) => {
                debug!(
                    "get_free: open /dev/loop-control returned error {:?}",
                    why.kind()
                );
                if why.kind() == ErrorKind::FileNotFound {
                    // if /dev/loop-control does not exist scan for free devices manually
                    debug!("get_free: /dev/loop-control does not exist scanning for free devices manually");
                    for loop_idx in 0..MAX_LOOP {
                        if PathBuf::from(&format!("/dev/loop{}", loop_idx)).exists() {
                            let mut loop_dev = LoopDevice::from_index(loop_idx, false)?;
                            if loop_dev.file.is_some() {
                                // device is in use
                                continue;
                            } else {
                                loop_dev.unset = auto_unset;
                                return Ok(loop_dev);
                            }
                        } else {
                            return LoopDevice::from_index(loop_idx, auto_unset);
                        }
                    }
                    Err(Error::with_context(
                        ErrorKind::NotFound,
                        "get_free: No free loop device was found",
                    ))
                } else {
                    Err(Error::with_context(
                        ErrorKind::NotFound,
                        "get_free: Unable to open /dev/loop-control",
                    ))
                }
            }
        }
    }

    /// retrieve the loop devices path
    pub fn get_path(&self) -> &Path {
        self.path.as_path()
    }

    #[allow(dead_code)]
    pub fn set_auto_unset(&mut self, auto_unset: bool) {
        self.unset = auto_unset;
    }

    fn set_info<P: AsRef<Path>>(&self, file: P, offset: u64, sizelimit: u64) -> Result<()> {
        trace!(
            "set_info: entered on '{}' with file: '{}', offset: 0x{:x}, size limit: 0x{:x}",
            self.path.display(),
            file.as_ref().display(),
            offset,
            sizelimit
        );
        let mut loop_info: LoopInfo64 = unsafe { MaybeUninit::zeroed().assume_init() };

        path_to_cbuffer(file, &mut loop_info.lo_file_name)?;
        debug!(
            "set_info: lo_file_name:\n{}",
            hex_dump(&loop_info.lo_file_name)
        );

        loop_info.lo_offset = offset;
        loop_info.lo_sizelimit = sizelimit;

        let mut retries = 3;
        loop {
            debug!(
                "set_info: calling IOCTL_LOOP_SET_STATUS_64, attempt {}",
                4 - retries
            );
            let ioctl_res =
                unsafe { ioctl(self.fd.get_fd(), IOCTL_LOOP_SET_STATUS_64, &loop_info) };
            if ioctl_res == 0 {
                return Ok(());
            } else {
                debug!(
                    "set_info: ioctl IOCTL_LOOP_SET_STATUS_64 returned {}",
                    io::Error::last_os_error()
                );
                if errno() == EAGAIN {
                    retries -= 1;
                    if retries > 0 {
                        sleep(Duration::from_millis(100));
                    }
                } else {
                    return Err(Error::with_context(
                        ErrorKind::Upstream,
                        &format!(
                            "set_info: ioctl IOCTL_LOOP_SET_STATUS_64 failed on device '{}', error {}",
                            self.path.display(),
                            io::Error::last_os_error()
                        ),
                    ));
                }
            }
        }
    }

    pub fn setup<P: AsRef<Path>>(
        &mut self,
        file: P,
        offset: Option<u64>,
        sizelimit: Option<u64>,
    ) -> Result<()> {
        trace!(
            "setup: on '{}' entered with '{}', offset: {}, size limit: {}",
            self.path.display(),
            file.as_ref().display(),
            offset.is_some(),
            sizelimit.is_some()
        );

        if self.file.is_some() {
            return Err(Error::with_context(
                ErrorKind::InvState,
                &format!("setup: device for is in use: '{}'", self.path.display()),
            ));
        }

        let abs_file = file
            .as_ref()
            .canonicalize()
            .upstream_with_context(&format!(
                "setup: Failed to canonicalize path '{}'",
                file.as_ref().display()
            ))?;

        let file_fd = Fd::open(&abs_file, O_RDWR)?;

        let offset = if let Some(offset) = offset { offset } else { 0 };

        let sizelimit = if let Some(sizelimit) = sizelimit {
            sizelimit
        } else {
            0
        };

        debug!("setup: calling IOCTL_LOOP_SET_FD",);

        let ioctl_res = unsafe { ioctl(self.fd.get_fd(), IOCTL_LOOP_SET_FD, file_fd.get_fd()) };
        if ioctl_res == 0 {
            // TODO: possibly initialze LoopInfo using get_loop_info_for_index
            debug!(
                "setup: offset: calling setinfo with file: '{}', offset: 0x{:x}, sizelimit: 0x{:x}",
                abs_file.display(),
                offset,
                sizelimit
            );

            self.set_info(&abs_file, offset, sizelimit)?;
            self.file = Some(abs_file);
            // TODO: cleanup (unset file)
            Ok(())
        } else {
            Err(Error::with_context(
                ErrorKind::Upstream,
                &format!(
                    "setup: ioctrl IOCTL_LOOP_SET_FD failed on device '{}' with error {}",
                    self.path.display(),
                    io::Error::last_os_error()
                ),
            ))
        }
    }

    pub fn modify_offset(&mut self, offset: u64, sizelimit: u64) -> Result<()> {
        if let Some(file) = &self.file {
            Ok(self.set_info(file, offset, sizelimit)?)
        } else {
            Err(Error::with_context(
                ErrorKind::InvState,
                &format!(
                    "modify_offset: Device has no associated file: '{}'",
                    self.path.display()
                ),
            ))
        }
    }

    pub fn unset(&mut self) -> Result<()> {
        let ioctl_res = unsafe { ioctl(self.fd.get_fd(), IOCTL_LOOP_CLR_FD) };
        if ioctl_res == 0 {
            self.file = None;
            Ok(())
        } else {
            Err(Error::with_context(
                ErrorKind::Upstream,
                &format!(
                    "unset: Failed to reset loop device '{}', error: {}",
                    self.path.display(),
                    io::Error::last_os_error()
                ),
            ))
        }
    }

    pub fn get_loop_info(&self) -> Result<LoopInfo64> {
        let loop_info: LoopInfo64 = unsafe { MaybeUninit::zeroed().assume_init() };
        let ioctl_res = unsafe { ioctl(self.fd.get_fd(), IOCTL_LOOP_GET_STATUS_64, &loop_info) };
        if ioctl_res == 0 {
            Ok(loop_info)
        } else if errno() == ENXIO {
            Err(Error::with_context(
                ErrorKind::DeviceNotFound,
                &format!(
                    "get_loop_info: Not a valid loop device: '{}'",
                    self.path.display()
                ),
            ))
        } else {
            Err(Error::with_context(
                ErrorKind::Upstream,
                &format!(
                    "get_loop_info: ioctl IOCTL_LOOP_GET_STATUS_64 on '{}' failed with error {}",
                    self.path.display(),
                    io::Error::last_os_error()
                ),
            ))
        }
    }

    #[allow(dead_code)]
    pub fn get_loop_infos() -> Result<Vec<LoopInfo64>> {
        let mut loop_infos: Vec<LoopInfo64> = Vec::new();
        for loop_idx in 0..MAX_LOOP {
            if PathBuf::from(&format!("/dev/loop{}", loop_idx)).exists() {
                match LoopDevice::from_index(loop_idx, false) {
                    Ok(loop_dev) => {
                        if loop_dev.file.is_some() {
                            loop_infos.push(loop_dev.get_loop_info()?);
                        }
                    }
                    Err(why) => {
                        if why.kind() == ErrorKind::FileNotFound {
                            continue;
                        } else {
                            return Err(Error::from_upstream(
                                Box::new(why),
                                &format!(
                                    "get_loop_infos: Failed to open loop device for index {}",
                                    loop_idx
                                ),
                            ));
                        }
                    }
                }
            }
        }
        Ok(loop_infos)
    }
}

impl Drop for LoopDevice {
    fn drop(&mut self) {
        if self.unset && self.file.is_some() {
            let _res = self.unset();
        }
    }
}

fn path_to_cbuffer<P: AsRef<Path>>(path: P, buffer: &mut [u8]) -> Result<()> {
    let c_string = path_to_cstring(path)?;
    let src = c_string.to_bytes_with_nul();
    if src.len() > buffer.len() {
        return Err(Error::with_context(
            ErrorKind::InvParam,
            "path_to_cbuffer: insufficient target buffer size",
        ));
    }
    buffer[0..src.len()].copy_from_slice(src);
    Ok(())
}

fn cbuffer_to_pathbuf(buffer: &[u8]) -> PathBuf {
    let mut num_chars = buffer.len();
    for (idx, el) in buffer.iter().enumerate() {
        if *el == 0 {
            num_chars = idx;
            break;
        }
    }

    PathBuf::from(OsStr::from_bytes(&buffer[0..num_chars]))
}

/// check if buffer contains a valid c_string
#[allow(dead_code)]
fn check_str_buffer(buffer: &[u8]) -> bool {
    for el in buffer {
        if *el == 0 {
            return true;
        }
    }
    false
}

pub(crate) fn key_to_string(key: &[u8], max_len: Option<usize>) -> String {
    let max_len = if let Some(max_len) = max_len {
        min(max_len, key.len())
    } else {
        key.len()
    };

    let mut res = String::new();
    for val in 0..max_len {
        res.push_str(&format!("{:02x}", val))
    }
    res
}
