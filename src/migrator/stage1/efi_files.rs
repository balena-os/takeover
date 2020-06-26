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
            let ldd_path =
                whereis("ldd").upstream_with_context("Failed to locate ldd executable")?;

            let mut efi_files = EfiFiles {
                req_space: 0,
                files: HashSet::new(),
            };

            efi_files.get_libs_for(efi_boot_mgr, ldd_path.as_str())?;

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

    fn get_libs_for(&mut self, file: PathBuf, ldd_path: &str) -> Result<()> {
        if file.exists() {
            if self.files.insert(file.clone()) {
                self.req_space += file
                    .metadata()
                    .upstream_with_context(&format!(
                        "Failed to get metadata for file: '{}'",
                        file.display()
                    ))?
                    .len();
                let ldd_res = call_command!(
                    ldd_path,
                    &[&*file.to_string_lossy()],
                    &format!("failed to retrieve dynamic libs for '{}'", file.display())
                )?;

                lazy_static! {
                    static ref LIB_REGEX: Regex =
                        Regex::new(r#"^\s*(\S+)\s+(=>\s+(\S+)\s+)?(\(0x[0-9,a-f,A-F]+\))$"#)
                            .unwrap();
                }

                for lib_str in ldd_res.lines() {
                    if let Some(captures) = LIB_REGEX.captures(lib_str) {
                        let lib_name = captures.get(1).unwrap().as_str();
                        if let Some(lib_path) = captures.get(3) {
                            self.get_libs_for(PathBuf::from(lib_path.as_str()), ldd_path)?;
                        } else {
                            debug!("setup_efi: no path for {}", lib_name);
                        }
                    } else {
                        warn!("setup_efi: no match for {}", lib_str);
                    }
                }
                Ok(())
            } else {
                // already processed
                Ok(())
            }
        } else {
            Err(Error::with_context(
                ErrorKind::FileNotFound,
                &format!("File could not be found: '{}'", file.display()),
            ))
        }
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
