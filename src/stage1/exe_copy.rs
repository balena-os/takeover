use crate::common::system::{is_lnk, lstat, stat};
use crate::common::{dir_exists, path_append, whereis, Error, ErrorKind, Result, ToError};

use lddtree::DependencyAnalyzer;
use log::{debug, error, info, trace};
use std::collections::HashSet;
use std::fs::{copy, create_dir, create_dir_all, read_link};
use std::path::{Path, PathBuf};

/// Copies a list of executables for takeover tooling to a provided directory,
/// including the dependent libraries. We expect takeover will pivot the provided
/// directory to be the root directory, and so the executables must run in that
/// context.
pub(crate) struct ExeCopy {
    req_space: u64,
    libraries: HashSet<String>,
    executables: HashSet<String>,
    /// If true, don't create a top level "lib" directory when copy_files() because
    /// the root filesystem merges (symlinks) the /lib directory to /usr/lib.
    /// Instead copies to "/usr/lib" in add_lib().
    is_merged_lib: bool,
}

impl ExeCopy {
    pub fn new(cmd_list: Vec<&str>) -> Result<ExeCopy> {
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

        let lib_stat = lstat("/lib").upstream_with_context("Failed to lstat /lib")?;
        debug!("/lib is a symlink? {}", is_lnk(&lib_stat));

        let mut efi_files = ExeCopy {
            req_space: 0,
            libraries: HashSet::new(),
            executables,
            is_merged_lib: is_lnk(&lib_stat),
        };

        efi_files.get_libs_for()?;

        Ok(efi_files)
    }
    pub fn get_req_space(&self) -> u64 {
        self.req_space
    }

    fn get_libs_for(&mut self) -> Result<()> {
        trace!("get_libs_for: entered");
        let mut check_libs: HashSet<String> = HashSet::new();

        for curr_path in &self.executables {
            let stat = stat(curr_path)
                .upstream_with_context(&format!("Failed to stat '{}'", curr_path))?;
            self.req_space += stat.st_size as u64;
            self.get_libs(curr_path, &mut check_libs)?;
        }

        // This goes down the dependency tree and check sub-dependencies
        while !check_libs.is_empty() {
            let mut unchecked_libs: HashSet<String> = HashSet::new();
            for curr in &check_libs {
                self.add_lib(curr)?;
            }
            for curr in &check_libs {
                self.get_libs(curr, &mut unchecked_libs)?;
            }
            check_libs = unchecked_libs;
        }
        Ok(())
    }

    fn add_lib(&mut self, lib_path: &str) -> Result<bool> {
        trace!("add_lib: entered with '{}'", lib_path);

        // If merged /lib + /usr/lib, must divert a target for /lib to /usr/lib.
        let mut save_path = String::from("");
        if self.is_merged_lib && lib_path.starts_with("/lib/") {
            save_path.push_str("/usr");
        }
        save_path.push_str(lib_path);

        match stat(&save_path) {
            Ok(stat) => {
                if self.libraries.insert(save_path.to_owned()) {
                    self.req_space += stat.st_size as u64;
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            Err(why) => Err(Error::with_all(
                ErrorKind::FileNotFound,
                &format!("The file could not be found: '{}'", save_path),
                Box::new(why),
            )),
        }
    }

    fn get_libs(&self, file: &str, found: &mut HashSet<String>) -> Result<()> {
        trace!("get_libs: entered with '{}'", file);
        let analyzer = DependencyAnalyzer::new(PathBuf::from("/"));

        match analyzer.analyze(file) {
            Ok(dependencies) => {
                trace!("Dependency Tree for {file}:\n {:#?}", dependencies);
                for (_libname, library) in dependencies.libraries.iter() {
                    let path = library.path.to_str().unwrap();

                    if !self.libraries.contains(path) {
                        found.insert(path.to_owned());
                    }
                }
                Ok(())
            }
            Err(why) => {
                error!("Error analysing dependency for: '{}': {:?}", file, why);
                Err(Error::with_context(
                    ErrorKind::Upstream,
                    &format!("Error analysing dependency for: '{}': {:?}", file, why),
                ))
            }
        }
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
