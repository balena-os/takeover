use std::cmp::min;
use std::ffi::{CStr, CString, OsString};
use std::fs::{read_to_string, OpenOptions};
use std::io::Write;
use std::mem::MaybeUninit;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

use log::{debug, error, trace, warn};
use regex::Regex;
use which::which;

pub(crate) mod stage2_config;

pub(crate) mod defs;

pub(crate) mod system;
use system::{is_dir, stat};

pub(crate) mod loop_device;

pub mod error;
pub use error::{Error, ErrorKind, Result, ToError};

pub mod options;
use crate::common::defs::{OLD_ROOT_MP, PIDOF_CMD};

use nix::unistd::sync;
pub use options::Options;

pub(crate) mod debug;
pub(crate) mod disk_util;
pub(crate) mod stream_progress;

const OS_NAME_REGEX: &str = r#"^PRETTY_NAME="([^"]+)"$"#;
const OS_RELEASE_FILE: &str = "/etc/os-release";

#[derive(Debug)]
pub(crate) struct CmdRes {
    pub stdout: String,
    pub stderr: String,
    pub status: ExitStatus,
}

pub(crate) fn call(cmd: &str, args: &[&str], trim_stdout: bool) -> Result<CmdRes> {
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
            error!("call: output failed for command: '{}': {:?}", cmd, why);
            Err(Error::with_context(
                ErrorKind::Upstream,
                &format!("call: failed to execute: command {} '{:?}'", cmd, args),
            ))
        }
    }
}

// We should probably get rid of this function and use `which` directly, but the
// differences in typing between them propagate everywhere. So even if the types
// for `which` are arguably better, we keep this function for now.
pub(crate) fn whereis(cmd: &str) -> Result<String> {
    let actual_cmd = which(cmd);
    match actual_cmd {
        Ok(cmd) => Ok(cmd.to_string_lossy().to_string()),
        Err(why) => Err(Error::from_upstream(
            Box::new(why),
            &format!("'which' failed to find '{}'", cmd),
        )),
    }
}

pub(crate) fn pidof(proc_name: &str) -> Result<Vec<u32>> {
    let cmd_res = call(PIDOF_CMD, &[proc_name], true)?;
    let mut res: Vec<u32> = Vec::new();
    if cmd_res.status.success() {
        for pid in cmd_res.stdout.split_whitespace() {
            res.push(pid.parse::<u32>().upstream_with_context(&format!(
                "pidof: Failed to parse string to u32: '{}'",
                pid
            ))?);
        }
    }
    Ok(res)
}

pub(crate) fn get_mem_info() -> Result<(u64, u64)> {
    trace!("get_mem_info: entered");
    // TODO: could add loads, uptime if needed
    let mut s_info: libc::sysinfo = unsafe { MaybeUninit::<libc::sysinfo>::zeroed().assume_init() };
    let res = unsafe { libc::sysinfo(&mut s_info) };
    if res == 0 {
        // Fields `totalram` and `freeram` are typed either as `u32` or `u64`
        // depending on the platform. We need the conversion for 32-bit
        // architectures, but clippy would complain about it in 64-bit ones.
        // Therefore, we suppress the warning.
        #[allow(clippy::unnecessary_cast)]
        Ok((
            (s_info.totalram as u64) * (s_info.mem_unit as u64),
            (s_info.freeram as u64) * (s_info.mem_unit as u64),
        ))
    } else {
        Err(Error::new(ErrorKind::NotImpl))
    }
}

/******************************************************************
 * Get OS name from /etc/os-release
 ******************************************************************/

pub(crate) fn get_os_name() -> Result<String> {
    trace!("get_os_name: entered");

    // TODO: implement other source as fallback

    if file_exists(OS_RELEASE_FILE) {
        // TODO: ensure availabilty of method / file exists
        if let Some(os_name) = parse_file(OS_RELEASE_FILE, &Regex::new(OS_NAME_REGEX).unwrap())? {
            Ok(os_name[1].clone())
        } else {
            Err(Error::with_context(
                ErrorKind::NotFound,
                &format!(
                    "get_os_name: could not be located in file {}",
                    OS_RELEASE_FILE
                ),
            ))
        }
    } else {
        Err(Error::with_context(
            ErrorKind::NotFound,
            &format!("get_os_name: could not locate file {}", OS_RELEASE_FILE),
        ))
    }
}

pub(crate) fn is_admin() -> Result<bool> {
    trace!("is_admin: entered");
    let admin = unsafe { libc::getuid() } == 0;
    Ok(admin)
}

pub fn file_exists<P: AsRef<Path>>(file: P) -> bool {
    file.as_ref().exists()
}

