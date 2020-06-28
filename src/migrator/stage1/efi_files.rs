use crate::common::{call, dir_exists, path_append, Error, ErrorKind, Result, ToError};
use crate::stage1::utils::whereis;
use lazy_static::lazy_static;
use log::{debug, trace, warn};
use regex::Regex;
use std::collections::HashSet;
use std::fs::{copy, create_dir_all};
use std::path::{Path, PathBuf};

const EFI_DIR: &str = "/sys/firmware/efi";

pub(crate) struct EfiFiles {
    req_space: u64,
    files: HashSet<PathBuf>,
}

impl EfiFiles {
    pub fn new() -> Result<EfiFiles> {
        trace!("new: entered",);
        if dir_exists(EFI_DIR)? {
            let efi_boot_mgr = PathBuf::from(whereis("efibootmgr").error_with_all(
                ErrorKind::FileNotFound,
                &format!("efibootmgr could not be located"),
            )?);

            let mut efi_files = EfiFiles {
                req_space: 0,
                files: HashSet::new(),
            };

            efi_files.get_libs_for(efi_boot_mgr)?;

            Ok(efi_files)
        } else {
            // Nothing to do
            Ok(EfiFiles {
                req_space: 0,
                files: HashSet::new(),
            })
        }
    }
    pub fn get_req_space(&self) -> u64 {
        self.req_space
    }

    fn get_libs_for(&mut self, file: PathBuf) -> Result<()> {
        trace!("get_libs_for: entered with '{}'", file.display());
        if self.add_lib(&file)? {
            let ldd_path =
                whereis("ldd").upstream_with_context("Failed to locate ldd executable")?;
            let mut check_libs = self.get_libs(file.as_path(), ldd_path.as_str())?;
            while !check_libs.is_empty() {
                let mut unchecked_libs: Vec<PathBuf> = Vec::new();
                for curr in &check_libs {
                    self.add_lib(curr.as_path())?;
                }
                for curr in &check_libs {
                    unchecked_libs.append(&mut self.get_libs(curr.as_path(), ldd_path.as_str())?)
                }
                check_libs = unchecked_libs;
            }
        }
        Ok(())
    }

    fn add_lib(&mut self, lib_path: &Path) -> Result<bool> {
        trace!("add_lib: entered with '{}'", lib_path.display());
        if lib_path.exists() {
            if self.files.insert(lib_path.to_path_buf()) {
                self.req_space += lib_path
                    .metadata()
                    .upstream_with_context(&format!(
                        "Failed to get metadata for file: '{}'",
                        lib_path.display()
                    ))?
                    .len();

                Ok(true)
            } else {
                Ok(false)
            }
        } else {
            Err(Error::with_context(
                ErrorKind::FileNotFound,
                &format!("The file could not be found: '{}'", lib_path.display()),
            ))
        }
    }

    fn get_libs(&self, file: &Path, ldd_path: &str) -> Result<Vec<PathBuf>> {
        trace!("get_libs: entered with '{}'", file.display());
        let ldd_res = call_command!(
            ldd_path,
            &[&*file.to_string_lossy()],
            &format!("failed to retrieve dynamic libs for '{}'", file.display())
        )?;

        lazy_static! {
            static ref LIB_REGEX: Regex = Regex::new(
                r#"^\s*(\S+)\s+(=>\s+(\S+)\s+)?(\(0x[0-9,a-f,A-F]+\))|statically linked$"#
            )
            .unwrap();
        }

        let mut result: Vec<PathBuf> = Vec::new();

        for lib_str in ldd_res.lines() {
            if let Some(captures) = LIB_REGEX.captures(lib_str) {
                if let Some(lib_name) = captures.get(1) {
                    let lib_name = lib_name.as_str();
                    if let Some(lib_path) = captures.get(3) {
                        let lib_path = PathBuf::from(lib_path.as_str());
                        if !self.files.contains(lib_path.as_path()) {
                            result.push(lib_path);
                        }
                    } else {
                        let lib_path = PathBuf::from(lib_name);
                        if !self.files.contains(lib_path.as_path()) {
                            if lib_path.is_absolute() && lib_path.exists() {
                                result.push(lib_path);
                            } else {
                                debug!("get_libs: no path for {}", lib_name);
                            }
                        }
                    }
                } else {
                    debug!("get_libs: lib is statically linked: '{}'", file.display());
                }
            } else {
                warn!("get_libs: no match for {}", lib_str);
            }
        }
        Ok(result)
    }

    pub fn copy_files<P: AsRef<Path>>(&self, takeover_dir: P) -> Result<()> {
        let takeover_dir = takeover_dir.as_ref();
        for src_path in &self.files {
            let dest_dir = if let Some(parent) = src_path.parent() {
                path_append(takeover_dir, parent)
            } else {
                takeover_dir.to_path_buf()
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
                "copy_file: copying '{}' to '{}' to ",
                src_path.display(),
                dest_path.display()
            );
            copy(&src_path, &dest_path).upstream_with_context(&format!(
                "Failed toop copy '{}' to '{}'",
                src_path.display(),
                dest_path.display()
            ))?;
        }
        Ok(())
    }
}
