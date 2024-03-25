use crate::common::system::stat;
use crate::common::{
    call, dir_exists, options::Options, path_append, whereis, Error, ErrorKind, Result, ToError,
};

use lazy_static::lazy_static;
use log::{debug, info, trace, warn};
use nix::NixPath;
use regex::Regex;
use std::collections::HashSet;
use std::fs::{copy, create_dir, create_dir_all, read_link};
use std::path::{Path, PathBuf};

pub(crate) struct ExeCopy {
    req_space: u64,
    libraries: HashSet<String>,
    executables: HashSet<String>,
    ldd_script_path: PathBuf,
}

impl ExeCopy {
    pub fn new(cmd_list: Vec<&str>, opts: &Options) -> Result<ExeCopy> {
        trace!("new: entered with {:?}", cmd_list);

        let mut executables: HashSet<String> = HashSet::new();

        executables.insert(
            read_link("/proc/self/exe")
                .upstream_with_context("Failed to read link to this executable")?
                .to_string_lossy()
                .to_string(),
        );

        for command in cmd_list {
            executables.insert(whereis(command).error_with_all(
                ErrorKind::FileNotFound,
                &format!("Command '{}' could not be located", command),
            )?);
        }

        let mut efi_files = ExeCopy {
            req_space: 0,
            libraries: HashSet::new(),
            executables,
            ldd_script_path: opts.ldd_path(),
        };

        efi_files.get_libs_for()?;

        Ok(efi_files)
    }
    pub fn get_req_space(&self) -> u64 {
        self.req_space
    }

    fn get_libs_for(&mut self) -> Result<()> {
        trace!("get_libs_for: entered");

        let ldd_path: String = if self.ldd_script_path.is_empty() {
            debug!("lld path was not provided, will use the one from current OS, if available");
            whereis("ldd").upstream_with_context(
                "Failed to locate ldd executable, please provide path to ldd manually",
            )?
        } else {
            debug!("Provided ldd path: {}", self.ldd_script_path.display());
            self.ldd_script_path.as_path().display().to_string()
        };
        let mut check_libs: HashSet<String> = HashSet::new();

        // TODO: this_path processing

        for curr_path in &self.executables {
            let stat = stat(curr_path)
                .upstream_with_context(&format!("Failed to stat '{}'", curr_path))?;
            self.req_space += stat.st_size as u64;
            self.get_libs(curr_path, ldd_path.as_str(), &mut check_libs)?;
        }

        while !check_libs.is_empty() {
            let mut unchecked_libs: HashSet<String> = HashSet::new();
            for curr in &check_libs {
                self.add_lib(curr)?;
            }
            for curr in &check_libs {
                self.get_libs(curr, ldd_path.as_str(), &mut unchecked_libs)?;
            }
            check_libs = unchecked_libs;
        }
        Ok(())
    }

