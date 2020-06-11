use failure::{Fail, ResultExt};

use std::fmt::{self, Debug, Formatter};
use std::mem::MaybeUninit;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::Duration;

use nix::errno::errno;
use std::cmp::min;
use std::ffi::{CStr, CString};
use std::io;

use std::os::raw::c_int;

use crate::common::mig_error::{MigErrCtx, MigError, MigErrorKind};
use log::debug;

use libc::{close, ioctl, makedev, mknod, open, EAGAIN, ENOENT, ENXIO, O_CLOEXEC, O_RDWR, S_IFBLK};

const MAX_LOOP: u32 = 1024;

const IOCTL_LOOP_SET_FD: u64 = 0x4c00;
const IOCTL_LOOP_CLR_FD: u64 = 0x4c01;
const IOCTL_LOOP_CTL_GET_FREE: u64 = 0x4c82;
const IOCTL_LOOP_GET_STATUS_64: u64 = 0x4c05;
const IOCTL_LOOP_SET_STATUS_64: u64 = 0x4c04;

// from /usr/src/linux/loop.h

const LO_NAME_SIZE: usize = 64;
const LO_KEY_SIZE: usize = 32;

/* prio to kernel 2.6
#[repr(C)]
pub struct LoopInfo {
    lo_number: c_int,
    lo_device: dev_t, // __kernel_old_dev_t
    lo_inode: c_ulong,
    lo_rdevice: dev_t, // __kernel_old_dev_t
    lo_offset: c_int,
    lo_encrypt_type: c_int,
    lo_encrypt_key_size: c_int,
    lo_flags: c_int,
    lo_name: [c_char; LO_NAME_SIZE],
    lo_encrypt_key: [c_uchar; LO_KEY_SIZE],
    lo_init: [c_ulong; 2],
    reserved: [c_char; 4],
}
*/

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
            .field("lo_file_name", &u8_slice_to_string(&self.lo_file_name))
            .field("lo_crypt_name", &u8_slice_to_string(&self.lo_crypt_name))
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
}

impl LoopDevice {
    /// create or open loop device for index

    pub fn from_index(loop_index: u32) -> Result<LoopDevice, MigError> {
        let path = PathBuf::from(&format!("/dev/loop{}", loop_index));

        // try to open device file
        match Fd::open(&path, O_RDWR | O_CLOEXEC) {
            Ok(fd) => {
                // device node exists, check if in use
                let mut loop_dev = LoopDevice {
                    path,
                    fd,
                    file: None,
                };

                match loop_dev.get_loop_info() {
                    Ok(loop_info) => {
                        // valid loop info, grab path
                        loop_dev.file =
                            Some(PathBuf::from(cbuffer_to_string(&loop_info.lo_file_name)))
                    }
                    Err(why) => {
                        if why.kind() != MigErrorKind::DeviceNotFound {
                            return Err(from_upstream!(
                                why,
                                &format!(
                                    "failed to retrieve loop info from device'{}'",
                                    loop_dev.path.display()
                                )
                            ));
                        }
                    }
                }
                Ok(loop_dev)
            }
            Err(why) => {
                // failed to open device
                if why.kind() == MigErrorKind::FileNotFound {
                    // file does not exist - try to create device node
                    let c_path = path_to_cstring(&path)?.into_raw();
                    let res = unsafe { mknod(c_path, S_IFBLK | 0644, makedev(7, loop_index)) };
                    let _c_path = unsafe { CString::from_raw(c_path) };
                    if res == 0 {
                        let fd = Fd::open(&path, O_RDWR | O_CLOEXEC)?;
                        return Ok(LoopDevice {
                            path,
                            fd,
                            file: None,
                        });
                    } else {
                        return Err(MigError::from_remark(
                            MigErrorKind::Upstream,
                            &format!(
                                "Failed to create device node '{}', error {}",
                                path.display(),
                                io::Error::last_os_error().to_string()
                            ),
                        ));
                    }
                } else {
                    // some other error opening device
                    Err(from_upstream!(
                        why,
                        &format!("Failed to open device '{}'", path.display())
                    ))
                }
            }
        }
    }

    /// create a loop device associated with the given file
    #[allow(dead_code)]
    pub fn for_file<P: AsRef<Path>>(
        file: P,
        offset: Option<u64>,
        size_limit: Option<u64>,
        loop_index: Option<u32>,
    ) -> Result<LoopDevice, MigError> {
        let mut loop_device = if let Some(loop_index) = loop_index {
            LoopDevice::from_index(loop_index)?
        } else {
            LoopDevice::get_free()?
        };
        loop_device.setup(file, offset, size_limit)?;
        Ok(loop_device)
    }

