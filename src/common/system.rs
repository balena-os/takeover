use libc::{
    self, ino_t, mode_t, utsname, EACCES, EEXIST, ENOENT, ENXIO, EPERM, EIO, O_RDONLY, S_IFBLK, S_IFCHR,
    S_IFDIR, S_IFIFO, S_IFLNK, S_IFMT, S_IFREG, S_IFSOCK,
};

use std::collections::HashMap;
use std::fs::{copy, read_to_string, ReadDir};
use std::fs::{read_dir, read_link};
use std::io;
use std::mem::MaybeUninit;
use std::path::{Path, PathBuf};

use lazy_static::lazy_static;
use log::{debug, trace, warn};
use nix::errno::errno;
use regex::Regex;

use crate::common::{
    dir_exists,
    error::{Error, ErrorKind, Result, ToError},
    path_append, path_to_cstring, string_from_c_string,
};

pub(crate) mod fd;
use fd::Fd;
use std::thread::sleep;
use std::time::Duration;

pub(crate) fn is_lnk(stat: &libc::stat) -> bool {
    (stat.st_mode & S_IFMT) == S_IFLNK
}
pub(crate) fn is_reg(stat: &libc::stat) -> bool {
    (stat.st_mode & S_IFMT) == S_IFREG
}
pub(crate) fn is_dir(stat: &libc::stat) -> bool {
    (stat.st_mode & S_IFMT) == S_IFDIR
}
pub(crate) fn is_chr(stat: &libc::stat) -> bool {
    (stat.st_mode & S_IFMT) == S_IFCHR
}
pub(crate) fn is_blk(stat: &libc::stat) -> bool {
    (stat.st_mode & S_IFMT) == S_IFBLK
}
pub(crate) fn is_fifo(stat: &libc::stat) -> bool {
    (stat.st_mode & S_IFMT) == S_IFIFO
}
pub(crate) fn is_sock(stat: &libc::stat) -> bool {
    (stat.st_mode & S_IFMT) == S_IFSOCK
}

fn sys_error(message: &str) -> Error {
    let error_kind = match errno() {
        EPERM => ErrorKind::NotPermitted,
        EACCES => ErrorKind::Permission,
        ENOENT => ErrorKind::FileNotFound,
        ENXIO => ErrorKind::DeviceNotFound,
        EEXIST => ErrorKind::FileExists,
        EIO=> ErrorKind::CmdIo,
        _ => ErrorKind::Upstream,
    };
    Error::with_all(error_kind, message, Box::new(io::Error::last_os_error()))
}

pub(crate) struct UtsName {
    sysname: String,    /* Operating system name (e.g., "Linux") */
    nodename: String,   /* Name within "some implementation-defined network" */
    release: String,    /* Operating system release (e.g., "2.6.28") */
    version: String,    /* Operating system version */
    machine: String,    /* Hardware identifier */
    domainname: String, /* NIS or YP domain name */
}

impl UtsName {
    #[allow(dead_code)]
    pub fn get_sysname(&self) -> &str {
        self.sysname.as_str()
    }
    #[allow(dead_code)]
    pub fn get_nodename(&self) -> &str {
        self.nodename.as_str()
    }
    #[allow(dead_code)]
    pub fn get_release(&self) -> &str {
        self.release.as_str()
    }
    #[allow(dead_code)]
    pub fn get_version(&self) -> &str {
        self.version.as_str()
    }
    pub fn get_machine(&self) -> &str {
        self.machine.as_str()
    }

    #[allow(dead_code)]
    pub fn get_domainname(&self) -> &str {
        self.domainname.as_str()
    }
}

struct ProcessIterator {
    read_dir: ReadDir,
}

impl ProcessIterator {
    pub fn new() -> Result<ProcessIterator> {
        Ok(ProcessIterator {
            read_dir: read_dir("/proc")
                .upstream_with_context("Failed to read directory '/proc'")?,
        })
    }
}

impl Iterator for ProcessIterator {
    type Item = Result<(i32, PathBuf)>;