    fn add_lib(&mut self, lib_path: &str) -> Result<bool> {
        trace!("add_lib: entered with '{}'", lib_path);
        match stat(lib_path) {
            Ok(stat) => {
                if self.libraries.insert(lib_path.to_owned()) {
                    self.req_space += stat.st_size as u64;
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            Err(why) => Err(Error::with_all(
                ErrorKind::FileNotFound,
                &format!("The file could not be found: '{}'", lib_path),
                Box::new(why),
            )),
        }
    }

    fn get_libs(&self, file: &str, ldd_path: &str, found: &mut HashSet<String>) -> Result<()> {
        trace!("get_libs: entered with '{}'", file);
        let ldd_res = match call(ldd_path, &[file], true) {
            Ok(cmd_res) => {
                if cmd_res.status.success() {
                    cmd_res.stdout
                } else if cmd_res.stderr.contains("not a dynamic executable")
                    || cmd_res.stdout.contains("not a dynamic executable")
                {
                    return Ok(());
                } else {
                    return Err(Error::with_context(
                        ErrorKind::ExecProcess,
                        &format!(
                            "Failed to retrieve dynamic libs for '{}', error: {}",
                            file, cmd_res.stderr
                        ),
                    ));
                }
            }
            Err(why) => {
                return Err(Error::with_all(
                    ErrorKind::ExecProcess,
                    &format!("2failed to retrieve dynamic libs for '{}'", file),
                    Box::new(why),
                ));
            }
        };

        lazy_static! {
            static ref LIB_REGEX: Regex = Regex::new(
                r#"^\s*(\S+)\s+(=>\s+(\S+)\s+)?(\(0x[0-9,a-f,A-F]+\))|statically linked$"#
            )
            .unwrap();
        }

        for lib_str in ldd_res.lines() {
            if let Some(captures) = LIB_REGEX.captures(lib_str) {
                if let Some(lib_name) = captures.get(1) {
                    let lib_name = lib_name.as_str();
                    if let Some(lib_path) = captures.get(3) {
                        let lib_path = lib_path.as_str();
                        if !self.libraries.contains(lib_path) {
                            found.insert(lib_path.to_owned());
                        }
                    } else {
                        let lib_path = PathBuf::from(lib_name);
                        if !self.libraries.contains(lib_name) {
                            if lib_path.is_absolute() && lib_path.exists() {
                                found.insert(lib_name.to_owned());
                            } else {
                                debug!("get_libs: no path for {}", lib_name);
                            }
                        }
                    }
                } else {
                    debug!("get_libs: lib is statically linked: '{}'", file);
                }
            } else {
                warn!("get_libs: no match for {}", lib_str);
            }
        }
        Ok(())
    }

    fn copy_file<P1: AsRef<Path>, P2: AsRef<Path>>(src_path: P1, takeover_dir: P2) -> Result<()> {
        trace!(
            "copy_file: entered with '{}'",
            takeover_dir.as_ref().display()
        );
        let src_path = src_path.as_ref();

        let dest_dir = if let Some(parent) = src_path.parent() {
            path_append(takeover_dir, parent)
        } else {
            takeover_dir.as_ref().to_path_buf()
        };

        if !dir_exists(&dest_dir)? {
            create_dir_all(&dest_dir).upstream_with_context(&format!(
                "Failed to create target directory '{}'",
                dest_dir.display()
            ))?;
        }

        let dest_path = if let Some(name) = src_path.file_name() {
            path_append(dest_dir, name)
        } else {
            return Err(Error::with_context(
                ErrorKind::InvState,
                &format!(
                    "Failed to extract file name from path: {}",
                    src_path.display()
                ),
            ));
        };

        debug!(
            "copy_file: copying '{}' to '{}'",
            src_path.display(),
            dest_path.display()
        );
        copy(src_path, &dest_path).upstream_with_context(&format!(
            "Failed toop copy '{}' to '{}'",
            src_path.display(),
            dest_path.display()
        ))?;

        Ok(())
    }

    pub fn copy_files<P: AsRef<Path>>(&self, takeover_dir: P) -> Result<()> {
        trace!(
            "copy_files: entered with '{}'",
            takeover_dir.as_ref().display()
        );
        let takeover_dir = takeover_dir.as_ref();

        for src_path in &self.libraries {
            ExeCopy::copy_file(src_path, takeover_dir)?;
        }

        let dest_path = path_append(takeover_dir, "/bin");
        if !dest_path.exists() {
            create_dir(&dest_path).upstream_with_context(&format!(
                "Failed to create directory '{}'",
                dest_path.display()
            ))?;
        }

        for file in &self.executables {
            if let Some(file_name) = PathBuf::from(file).file_name() {
                let dest_path = path_append(&dest_path, file_name);
                trace!(
                    "copy_files: copying '{}' to '{}'",
                    &file,
                    dest_path.display()
                );
                copy(file, &dest_path).upstream_with_context(&format!(
                    "Failed to copy '{}' to '{}'",
                    file,
                    dest_path.display()
                ))?;
                info!("Copied '{}' to '{}'", &file, dest_path.display());
            } else {
                return Err(Error::with_context(
                    ErrorKind::InvState,
                    &format!("Failed to retrieve filename from '{}'", file),
                ));
            }
        }

        Ok(())
    }
}