pub fn dir_exists<P: AsRef<Path>>(name: P) -> Result<bool> {
    match stat(name) {
        Ok(stat_info) => Ok(is_dir(&stat_info)),
        Err(why) => {
            if why.kind() == ErrorKind::FileNotFound {
                Ok(false)
            } else {
                Err(Error::with_cause(ErrorKind::Upstream, Box::new(why)))
            }
        }
    }
}

pub(crate) fn parse_file<P: AsRef<Path>>(fname: P, regex: &Regex) -> Result<Option<Vec<String>>> {
    let path = fname.as_ref();
    let os_info =
        read_to_string(path).upstream_with_context(&format!("File read '{}'", path.display()))?;

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

pub fn get_mountpoint<P: AsRef<Path>>(device: P) -> Result<Option<PathBuf>> {
    let device_str = &*device.as_ref().to_string_lossy();
    let mtab = read_to_string("/etc/mtab").upstream_with_context("Failed to read /etc/mtab")?;
    for line in mtab.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        if let Some(device) = words.first() {
            if device == &device_str {
                if let Some(mountpoint) = words.get(1) {
                    return Ok(Some(PathBuf::from(mountpoint)));
                } else {
                    return Err(Error::with_context(
                        ErrorKind::InvState,
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

pub(crate) fn path_to_cstring<P: AsRef<Path>>(path: P) -> Result<CString> {
    let temp: OsString = path.as_ref().into();
    CString::new(temp.as_bytes()).upstream_with_context(&format!(
        "Failed to convert path to CString: '{}'",
        path.as_ref().display()
    ))
}

#[allow(dead_code)]
pub(crate) unsafe fn hex_dump_ptr_i8(buffer: *const i8, length: isize) -> String {
    hex_dump_ptr_u8(buffer as *const u8, length)
}

pub(crate) unsafe fn hex_dump_ptr_u8(buffer: *const u8, length: isize) -> String {
    let mut idx = 0;
    let mut output = String::new();
    while idx < length {
        output.push_str(&format!("0x{:08x}: ", idx));
        for _ in 0..min(length - idx, 16) {
            let byte: u8 = *buffer.offset(idx);
            let char: char =
                if byte.is_ascii_alphanumeric() || byte.is_ascii_punctuation() || byte == 32 {
                    char::from(byte)
                } else {
                    '.'
                };
            output.push_str(&format!("{:02x} {}  ", byte, char));
            idx += 1;
        }
        output.push('\n');
    }
    output
}

pub(crate) fn hex_dump(buffer: &[u8]) -> String {
    unsafe { hex_dump_ptr_u8(buffer as *const [u8] as *const u8, buffer.len() as isize) }
}

cfg_if::cfg_if! {
    if #[cfg(any(target_arch = "x86_64", target_arch = "x86"))] {
        pub(crate) fn string_from_c_string(c_string: &[i8]) -> Result<String> {
            let mut len: Option<usize> = None;
            for (idx, char) in c_string.iter().enumerate() {
                if *char == 0 {
                    len = Some(idx);
                    break;
                }
            }
            if let Some(len) = len {
                let u8_str = &c_string[0..=len] as *const [i8] as *const [u8] as *const CStr;
                unsafe { Ok(String::from(&*(*u8_str).to_string_lossy())) }
            } else {
                Err(Error::with_context(
                    ErrorKind::InvParam,
                    "Not a nul terminated C string",
                ))
            }
        }
    } else {
        pub(crate) fn string_from_c_string(c_string: &[u8]) -> Result<String> {
            let mut len: Option<usize> = None;
            for (idx, char) in c_string.iter().enumerate() {
                if *char == 0 {
                    len = Some(idx);
                    break;
                }
            }
            if let Some(len) = len {
                let u8_str = &c_string[0..=len] as *const [u8] as *const CStr;
                unsafe { Ok(String::from(&*(*u8_str).to_string_lossy())) }
            } else {
                Err(Error::with_context(
                    ErrorKind::InvParam,
                    "Not a nul terminated C string",
                ))
            }
        }
    }
}

#[allow(dead_code)]
pub(crate) fn log(text: &str) {
    let log_path = if let Ok(stat) = stat(OLD_ROOT_MP) {
        if is_dir(&stat) {
            path_append(OLD_ROOT_MP, "balena-takeover.log")
        } else {
            PathBuf::from("/balena-takeover.log")
        }
    } else {
        PathBuf::from("/balena-takeover.log")
    };
    if let Ok(mut log_file) = OpenOptions::new().create(true).append(true).open(log_path) {
        let _res = writeln!(log_file, "{}", text);
        let _res = log_file.flush();
        sync()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_path_to_cstring() {
        const PATH: &str = "/bla/blub";
        let c_path = path_to_cstring(PATH).unwrap();
        assert_eq!(&*c_path.to_string_lossy(), PATH);
    }
}