    fn next(&mut self) -> Option<Self::Item> {
        lazy_static! {
            static ref DIR_REGEX: Regex = Regex::new(r"^/proc/(\d+)$").unwrap();
        }
        loop {
            if let Some(dir_entry) = self.read_dir.next() {
                match dir_entry {
                    Ok(dir_entry) => {
                        let curr_path = dir_entry.path();
                        if let Some(captures) = DIR_REGEX.captures(&*curr_path.to_string_lossy()) {
                            let pid_str = captures.get(1).unwrap().as_str();
                            match pid_str.parse::<i32>() {
                                Ok(pid) => return Some(Ok((pid, curr_path))),
                                Err(why) => {
                                    return Some(Err(Error::from_upstream(
                                        Box::new(why),
                                        &format!(
                                            "Failed to parse pid from path '{}'",
                                            curr_path.display()
                                        ),
                                    )))
                                }
                            }
                        }
                    }
                    Err(why) => {
                        return Some(Err(Error::from_upstream(
                            Box::new(why),
                            "Failed to read directory entry from '/proc'",
                        )))
                    }
                }
            } else {
                return None;
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct ProcessInfo {
    process_id: i32,
    status: HashMap<String, String>,
    executable: Option<PathBuf>,
    root: PathBuf,
}

impl ProcessInfo {
    pub fn process_id(&self) -> i32 {
        self.process_id
    }
    pub fn executable(&self) -> Option<&Path> {
        if let Some(executable) = &self.executable {
            Some(executable.as_path())
        } else {
            None
        }
    }
    pub fn status(&self) -> &HashMap<String, String> {
        &self.status
    }

    #[allow(dead_code)]
    pub fn root(&self) -> &Path {
        &self.root
    }
}

fn parse_status(base_path: &Path) -> Result<HashMap<String, String>> {
    lazy_static! {
        static ref STATUS_REGEX: Regex = Regex::new(r"^\s*([^:]+):\s+(.*)$").unwrap();
    }

    let status_path = path_append(base_path, "status");
    match read_to_string(&status_path) {
        Ok(file_content) => {
            let mut result: HashMap<String, String> = HashMap::new();
            for line in file_content.lines() {
                if let Some(captures) = STATUS_REGEX.captures(line) {
                    let _res = result.insert(
                        captures.get(1).unwrap().as_str().to_owned(),
                        captures.get(2).unwrap().as_str().to_owned(),
                    );
                } else {
                    warn!("parse_status: no match on '{}'", line);
                }
            }
            Ok(result)
        }
        Err(why) => Err(Error::from_upstream(
            Box::new(why),
            &format!("Failed to read process status: '{}'", status_path.display()),
        )),
    }
}

pub(crate) fn get_process_info_for(pid: i32, directory: Option<&Path>) -> Result<ProcessInfo> {
    let proc_dir;
    let directory = if let Some(directory) = directory {
        directory
    } else {
        proc_dir = PathBuf::from(&format!("/proc/{}", pid));
        &proc_dir
    };

    let exec_path = path_append(&directory, "exe");
    let executable = match read_link(&exec_path) {
        Ok(link_path) => Some(link_path),
        Err(why) => {
            if why.kind() == io::ErrorKind::NotFound {
                None
            } else {
                return Err(Error::from_upstream(
                    Box::new(why),
                    &format!(
                        "Failed to read link to executable: '{}'",
                        exec_path.display()
                    ),
                ));
            }
        }
    };
    let root_path = path_append(&directory, "root");
    let root = match read_link(&root_path) {
        Ok(link_path) => link_path,
        Err(why) => {
            return Err(Error::from_upstream(
                Box::new(why),
                &format!(
                    "Failed to read link to executable: '{}'",
                    exec_path.display()
                ),
            ));
        }
    };

    Ok(ProcessInfo {
        process_id: pid,
        status: parse_status(&directory)?,
        executable,
        root,
    })
}

pub(crate) fn get_process_infos() -> Result<Vec<ProcessInfo>> {
    let mut result: Vec<ProcessInfo> = Vec::new();

    for proc_info in ProcessIterator::new()? {
        match proc_info {
            Ok((pid, directory)) => {
                result.push(get_process_info_for(pid, Some(&directory))?);
            }
            Err(why) => {
                return Err(Error::from_upstream_error(
                    Box::new(why),
                    "Failed to read process info",
                ))
            }
        }
    }

    Ok(result)
}

pub(crate) fn fuser<P: AsRef<Path>>(
    path: P,
    signal: i32,
    wait_for_term: Option<Duration>,
) -> Result<usize> {
    trace!(
        "fuser: entered with '{}', {}",
        path.as_ref().display(),
        signal
    );
    lazy_static! {
        static ref DIR_REGEX: Regex = Regex::new(r"^.*/(\d+)$").unwrap();
    }

    let mut sent_signals: Vec<i32> = Vec::new();

    for proc_info in ProcessIterator::new()? {
        match proc_info {
            Ok((curr_pid, directory)) => {
                let fd_dir = path_append(&directory, "fd");
                for dir_entry in match read_dir(&fd_dir) {
                    Ok(read_dir) => read_dir,
                    Err(why) => {
                        if why.kind() == io::ErrorKind::NotFound {
                            continue;
                        } else {
                            return Err(Error::from_upstream(
                                Box::new(why),
                                &format!("Failed to read directory '{}'", fd_dir.display()),
                            ));
                        }
                    }
                } {
                    match dir_entry {
                        Ok(dir_entry) => {
                            let curr_path = dir_entry.path();
                            if let Some(captures) =
                                DIR_REGEX.captures(&*curr_path.to_string_lossy())
                            {
                                let curr_fd = captures
                                    .get(1)
                                    .unwrap()
                                    .as_str()
                                    .parse::<i32>()
                                    .upstream_with_context(&format!(
                                        "Failed to parse fd from path '{}'",
                                        curr_path.display()
                                    ))?;

                                match lstat(curr_path.as_path()) {
                                    Ok(stat_info) => {
                                        if is_lnk(&stat_info) {
                                            let link_data = read_link(curr_path.as_path())
                                                .upstream_with_context(&format!(
                                                    "Failed to read link '{}'",
                                                    curr_path.display()
                                                ))?;
                                            debug!(
                                                "looking at pid: {}, fd {}, file: '{}' -> '{}'",
                                                curr_pid,
                                                curr_fd,
                                                curr_path.display(),
                                                link_data.display()
                                            );

                                            if link_data.starts_with(path.as_ref()) {
                                                debug!("sending signal {} to {}", signal, curr_pid,);
                                                if unsafe { libc::kill(curr_pid, signal) } != 0 {
                                                    warn!(
                                                        "Failed to send signal {} to pid {}, error: {}",
                                                        signal,
                                                        curr_pid,
                                                        io::Error::last_os_error()
                                                    );
                                                } else {
                                                    sent_signals.push(curr_pid);
                                                }
                                                break;
                                            }
                                        } else {
                                            return Err(Error::with_context(
                                                ErrorKind::InvState,
                                                &format!("file '{}' is not a link", curr_path.display()),
                                            ));
                                        }
                                    }
                                    Err(_) => {
                                        debug!("continue after lstat error");
                                    }
                                }
                            }
                        }
                        Err(why) => {
                            return Err(Error::from_upstream_error(
                                Box::new(why),
                                &format!(
                                    "Failed to read directory entry for '{}'",
                                    fd_dir.display()
                                ),
                            ))
                        }
                    }
                }
            }
            Err(why) => {
                return Err(Error::from_upstream_error(
                    Box::new(why),
                    "Failed to read proc_info entry from '/proc'",
                ))
            }
        }
    }
    if !sent_signals.is_empty() {
        sleep(if let Some(wait_for_term) = wait_for_term {
            wait_for_term
        } else {
            Duration::from_millis(500)
        });
        let mut kill_count = 0;
        for pid in sent_signals {
            if !dir_exists(&format!("/proc/{}", pid))? {
                kill_count += 1;
            } else {
                match read_link(&format!("/proc/{}/exe", pid)) {
                    Ok(exe_path) => {
                        warn!(
                            "process still alive after signal {}, pid: {}, '{}'",
                            signal,
                            pid,
                            exe_path.display()
                        );
                    }
                    Err(_) => {
                        warn!("process still alive after  signal {}, pid: {}", signal, pid);
                    }
                }
            }
        }
        Ok(kill_count)
    } else {
        Ok(0)
    }
}

pub(crate) fn uname() -> Result<UtsName> {
    let mut uts_name: utsname = unsafe { MaybeUninit::zeroed().assume_init() };

    let res = unsafe { libc::uname(&mut uts_name) };

    if res == 0 {
        Ok(UtsName {
            sysname: string_from_c_string(&uts_name.sysname[0..])?,
            nodename: string_from_c_string(&uts_name.nodename[0..])?,
            release: string_from_c_string(&uts_name.release[0..])?,
            version: string_from_c_string(&uts_name.version[0..])?,
            machine: string_from_c_string(&uts_name.machine[0..])?,
            domainname: string_from_c_string(&uts_name.domainname[0..])?,
        })
    } else {
        Err(Error::with_all(
            ErrorKind::Upstream,
            "A call to uname failed",
            Box::new(io::Error::last_os_error()),
        ))
    }
}

pub(crate) fn lstat<P: AsRef<Path>>(path: P) -> Result<libc::stat> {
    let mut file_stat: libc::stat = unsafe { MaybeUninit::zeroed().assume_init() };

    let c_path = path_to_cstring(&path)?;
    cfg_if::cfg_if! {
         if #[cfg(target_arch = "x86_64")] {
            let c_path_ptr = c_path.as_bytes_with_nul() as *const [u8] as *const i8;
         } else {
            let c_path_ptr = c_path.as_bytes_with_nul() as *const [u8] as *const u8;
         }
    }

    let res = unsafe { libc::lstat(c_path_ptr, &mut file_stat) };
    if res == 0 {
        Ok(file_stat)
    } else {
        Err(sys_error(&format!(
            "libc::lstat failed for path: '{}'",
            path.as_ref().display()
        )))
    }
}

pub(crate) fn stat<P: AsRef<Path>>(path: P) -> Result<libc::stat> {
    let mut file_stat: libc::stat = unsafe { MaybeUninit::zeroed().assume_init() };

    let c_path = path_to_cstring(&path)?;

    cfg_if::cfg_if! {
         if #[cfg(target_arch = "x86_64")] {
            let c_path_ptr = c_path.as_bytes_with_nul() as *const [u8] as *const i8;
         } else {
            let c_path_ptr = c_path.as_bytes_with_nul() as *const [u8] as *const u8;
         }
    }

    let res = unsafe { libc::stat(c_path_ptr, &mut file_stat) };
    if res == 0 {
        Ok(file_stat)
    } else {
        Err(sys_error(&format!(
            "libc::stat failed for path: '{}'",
            path.as_ref().display()
        )))
    }
}

pub(crate) fn mkfifo<P: AsRef<Path>>(path: P, mode: u32) -> Result<()> {
    let c_path = path_to_cstring(&path)?;
    cfg_if::cfg_if! {
         if #[cfg(target_arch = "x86_64")] {
            let c_path_ptr = c_path.as_bytes_with_nul() as *const [u8] as *const i8;
         } else {
            let c_path_ptr = c_path.as_bytes_with_nul() as *const [u8] as *const u8;
         }
    }

    let res = unsafe { libc::mkfifo(c_path_ptr, mode) };
    if res == 0 {
        Ok(())
    } else {
        Err(sys_error(&format!(
            "libc::mkfifo failed for path: '{}'",
            path.as_ref().display()
        )))
    }
}

pub(crate) fn mknod<P: AsRef<Path>>(path: P, mode: u32, dev_id: u64) -> Result<()> {
    let c_path = path_to_cstring(&path)?;

    cfg_if::cfg_if! {
         if #[cfg(target_arch = "x86_64")] {
            let c_path_ptr = c_path.as_bytes_with_nul() as *const [u8] as *const i8;
         } else {
            let c_path_ptr = c_path.as_bytes_with_nul() as *const [u8] as *const u8;
         }
    }

