use libc::S_IFREG;
use log::info;
use nix::mount::{mount, MsFlags};
use rand::distributions::Alphanumeric;
use rand::{thread_rng, Rng};
use std::cmp::min;
use std::io;
use std::path::{Path, PathBuf};

use crate::{
    common::{
        call,
        defs::{MOKUTIL_CMD, NIX_NONE, SYS_EFI_DIR},
        dir_exists,
        system::{mkdir, mknod, uname},
        whereis, Error, ErrorKind, Result, ToError,
    },
    stage1::defs::OSArch,
};

use log::{error, trace, warn};
use regex::Regex;

use crate::common::path_append;
use crate::stage1::migrate_info::MigrateInfo;

use std::fs::create_dir_all;
use std::io::Read;

pub(crate) fn get_os_arch() -> Result<OSArch> {
    trace!("get_os_arch: entered");

    let uname_res = uname()?;
    let machine = uname_res.get_machine();
    match machine {
        "x86_64" => Ok(OSArch::AMD64),
        "i386" => Ok(OSArch::I386),
        "armv7l" => Ok(OSArch::ARMHF),
        "armv6l" => Ok(OSArch::ARMHF),
        "aarch64" => Ok(OSArch::ARM64),
        _ => Err(Error::with_context(
            ErrorKind::InvParam,
            &format!("get_os_arch: unsupported architecture '{}'", machine),
        )),
    }
}

/******************************************************************
 * Try to find out if secure boot is enabled using mokutil
 * assuming secure boot is not enabled if mokutil is absent
 ******************************************************************/

pub(crate) fn is_secure_boot() -> Result<bool> {
    trace!("is_secure_boot: entered");

    // TODO: check for efi vars

    if dir_exists(SYS_EFI_DIR)? {
        let mokutil_path = match whereis(MOKUTIL_CMD) {
            Ok(path) => path,
            Err(_why) => {
                warn!("The mokutil command '{}' could not be found", MOKUTIL_CMD);
                return Ok(false);
            }
        };

        let cmd_res = call(&mokutil_path, &["--sb-state"], true)?;
        if cmd_res.stderr.is_empty() {
            let regex = Regex::new(r"^SecureBoot\s+(disabled|enabled)$").unwrap();
            let lines = cmd_res.stdout.lines();
            for line in lines {
                if let Some(cap) = regex.captures(line) {
                    if cap.get(1).unwrap().as_str() == "enabled" {
                        return Ok(true);
                    } else {
                        return Ok(false);
                    }
                }
            }

            error!(
                "is_secure_boot: failed to parse command output: '{}'",
                cmd_res.stdout
            );
            Err(Error::with_context(
                ErrorKind::InvParam,
                "is_secure_boot: failed to parse command output",
            ))
        } else if cmd_res
            .stderr
            .starts_with("This system doesn't support Secure Boot")
        {
            Ok(false)
        } else {
            Err(Error::with_context(
                ErrorKind::ExecProcess,
                &format!("mokutil returned an error message: '{}'", cmd_res.stderr),
            ))
        }
    } else {
        Ok(false)
    }
}

pub(crate) fn mktemp<P: AsRef<Path>>(
    dir: bool,
    prefix: Option<&str>,
    suffix: Option<&str>,
    path: Option<P>,
) -> Result<PathBuf> {
    loop {
        let mut file_name = String::new();
        if let Some(prefix) = prefix {
            file_name.push_str(prefix);
        }
        file_name.push_str(
            thread_rng()
                .sample_iter(&Alphanumeric)
                .take(10)
                .collect::<String>()
                .as_str(),
        );
        if let Some(suffix) = suffix {
            file_name.push_str(suffix);
        }

        let new_path = if let Some(path) = &path {
            path_append(path.as_ref(), file_name.as_str())
        } else {
            path_append("/tmp", file_name.as_str())
        };

        match if dir {
            mkdir(new_path.as_path(), 0o755)
        } else {
            mknod(new_path.as_path(), S_IFREG | 0o755, 0)
        } {
            Ok(_) => return Ok(new_path),
            Err(why) => {
                if why.kind() != ErrorKind::FileExists {
                    return Err(Error::with_cause(ErrorKind::Upstream, Box::new(why)));
                }
            }
        }
    }
}

pub(crate) fn check_tcp_connect(host: &str, port: u16, timeout: u64) -> Result<()> {
    use std::net::{Shutdown, TcpStream, ToSocketAddrs};
    use std::time::Duration;
    let url = format!("{}:{}", host, port);
    let mut addrs_iter = url.to_socket_addrs().upstream_with_context(&format!(
        "check_tcp_connect: failed to resolve host address: '{}'",
        url
    ))?;

    if let Some(ref sock_addr) = addrs_iter.next() {
        let tcp_stream = TcpStream::connect_timeout(sock_addr, Duration::from_secs(timeout))
            .upstream_with_context(&format!(
                "check_tcp_connect: failed to connect to: '{}' with timeout: {}",
                url, timeout
            ))?;

        let _res = tcp_stream.shutdown(Shutdown::Both);
        Ok(())
    } else {
        Err(Error::with_context(
            ErrorKind::InvState,
            &format!(
                "check_tcp_connect: no results from name resolution for: '{}",
                url
            ),
        ))
    }
}

pub(crate) fn mount_fs<P: AsRef<Path>>(
    mount_dir: P,
    fs: &str,
    fs_type: &str,
    mig_info: Option<&mut MigrateInfo>,
) -> Result<()> {
    let mount_dir = mount_dir.as_ref();
    if !dir_exists(mount_dir)? {
        create_dir_all(mount_dir).upstream_with_context(&format!(
            "Failed to create mount directory '{}'",
            mount_dir.display()
        ))?;
    }

    mount(
        Some(fs.as_bytes()),
        mount_dir,
        Some(fs_type.as_bytes()),
        MsFlags::empty(),
        NIX_NONE,
    )
    .upstream_with_context(&format!(
        "Failed to mount {} on {} with fstype {}",
        fs,
        mount_dir.display(),
        fs_type
    ))?;

    if let Some(mig_info) = mig_info {
        mig_info.add_mount(mount_dir);
    }

    info!("Mounted {} file system on '{}'", fs, mount_dir.display());

    Ok(())
}

pub(crate) struct ReadBuffer<'a> {
    buffer: &'a [u8],
    read_pos: usize,
}

impl<'a> ReadBuffer<'a> {
    pub fn new(buffer: &'a [u8]) -> ReadBuffer {
        ReadBuffer {
            buffer,
            read_pos: 0,
        }
    }
}

impl<'a> Read for ReadBuffer<'a> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.read_pos >= self.buffer.len() {
            Ok(0)
        } else {
            let bytes = min(buf.len(), self.buffer.len() - self.read_pos);
            buf[0..bytes].copy_from_slice(&self.buffer[self.read_pos..self.read_pos + bytes]);
            self.read_pos += bytes;
            Ok(bytes)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::copy;

    #[test]
    fn test_read_buffer() {
        const BUFFER: [u8; 16] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
        let mut read_buffer = ReadBuffer::new(&BUFFER[..]);
        let mut buffer: Vec<u8> = Vec::with_capacity(16);
        copy(&mut read_buffer, &mut buffer).unwrap();
        assert_eq!(&BUFFER[..], buffer.as_slice());
    }
}
