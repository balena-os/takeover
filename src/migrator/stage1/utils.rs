use nix::mount::{mount, MsFlags};
use std::path::{Path, PathBuf};

use log::info;

use crate::{
    common::{
        call,
        defs::{MKTEMP_CMD, MOKUTIL_CMD, NIX_NONE, UNAME_CMD, WHEREIS_CMD},
        dir_exists, file_exists, MigErrCtx, MigError, MigErrorKind,
    },
    stage1::defs::{OSArch, SYS_UEFI_DIR},
};

use log::{error, trace, warn};
use regex::Regex;

use crate::stage1::migrate_info::MigrateInfo;
use failure::ResultExt;
use std::fs::create_dir_all;

pub(crate) fn get_os_arch() -> Result<OSArch, MigError> {
    const UNAME_ARGS_OS_ARCH: [&str; 1] = ["-m"];
    trace!("get_os_arch: entered");
    let cmd_res = call(UNAME_CMD, &UNAME_ARGS_OS_ARCH, true).context(upstream_context!(
        &format!("get_os_arch: call {}", UNAME_CMD)
    ))?;

    if cmd_res.status.success() {
        if cmd_res.stdout.to_lowercase() == "x86_64" {
            Ok(OSArch::AMD64)
        } else if cmd_res.stdout.to_lowercase() == "i386" {
            Ok(OSArch::I386)
        } else if cmd_res.stdout.to_lowercase() == "armv7l" {
            // TODO: try to determine the CPU Architecture
            Ok(OSArch::ARMHF)
        } else {
            Err(MigError::from_remark(
                MigErrorKind::InvParam,
                &format!("get_os_arch: unsupported architectute '{}'", cmd_res.stdout),
            ))
        }
    } else {
        Err(MigError::from_remark(
            MigErrorKind::ExecProcess,
            &format!("get_os_arch: command failed: {} {:?}", UNAME_CMD, cmd_res),
        ))
    }
}

/******************************************************************
 * Try to find out if secure boot is enabled using mokutil
 * assuming secure boot is not enabled if mokutil is absent
 ******************************************************************/

pub(crate) fn is_secure_boot() -> Result<bool, MigError> {
    trace!("is_secure_boot: entered");

    // TODO: check for efi vars

    if dir_exists(SYS_UEFI_DIR)? {
        let mokutil_path = match whereis(MOKUTIL_CMD) {
            Ok(path) => path,
            Err(_why) => {
                warn!("The mokutil command '{}' could not be found", MOKUTIL_CMD);
                return Ok(false);
            }
        };

        let cmd_res = call(&mokutil_path, &["--sb-state"], true)?;

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
        Err(MigError::from_remark(
            MigErrorKind::InvParam,
            &"is_secure_boot: failed to parse command output".to_string(),
        ))
    } else {
        Ok(false)
    }
}

pub(crate) fn whereis(cmd: &str) -> Result<String, MigError> {
    const BIN_DIRS: &[&str] = &["/bin", "/usr/bin", "/sbin", "/usr/sbin"];
    // try manually first
    for path in BIN_DIRS {
        let path = format!("{}/{}", &path, cmd);
        if file_exists(&path) {
            return Ok(path);
        }
    }

    // else try whereis command
    let args: [&str; 2] = ["-b", cmd];
    let cmd_res = match call(WHEREIS_CMD, &args, true) {
        Ok(cmd_res) => cmd_res,
        Err(why) => {
            // manually try the usual suspects
            return Err(MigError::from_remark(
                MigErrorKind::NotFound,
                &format!(
                    "whereis failed to execute for: {:?}, error: {:?}",
                    args, why
                ),
            ));
        }
    };

    if cmd_res.status.success() {
        if cmd_res.stdout.is_empty() {
            Err(MigError::from_remark(
                MigErrorKind::InvParam,
                &format!("whereis: no command output for {}", cmd),
            ))
        } else {
            let mut words = cmd_res.stdout.split(' ');
            if let Some(s) = words.nth(1) {
                Ok(String::from(s))
            } else {
                Err(MigError::from_remark(
                    MigErrorKind::NotFound,
                    &format!("whereis: command not found: '{}'", cmd),
                ))
            }
        }
    } else {
        Err(MigError::from_remark(
            MigErrorKind::ExecProcess,
            &format!(
                "whereis: command failed for {}: {}",
                cmd,
                cmd_res.status.code().unwrap_or(0)
            ),
        ))
    }
}

pub(crate) fn mktemp<P: AsRef<Path>>(
    dir: bool,
    pattern: Option<&str>,
    path: Option<P>,
) -> Result<PathBuf, MigError> {
    let mut cmd_args: Vec<&str> = Vec::new();

    let mut dir_path = String::new();
    if let Some(path) = path {
        dir_path = path.as_ref().to_string_lossy().to_string();
        cmd_args.push("-p");
        cmd_args.push(dir_path.as_str());
    }

    if dir {
        cmd_args.push("-d");
    }

    if let Some(pattern) = pattern {
        cmd_args.push(pattern);
    }

    let cmd_res = call(MKTEMP_CMD, cmd_args.as_slice(), true)?;

    if cmd_res.status.success() {
        Ok(PathBuf::from(cmd_res.stdout))
    } else {
        Err(MigError::from_remark(
            MigErrorKind::ExecProcess,
            &format!(
                "Failed to create temporary file for image extraction, error: {}",
                cmd_res.stderr
            ),
        ))
    }
}

pub(crate) fn check_tcp_connect(host: &str, port: u16, timeout: u64) -> Result<(), MigError> {
    use std::net::{Shutdown, TcpStream, ToSocketAddrs};
    use std::time::Duration;
    let url = format!("{}:{}", host, port);
    let mut addrs_iter = url.to_socket_addrs().context(MigErrCtx::from_remark(
        MigErrorKind::Upstream,
        &format!(
            "check_tcp_connect: failed to resolve host address: '{}'",
            url
        ),
    ))?;

    if let Some(ref sock_addr) = addrs_iter.next() {
        let tcp_stream = TcpStream::connect_timeout(sock_addr, Duration::from_secs(timeout))
            .context(MigErrCtx::from_remark(
                MigErrorKind::Upstream,
                &format!(
                    "check_tcp_connect: failed to connect to: '{}' with timeout: {}",
                    url, timeout
                ),
            ))?;

        let _res = tcp_stream.shutdown(Shutdown::Both);
        Ok(())
    } else {
        Err(MigError::from_remark(
            MigErrorKind::InvState,
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
    mig_info: &mut MigrateInfo,
) -> Result<(), MigError> {
    let mount_dir = mount_dir.as_ref();
    if !dir_exists(mount_dir)? {
        create_dir_all(mount_dir).context(upstream_context!(&format!(
            "Failed to create mount directory '{}'",
            mount_dir.display()
        )))?;
    }

    mount(
        Some(fs.as_bytes()),
        mount_dir,
        Some(fs_type.as_bytes()),
        MsFlags::empty(),
        NIX_NONE,
    )
    .context(upstream_context!(&format!(
        "Failed to mount {} on {} with fstype {}",
        fs,
        mount_dir.display(),
        fs_type
    )))?;

    mig_info.add_mount(mount_dir);

    info!("Mounted {} file system on '{}'", fs, mount_dir.display());

    Ok(())
}