    let res = unsafe { libc::mknod(c_path_ptr, mode, dev_id) };
    if res == 0 {
        Ok(())
    } else {
        Err(sys_error(&format!(
            "libc::mknod failed for path: '{}'",
            path.as_ref().display()
        )))
    }
}

pub(crate) fn link<P1: AsRef<Path>, P2: AsRef<Path>>(old_file: P1, new_file: P2) -> Result<()> {
    let old_c_path = path_to_cstring(&old_file)?;

    let new_c_path = path_to_cstring(&new_file)?;

    cfg_if::cfg_if! {
         if #[cfg(target_arch = "x86_64")] {
            let old_c_path_ptr = old_c_path.as_bytes_with_nul() as *const [u8] as *const i8;
            let new_c_path_ptr = new_c_path.as_bytes_with_nul() as *const [u8] as *const i8;
         } else {
            let old_c_path_ptr = old_c_path.as_bytes_with_nul() as *const [u8] as *const u8;
            let new_c_path_ptr = new_c_path.as_bytes_with_nul() as *const [u8] as *const u8;
         }
    }

    let res = unsafe { libc::link(old_c_path_ptr, new_c_path_ptr) };
    if res == 0 {
        Ok(())
    } else {
        Err(sys_error(&format!(
            "libc::link failed for path: '{}', '{}'",
            old_file.as_ref().display(),
            new_file.as_ref().display()
        )))
    }
}

