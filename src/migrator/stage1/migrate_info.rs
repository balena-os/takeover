use std::fs::remove_dir_all;
use std::path::{Path, PathBuf};

use log::warn;
use mod_logger::Level;
use nix::mount::umount;

use crate::{
    common::{get_os_name, mig_error::MigError, options::Options},
    stage1::assets::Assets,
};

#[derive(Debug)]
pub(crate) struct MigrateInfo {
    os_name: String,
    assets: Assets,
    mounts: Vec<PathBuf>,
    to_dir: Option<PathBuf>,
    log_level: Level,
}

#[allow(dead_code)]
impl MigrateInfo {
    pub fn new(opts: &Options) -> Result<MigrateInfo, MigError> {
        Ok(MigrateInfo {
            assets: Assets::new(),
            os_name: get_os_name()?,
            to_dir: None,
            mounts: Vec::new(),
            log_level: if opts.is_trace() {
                Level::Trace
            } else if opts.is_debug() {
                Level::Debug
            } else {
                Level::Info
            },
        })
    }

    pub fn get_assets(&self) -> &Assets {
        &self.assets
    }

    pub fn set_to_dir(&mut self, to_dir: &PathBuf) {
        self.to_dir = Some(to_dir.clone())
    }

    pub fn get_log_level(&self) -> Level {
        self.log_level
    }

    pub fn get_to_dir(&self) -> &Option<PathBuf> {
        &self.to_dir
    }

    pub fn add_mount<P: AsRef<Path>>(&mut self, mount: P) {
        self.mounts.push(mount.as_ref().to_path_buf())
    }

    pub fn get_mounts(&self) -> &Vec<PathBuf> {
        &self.mounts
    }

    pub fn umount_all(&mut self) {
        loop {
            if let Some(mountpoint) = self.mounts.pop() {
                if let Err(why) = umount(&mountpoint) {
                    warn!(
                        "Failed to unmount mountpoint: '{}', error : {:?}",
                        mountpoint.display(),
                        why
                    );
                }
            } else {
                break;
            }
        }

        if let Some(takeover_dir) = &self.to_dir {
            if let Err(why) = remove_dir_all(takeover_dir) {
                warn!(
                    "Failed to remove takeover directory: '{}', error : {:?}",
                    takeover_dir.display(),
                    why
                );
            }
        }
    }
}