    pub fn get_free() -> Result<LoopDevice, MigError> {
        // Find a free loop device, try use /dev/loop-control first
        match Fd::open("/dev/loop-control", O_RDWR | O_CLOEXEC) {
            Ok(file_fd) => {
                let ioctl_res = unsafe { ioctl(file_fd.get_fd(), IOCTL_LOOP_CTL_GET_FREE) };
                if ioctl_res < 0 {
                    Err(MigError::from_remark(
                        MigErrorKind::Upstream,
                        &format!(
                            "ioctl IOCTL_LOOP_CTL_GET_FREE failed with error: {}",
                            io::Error::last_os_error().to_string()
                        ),
                    ))
                } else {
                    // success, return loop number and open device fd
                    Ok(LoopDevice::from_index(ioctl_res as u32)?)
                }
            }
            Err(why) => {
                if why.kind() == MigErrorKind::FileNotFound {
                    // if /dev/loop-control does not exist scan for free devices manually
                    for loop_idx in 0..MAX_LOOP {
                        let loop_dev = LoopDevice::from_index(loop_idx)?;
                        if loop_dev.file.is_some() {
                            // device is in use
                            continue;
                        } else {
                            return Ok(loop_dev);
                        }
                    }
                    Err(MigError::from_remark(
                        MigErrorKind::NotFound,
                        "No free loop device was found",
                    ))
                } else {
                    Err(MigError::from_remark(
                        MigErrorKind::NotFound,
                        "Unable to open /dev/loop-control",
                    ))
                }
            }
        }
    }

    /// retrieve the loop devices path
    pub fn get_path(&self) -> &Path {
        self.path.as_path()
    }

