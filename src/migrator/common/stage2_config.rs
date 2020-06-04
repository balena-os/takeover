use std::path::PathBuf;

use failure::ResultExt;
use serde::{Deserialize, Serialize};
use serde_yaml;

use crate::common::mig_error::{MigErrCtx, MigError, MigErrorKind};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub(crate) struct UmountPart {
    pub dev_name: PathBuf,
    pub mountpoint: PathBuf,
    pub fs_type: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub(crate) struct Stage2Config {
    pub log_dev: Option<PathBuf>,
    pub flash_dev: PathBuf,
    pub pretend: bool,
    pub umount_parts: Vec<UmountPart>,
    pub work_dir: PathBuf,
    pub image_path: PathBuf,
    pub config_path: PathBuf,
    pub backup_path: Option<PathBuf>,
}

#[allow(dead_code)]
impl Stage2Config {
    pub fn get_log_dev(&self) -> &Option<PathBuf> {
        &self.log_dev
    }

    pub fn serialize(&self) -> Result<String, MigError> {
        Ok(serde_yaml::to_string(self)
            .context(upstream_context!("Failed to deserialize stage2 config"))?)
    }

    pub fn deserialze(config_str: &str) -> Result<Stage2Config, MigError> {
        Ok(serde_yaml::from_str(&config_str)
            .context(upstream_context!("Failed to parse stage2 config"))?)
    }

    pub fn get_flash_dev(&self) -> &PathBuf {
        &self.flash_dev
    }
}