pub(crate) fn symlink<P1: AsRef<Path>, P2: AsRef<Path>>(source: P1, dest: P2) -> Result<()> {

    let src_c_path = path_to_cstring(&source)?;
    let dest_c_path = path_to_cstring(&dest)?;

    cfg_if::cfg_if! {
         if #[cfg(target_arch = "x86_64")] {
            let src_c_path_ptr = src_c_path.as_bytes_with_nul() as *const [u8] as *const i8;
            let dest_c_path_ptr = dest_c_path.as_bytes_with_nul() as *const [u8] as *const i8;
         } else {
            let src_c_path_ptr = src_c_path.as_bytes_with_nul() as *const [u8] as *const u8;
            let dest_c_path_ptr = dest_c_path.as_bytes_with_nul() as *const [u8] as *const u8;
         }
    }

    let res = unsafe { libc::symlink(src_c_path_ptr, dest_c_path_ptr) };
    if res == 0 {
        Ok(())
    } else {
        Err(sys_error(&format!(
            "libc::symlink failed for path: '{}', '{}'",
            source.as_ref().display(),
            dest.as_ref().display()
        )))
    }
}

pub(crate) fn mkdir<P: AsRef<Path>>(path: P, mode: u32) -> Result<()> {
    debug!("mkdir: '{}'", path.as_ref().display());
    let c_path = path_to_cstring(&path)?;
    cfg_if::cfg_if! {
         if #[cfg(target_arch = "x86_64")] {
            let c_path_ptr = c_path.as_bytes_with_nul() as *const [u8] as *const i8;
         } else {
            let c_path_ptr = c_path.as_bytes_with_nul() as *const [u8] as *const u8;
         }
    }

    let res = unsafe { libc::mkdir(c_path_ptr, mode) };
    if res == 0 {
        Ok(())
    } else {
        Err(sys_error(&format!(
            "libc::mkdir failed for path: '{}'",
            path.as_ref().display()
        )))
    }
}

