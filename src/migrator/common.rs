use std::fs::{read_link, read_to_string};
use std::mem::MaybeUninit;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

use failure::ResultExt;
use libc::getuid;
use regex::Regex;

use log::{debug, error, info, trace, warn};

pub(crate) mod stage2_config;

pub(crate) mod defs;
use defs::{
    OSArch, DISK_BY_LABEL_PATH, DISK_BY_PARTUUID_PATH, DISK_BY_UUID_PATH, MKTEMP_CMD, UNAME_CMD,
};
#[macro_use]
pub mod mig_error;
pub use mig_error::{MigErrCtx, MigError, MigErrorKind};

pub mod options;
pub use options::Options;

const OS_NAME_REGEX: &str = r#"^PRETTY_NAME="([^"]+)"$"#;
const OS_RELEASE_FILE: &str = "/etc/os-release";

#[derive(Debug)]
pub(crate) struct CmdRes {
    pub stdout: String,
    pub stderr: String,
    pub status: ExitStatus,
}

pub(crate) fn call(cmd: &str, args: &[&str], trim_stdout: bool) -> Result<CmdRes, MigError> {
    trace!("call: '{}' called with {:?}, {}", cmd, args, trim_stdout);

    match Command::new(cmd)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
    {
        Ok(output) => {
            debug!("call: output: {:?}", output);
            Ok(CmdRes {
                stdout: if trim_stdout {
                    String::from(String::from_utf8_lossy(&output.stdout).trim())
                } else {
                    String::from(String::from_utf8_lossy(&output.stdout))
                },
                stderr: String::from(String::from_utf8_lossy(&output.stderr)),
                status: output.status,
            })
        }
        Err(why) => {
            error!("call: output failed: {:?}", why);
            Err(MigError::from_remark(
                MigErrorKind::Upstream,
                &format!("call: failed to execute: command {} '{:?}'", cmd, args),
            ))
        }
    }
}