    fn set_info<P: AsRef<Path>>(
        &self,
        file: P,
        offset: u64,
        sizelimit: u64,
    ) -> Result<(), MigError> {
        let mut loop_info: LoopInfo64 = unsafe { MaybeUninit::zeroed().assume_init() };

        path_to_cbuffer(file, &mut loop_info.lo_file_name)?;
        loop_info.lo_offset = offset;
        loop_info.lo_sizelimit = sizelimit;

        let mut retries = 3;
        loop {
            debug!("calling IOCTL_LOOP_SET_STATUS_64, attempt {}", 4 - retries);
            let ioctl_res =
                unsafe { ioctl(self.fd.get_fd(), IOCTL_LOOP_SET_STATUS_64, &loop_info) };
            if ioctl_res == 0 {
                return Ok(());
            } else {
                debug!(
                    "ioctl IOCTL_LOOP_SET_STATUS_64 returned {}",
                    io::Error::last_os_error()
                );
                if errno() == EAGAIN {
                    retries -= 1;
                    if retries > 0 {
                        sleep(Duration::from_millis(100));
                    }
                } else {
                    return Err(MigError::from_remark(
                        MigErrorKind::Upstream,
                        &format!(
                            "ioctl IOCTL_LOOP_SET_STATUS_64 failed on device '{}', error {}",
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
    ) -> Result<(), MigError> {
        if self.file.is_some() {
            return Err(MigError::from_remark(
                MigErrorKind::InvState,
                &format!("setup: device for is in use: '{}'", self.path.display()),
            ));
        }

        let abs_file = file
            .as_ref()
            .canonicalize()
            .context(upstream_context!(&format!(
                "Failed to canonicalize path '{}'",
                file.as_ref().display()
            )))?;

        let file_fd = Fd::open(&abs_file, O_RDWR | O_CLOEXEC)?;

        let offset = if let Some(offset) = offset { offset } else { 0 };

        let sizelimit = if let Some(sizelimit) = sizelimit {
            sizelimit
        } else {
            0
        };

        let ioctl_res = unsafe { ioctl(self.fd.get_fd(), IOCTL_LOOP_SET_FD, file_fd.get_fd()) };
        if ioctl_res == 0 {
            // TODO: possibly initialze LoopInfo using get_loop_info_for_index
            self.set_info(&abs_file, offset, sizelimit)?;
            self.file = Some(abs_file);
            // TODO: cleanup (unset file)
            Ok(())
        } else {
            Err(MigError::from_remark(
                MigErrorKind::Upstream,
                &format!(
                    "ioctrl IOCTL_LOOP_SET_FD failed on device '{}' with error {}",
                    self.path.display(),
                    io::Error::last_os_error().to_string()
                ),
            ))
        }
    }

    pub fn modify_offset(&mut self, offset: u64, sizelimit: u64) -> Result<(), MigError> {
        if let Some(file) = &self.file {
            Ok(self.set_info(file, offset, sizelimit)?)
        } else {
            Err(MigError::from_remark(
                MigErrorKind::InvState,
                &format!("Device has no associated file: '{}'", self.path.display()),
            ))
        }
    }

    pub fn unset(&self) -> Result<(), MigError> {
        let ioctl_res = unsafe { ioctl(self.fd.get_fd(), IOCTL_LOOP_CLR_FD) };
        if ioctl_res == 0 {
            Ok(())
        } else {
            Err(MigError::from_remark(
                MigErrorKind::Upstream,
                &format!(
                    "Failed to reset loop device '{}', error: {}",
                    self.path.display(),
                    io::Error::last_os_error()
                ),
            ))
        }
    }

    pub fn get_loop_info(&self) -> Result<LoopInfo64, MigError> {
        let loop_info: LoopInfo64 = unsafe { MaybeUninit::zeroed().assume_init() };
        let ioctl_res = unsafe { ioctl(self.fd.get_fd(), IOCTL_LOOP_GET_STATUS_64, &loop_info) };
        if ioctl_res == 0 {
            Ok(loop_info)
        } else {
            if errno() == ENXIO {
                Err(MigError::from_remark(
                    MigErrorKind::DeviceNotFound,
                    &format!("Not a valid loop device: '{}'", self.path.display()),
                ))
            } else {
                Err(MigError::from_remark(
                    MigErrorKind::Upstream,
                    &format!(
                        "ioctl IOCTL_LOOP_GET_STATUS_64 on '{}' failed with error {}",
                        self.path.display(),
                        io::Error::last_os_error().to_string()
                    ),
                ))
            }
        }
    }

    #[allow(dead_code)]
    pub fn get_loop_infos() -> Result<Vec<LoopInfo64>, MigError> {
        let mut loop_infos: Vec<LoopInfo64> = Vec::new();
        for loop_idx in 0..MAX_LOOP {
            let loop_dev = match LoopDevice::from_index(loop_idx) {
                Ok(loop_dev) => loop_dev,
                Err(why) => {
                    if why.kind() == MigErrorKind::FileNotFound {
                        continue;
                    } else {
                        return Err(from_upstream!(
                            why,
                            &format!("Failed to open loop device for index {}", loop_idx)
                        ));
                    }
                }
            };
            match loop_dev.get_loop_info() {
                Ok(loop_info) => {
                    loop_infos.push(loop_info);
                }
                Err(why) => match why.kind() {
                    MigErrorKind::DeviceNotFound | MigErrorKind::FileNotFound => {
                        continue;
                    }
                    _ => {
                        return Err(from_upstream!(
                            why,
                            &format!("Failed to open device for index {}", loop_idx)
                        ))
                    }
                },
            }
        }
        Ok(loop_infos)
    }
}

fn path_to_cstring<P: AsRef<Path>>(path: P) -> Result<CString, MigError> {
    Ok(
        CString::new(&*path.as_ref().to_string_lossy()).context(upstream_context!(&format!(
            "Failed to convert path '{}' to c_str",
            path.as_ref().display()
        )))?,
    )
}

fn cbuffer_to_string(buffer: &[u8]) -> String {
    let c_string = buffer.as_ptr() as *const i8;
    let temp = unsafe { CStr::from_ptr(c_string) };
    String::from(&*temp.to_string_lossy())
}

fn path_to_cbuffer<P: AsRef<Path>>(path: P, buffer: &mut [u8]) -> Result<(), MigError> {
    let c_string = path_to_cstring(path)?;
    let src = c_string.to_bytes();
    if src.len() > buffer.len() {
        return Err(MigError::from_remark(
            MigErrorKind::InvParam,
            "path_to_cbuffer: insufficient target buffer size",
        ));
    }
    for idx in 0..src.len() {
        buffer[idx] = src[idx];
    }
    Ok(())
}

struct Fd {
    fd: c_int,
}

impl Fd {
    fn get_fd(&self) -> c_int {
        self.fd
    }
    fn open<P: AsRef<Path>>(file: P, mode: c_int) -> Result<Fd, MigError> {
        let file_name = path_to_cstring(&file)?;
        let fname_ptr = file_name.into_raw();
        let fd = unsafe { open(fname_ptr, mode) };
        let _file_name = unsafe { CString::from_raw(fname_ptr) };
        if fd >= 0 {
            Ok(Fd { fd })
        } else {
            if errno() == ENOENT {
                Err(MigError::from_remark(
                    MigErrorKind::FileNotFound,
                    &format!(
                        "Failed to open file '{}', error {}",
                        file.as_ref().display(),
                        io::Error::last_os_error()
                    ),
                ))
            } else {
                Err(MigError::from_remark(
                    MigErrorKind::Upstream,
                    &format!(
                        "Failed to open file '{}', error {}",
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
        ()
    }
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

pub(crate) fn u8_slice_to_string(str: &[u8]) -> Result<String, MigError> {
    u8_ptr_to_string(str.as_ptr(), Some(str.len()))
}

pub(crate) fn u8_ptr_to_string(str: *const u8, max_len: Option<usize>) -> Result<String, MigError> {
    let max_len = if let Some(max_len) = max_len {
        max_len
    } else {
        256
    };

    let mut res: Vec<u8> = Vec::new();
    for idx in 0..max_len {
        let curr_ptr: *const u8 = unsafe { str.offset(idx as isize) };
        let curr_val = unsafe { *curr_ptr };
        if curr_val == 0 {
            break;
        } else {
            res.push(curr_val);
        }
    }
    let temp = CString::new(res).context(upstream_context!(
        "Failed to create CString from c_char pointer"
    ))?;
    Ok(String::from(&*temp.to_string_lossy()))
}