#[allow(dead_code)]
pub(crate) fn chmod<P: AsRef<Path>>(file_name: P, mode: mode_t) -> Result<()> {
    let fd = Fd::open(file_name.as_ref(), O_RDONLY)?;
    let res = unsafe { libc::fchmod(fd.get_fd(), mode) };
    if res == 0 {
        Ok(())
    } else {
        Err(sys_error(&format!(
            "fchmod failed on file '{}'",
            file_name.as_ref().display()
        )))
    }
}

enum CopyInodes {
    SameFs,
    SeparateFs(HashMap<ino_t, PathBuf>),
}

fn recursive_copy(source: &Path, dest: &Path, inode_list: &mut CopyInodes) -> Result<()> {
    trace!(
        "recursive_copy: '{}' -> '{}', {}",
        source.display(),
        dest.display(),
        if let CopyInodes::SameFs = inode_list {
            "SameFs"
        } else {
            "SeparateFs"
        }
    );
    for dir_entry in read_dir(source).upstream_with_context(&format!(
        "Failed to read directory contents for '{}'",
        source.display()
    ))? {
        match dir_entry {
            Ok(dir_entry) => {
                let curr_src = dir_entry.path();
                debug!("************* looking at file: '{}'", curr_src.display());
                let stat_info = lstat(&curr_src)?;

                let curr_dest = if let Some(file_name) = curr_src.file_name() {
                    dest.join(file_name)
                } else {
                    return Err(Error::with_context(
                        ErrorKind::Upstream,
                        &format!("Failed to extract filename from '{}'", curr_src.display()),
                    ));
                };
                debug!("destination path is '{}'", curr_dest.display());

                let insert = if !is_dir(&stat_info) && (stat_info.st_nlink > 1) {
                    // this is a hard link
                    debug!("File has hard links: {}", stat_info.st_nlink);
                    match inode_list {
                        CopyInodes::SameFs => {
                            link(&curr_src, &curr_dest)?;
                            continue;
                        }
                        CopyInodes::SeparateFs(ref inode_list) => {
                            if let Some(last_path) = inode_list.get(&stat_info.st_ino) {
                                debug!("found last path: '{}'", last_path.display());
                                match link(last_path.as_path(), &curr_dest) {
                                    Ok(_) => {
                                        continue;
                                    }
                                    Err(why) => {
                                        if why.kind() == ErrorKind::NotPermitted {
                                            // FS might not support hard links, try to copy instead
                                            false
                                        } else {
                                            return Err(why);
                                        }
                                    }
                                }
                            } else {
                                true
                            }
                        }
                    }
                } else {
                    false
                };

                debug!("insert: {}", insert);

                if is_lnk(&stat_info) {
                    debug!("it's a symbolic link");
                    let lnk_dest = read_link(curr_src).upstream_with_context(&format!(
                        "Failed to read link '{}'",
                        source.display()
                    ))?;
                    symlink(lnk_dest.as_path(), &curr_dest)?;
                } else if is_dir(&stat_info) {
                    mkdir(&curr_dest, stat_info.st_mode & 0xFFFF)?;
                    recursive_copy(&curr_src, &curr_dest, inode_list)?;
                } else if is_fifo(&stat_info) {
                    mkfifo(&curr_dest, stat_info.st_mode & 0xFFFF)?;
                } else if is_blk(&stat_info) || is_chr(&stat_info) {
                    mknod(&curr_dest, stat_info.st_mode, stat_info.st_rdev)?;
                } else if is_sock(&stat_info) {
                    warn!("File '{}' is a socket - not copying", &curr_src.display());
                    continue;
                } else if is_reg(&stat_info) {
                    copy(&curr_src, &curr_dest).upstream_with_context(&format!(
                        "Failed to copy '{}' to '{}'",
                        curr_src.display(),
                        curr_dest.display()
                    ))?;
                } else {
                    return Err(Error::with_context(
                        ErrorKind::InvParam,
                        &format!(
                            "Encountered invalid source file mode 0x{:08x} for  '{}'",
                            stat_info.st_mode,
                            source.display()
                        ),
                    ));
                }
                if insert {
                    match inode_list {
                        CopyInodes::SameFs => {
                            return Err(Error::with_context(
                                ErrorKind::InvState,
                                "Trying to register inode in SameFs copy",
                            ))
                        }
                        CopyInodes::SeparateFs(ref mut unpacked) => {
                            debug!(
                                "inserting inode 0x{:08x} for '{}'",
                                stat_info.st_ino,
                                curr_dest.display()
                            );
                            let _res = unpacked.insert(stat_info.st_ino, curr_dest);
                        }
                    }
                }
                // let _res = inode_registry.insert(stat_info.st_ino, dest_path);
            }
            Err(why) => {
                return Err(Error::with_all(
                    ErrorKind::Upstream,
                    &format!(
                        "Failed to read directory entry for path: '{}'",
                        source.display()
                    ),
                    Box::new(why),
                ))
            }
        }
    }

    Ok(())
}

pub(crate) fn copy_dir<P1: AsRef<Path>, P2: AsRef<Path>>(source: P1, dest: P2) -> Result<()> {
    let source = source.as_ref();
    let dest = dest.as_ref();
    debug!("copy_files: '{}' -> '{}'", source.display(), dest.display(),);

    let source_stat = stat(source)?;
    if is_dir(&source_stat) {
        let dest_stat = stat(dest)?;
        if is_dir(&dest_stat) {
            debug!(
                "devices: 0x{:08x}, 0x{:08x}",
                source_stat.st_dev, dest_stat.st_dev
            );
            if source_stat.st_dev == dest_stat.st_dev {
                Ok(recursive_copy(source, dest, &mut CopyInodes::SameFs)?)
            } else {
                Ok(recursive_copy(
                    source,
                    dest,
                    &mut CopyInodes::SeparateFs(HashMap::new()),
                )?)
            }
        } else {
            Err(Error::with_context(
                ErrorKind::InvParam,
                &format!("Dest '{}' is not a directory", dest.display()),
            ))
        }
    } else {
        Err(Error::with_context(
            ErrorKind::InvParam,
            &format!("Source '{}' is not a directory", source.display()),
        ))
    }
}