#[allow(dead_code)]
pub(crate) fn mktemp(
    dir: bool,
    pattern: Option<&str>,
    path: Option<PathBuf>,
) -> Result<PathBuf, MigError> {
    let mut cmd_args: Vec<&str> = Vec::new();

    let mut _dir_path: Option<String> = None;
    if let Some(path) = path {
        _dir_path = Some(String::from(path.to_string_lossy()));
        cmd_args.push("-p");
        cmd_args.push(_dir_path.as_ref().unwrap());
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

pub(crate) fn get_os_arch() -> Result<OSArch, MigError> {
    const UNAME_ARGS_OS_ARCH: [&str; 1] = ["-m"];
    trace!("get_os_arch: entered");
    let cmd_res = call(UNAME_CMD, &UNAME_ARGS_OS_ARCH, true).context(MigErrCtx::from_remark(
        MigErrorKind::Upstream,
        &format!("get_os_arch: call {}", UNAME_CMD),
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

pub(crate) fn get_mem_info() -> Result<(u64, u64), MigError> {
    trace!("get_mem_info: entered");
    // TODO: could add loads, uptime if needed
    let mut s_info: libc::sysinfo = unsafe { MaybeUninit::<libc::sysinfo>::zeroed().assume_init() };
    let res = unsafe { libc::sysinfo(&mut s_info) };
    if res == 0 {
        Ok((s_info.totalram as u64, s_info.freeram as u64))
    } else {
        Err(MigError::from(MigErrorKind::NotImpl))
    }
}

#[allow(dead_code)]
pub(crate) enum DeviceType {
    RPI3(String),
    X86_64(String),
}

#[allow(dead_code)]
pub(crate) fn get_dev_type() -> Result<DeviceType, MigError> {
    let os_arch = get_os_arch()?;
    match os_arch {
        OSArch::ARMHF => {
            const DEVICE_TREE_MODEL: &str = "/proc/device-tree/model";
            const RPI_MODEL_REGEX: &str = r#"^Raspberry\s+Pi\s+(\S+)\s+Model\s+(.*)$"#;

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

            if let Some(captures) = Regex::new(RPI_MODEL_REGEX)
                .unwrap()
                .captures(&dev_tree_model)
            {
                let pitype = captures.get(1).unwrap().as_str();
                let model = captures
                    .get(2)
                    .unwrap()
                    .as_str()
                    .trim_matches(char::from(0));
                debug!(
                    "raspberrypi::is_rpi: selection entered with string: '{}'",
                    pitype
                );

                match pitype {
                    "2" => {
                        info!("Identified RaspberryPi2: model {}", model);
                        Err(MigError::from_remark(MigErrorKind::NotImpl, &format!("Migration is not implemented for os arch {:?}, device tree model: {}", os_arch, dev_tree_model)))
                    }
                    "3" => {
                        info!("Identified RaspberryPi3: model {}", model);
                        Ok(DeviceType::RPI3("raspbeerypi3".to_string()))
                    }
                    "4" => {
                        info!("Identified RaspberryPi4: model {}", model);
                        Err(MigError::from_remark(MigErrorKind::NotImpl, &format!("Migration is not implemented for os arch {:?}, device tree model: {}", os_arch, dev_tree_model)))
                    }
                    _ => {
                        let message = format!("The raspberry pi type reported by your device ('{} {}') is not supported by balena-migrate", pitype, model);
                        error!("{}", message);
                        Err(MigError::from_remark(MigErrorKind::InvParam, &message))
                    }
                }
            } else {
                Err(MigError::from_remark(
                    MigErrorKind::NotImpl,
                    &format!(
                        "Migration is not implemented for os arch {:?}, device tree model: {}",
                        os_arch, dev_tree_model
                    ),
                ))
            }
        }
        OSArch::AMD64 => Ok(DeviceType::X86_64(String::from("intel-nuc"))),
        _ => Err(MigError::from_remark(
            MigErrorKind::NotImpl,
            &format!("Migration is not implemented for os arch {:?}", os_arch),
        )),
    }
}

/******************************************************************
 * Get OS name from /etc/os-release
 ******************************************************************/

pub(crate) fn get_os_name() -> Result<String, MigError> {
    trace!("get_os_name: entered");

    // TODO: implement other source as fallback

    if file_exists(OS_RELEASE_FILE) {
        // TODO: ensure availabilty of method / file exists
        if let Some(os_name) = parse_file(OS_RELEASE_FILE, &Regex::new(OS_NAME_REGEX).unwrap())? {
            Ok(os_name[1].clone())
        } else {
            Err(MigError::from_remark(
                MigErrorKind::NotFound,
                &format!(
                    "get_os_name: could not be located in file {}",
                    OS_RELEASE_FILE
                ),
            ))
        }
    } else {
        Err(MigError::from_remark(
            MigErrorKind::NotFound,
            &format!("get_os_name: could not locate file {}", OS_RELEASE_FILE),
        ))
    }
}

pub(crate) fn is_admin() -> Result<bool, MigError> {
    trace!("is_admin: entered");
    let admin = Some(unsafe { getuid() } == 0);
    Ok(admin.unwrap())
}

pub fn file_exists<P: AsRef<Path>>(file: P) -> bool {
    file.as_ref().exists()
}

#[allow(dead_code)]
pub fn dir_exists<P: AsRef<Path>>(name: P) -> Result<bool, MigError> {
    let path = name.as_ref();
    if path.exists() {
        Ok(name
            .as_ref()
            .metadata()
            .context(MigErrCtx::from_remark(
                MigErrorKind::Upstream,
                &format!(
                    "dir_exists: failed to retrieve metadata for path: '{}'",
                    path.display()
                ),
            ))?
            .file_type()
            .is_dir())
    } else {
        Ok(false)
    }
}

pub(crate) fn parse_file<P: AsRef<Path>>(
    fname: P,
    regex: &Regex,
) -> Result<Option<Vec<String>>, MigError> {
    let path = fname.as_ref();
    let os_info = read_to_string(path).context(MigErrCtx::from_remark(
        MigErrorKind::Upstream,
        &format!("File read '{}'", path.display()),
    ))?;

    for line in os_info.lines() {
        debug!("parse_file: line: '{}'", line);

        if let Some(ref captures) = regex.captures(line) {
            let mut results: Vec<String> = Vec::new();
            for cap in captures.iter() {
                if let Some(cap) = cap {
                    results.push(String::from(cap.as_str()));
                } else {
                    results.push(String::from(""));
                }
            }
            return Ok(Some(results));
        };
    }

    Ok(None)
}

const GIB_SIZE: u64 = 1024 * 1024 * 1024;
const MIB_SIZE: u64 = 1024 * 1024;
const KIB_SIZE: u64 = 1024;

pub fn format_size_with_unit(size: u64) -> String {
    if size > (10 * GIB_SIZE) {
        format!("{} GiB", size / GIB_SIZE)
    } else if size > (10 * MIB_SIZE) {
        format!("{} MiB", size / MIB_SIZE)
    } else if size > (10 * KIB_SIZE) {
        format!("{} KiB", size / KIB_SIZE)
    } else {
        format!("{} B", size)
    }
}

pub fn get_mountpoint<P: AsRef<Path>>(device: P) -> Result<Option<PathBuf>, MigError> {
    let device_str = &*device.as_ref().to_string_lossy();
    let mtab = read_to_string("/etc/mtab").context(MigErrCtx::from_remark(
        MigErrorKind::Upstream,
        "Failed to read /etc/mtab",
    ))?;
    for line in mtab.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        if let Some(device) = words.get(0) {
            if device == &device_str {
                if let Some(mountpoint) = words.get(1) {
                    return Ok(Some(PathBuf::from(mountpoint)));
                } else {
                    return Err(MigError::from_remark(
                        MigErrorKind::InvState,
                        &format!("Encountered invalid line in /etc/mtab '{}'", line),
                    ));
                }
            }
        } else {
            warn!("Encountered empty line in /etc/mtab");
        }
    }
    Ok(None)
}

#[allow(dead_code)]
pub fn get_root_dev() -> Result<Option<PathBuf>, MigError> {
    let mtab = read_to_string("/etc/mtab").context(MigErrCtx::from_remark(
        MigErrorKind::Upstream,
        "Failed to read /etc/mtab",
    ))?;
    for line in mtab.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        if let Some(mountpoint) = words.get(1) {
            if *mountpoint == "/" {
                return Ok(Some(PathBuf::from(&words.get(0).unwrap())));
            }
        } else {
            warn!("Encountered empty line in /etc/mtab");
        }
    }
    Ok(None)
}

pub(crate) fn to_std_device_path(device: &Path) -> Result<PathBuf, MigError> {
    debug!("to_std_device_path: entered with '{}'", device.display());

    if !file_exists(device) {
        return Err(MigError::from_remark(
            MigErrorKind::NotFound,
            &format!("File does not exist: '{}'", device.display()),
        ));
    }

    if !(device.starts_with(DISK_BY_PARTUUID_PATH)
        || device.starts_with(DISK_BY_UUID_PATH)
        || device.starts_with(DISK_BY_LABEL_PATH))
    {
        return Ok(PathBuf::from(device));
    }

    trace!(
        "to_std_device_path: attempting to dereference as link '{}'",
        device.display()
    );

    match read_link(device) {
        Ok(link) => {
            if let Some(parent) = device.parent() {
                let dev_path = path_append(parent, link);
                Ok(dev_path.canonicalize().context(MigErrCtx::from_remark(
                    MigErrorKind::Upstream,
                    &format!("failed to canonicalize path from: '{}'", dev_path.display()),
                ))?)
            } else {
                trace!("Failed to retrieve parent from  '{}'", device.display());
                Ok(PathBuf::from(device))
            }
        }
        Err(why) => {
            trace!(
                "Failed to dereference file '{}' : {:?}",
                device.display(),
                why
            );
            Ok(PathBuf::from(device))
        }
    }
}

pub(crate) fn path_append<P1: AsRef<Path>, P2: AsRef<Path>>(base: P1, append: P2) -> PathBuf {
    let base = base.as_ref();
    let append = append.as_ref();

    if append.is_absolute() {
        let mut components = append.components();
        let mut curr = PathBuf::from(base);
        components.next();
        for comp in components {
            curr = curr.join(comp);
        }
        curr
    } else {
        base.join(append)
    }
}
