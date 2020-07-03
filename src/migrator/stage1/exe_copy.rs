use crate::common::system::stat;
use crate::common::{call, dir_exists, path_append, Error, ErrorKind, Result, ToError};
use crate::stage1::utils::whereis;
use lazy_static::lazy_static;
use log::{debug, trace, warn};
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::fs::{copy, create_dir_all};
use std::path::{Path, PathBuf};

pub(crate) struct ExeCopy {
    req_space: u64,
    files: HashSet<String>,
    exe_path: HashMap<String, String>,
}

impl ExeCopy {
    pub fn new(cmd_list: Vec<&str>) -> Result<ExeCopy> {
        trace!("new: entered with {:?}", cmd_list);

        let mut exe_path: HashMap<String, String> = HashMap::new();
        for command in cmd_list {
            exe_path.insert(
                command.to_owned(),
                whereis(&command).error_with_all(
                    ErrorKind::FileNotFound,
                    &format!("Command '{}' could not be located", command),
                )?,
            );
        }

        let mut efi_files = ExeCopy {
            req_space: 0,
            files: HashSet::new(),
            exe_path,
        };

        efi_files.get_libs_for()?;

        Ok(efi_files)
    }
    pub fn get_req_space(&self) -> u64 {
        self.req_space
    }

    fn get_libs_for(&mut self) -> Result<()> {
        trace!("get_libs_for: entered with '{:?}'", self.exe_path);
        let ldd_path = whereis("ldd").upstream_with_context("Failed to locate ldd executable")?;
        let mut check_libs: HashSet<String> = HashSet::new();
        for (_, curr_path) in &self.exe_path {
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
                if self.files.insert(lib_path.to_owned()) {
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
        let ldd_res = call_command!(
            ldd_path,
            &[file],
            &format!("failed to retrieve dynamic libs for '{}'", file)
        )?;

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
                        if !self.files.contains(lib_path) {
                            found.insert(lib_path.to_owned());
                        }
                    } else {
                        let lib_path = PathBuf::from(lib_name);
                        if !self.files.contains(lib_name) {
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

    pub fn get_exec_paths(self) -> HashMap<String, String> {
        self.exe_path
    }

    fn copy_file<P1: AsRef<Path>, P2: AsRef<Path>>(src_path: P1, takeover_dir: P2) -> Result<()> {
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
        copy(&src_path, &dest_path).upstream_with_context(&format!(
            "Failed toop copy '{}' to '{}'",
            src_path.display(),
            dest_path.display()
        ))?;

        Ok(())
    }

    pub fn copy_files<P: AsRef<Path>>(&self, takeover_dir: P) -> Result<()> {
        let takeover_dir = takeover_dir.as_ref();

        for (_, file) in &self.exe_path {
            ExeCopy::copy_file(file, takeover_dir)?;
        }

        for src_path in &self.files {
            ExeCopy::copy_file(src_path, takeover_dir)?;
        }
        Ok(())
    }
}
