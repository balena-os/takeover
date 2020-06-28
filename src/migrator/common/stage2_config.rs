use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_yaml;

use crate::common::error::{Result, ToError};

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
    pub efi_boot_mgr_path: Option<String>,
}

#[allow(dead_code)]
impl Stage2Config {
    pub fn get_log_dev(&self) -> &Option<PathBuf> {
        &self.log_dev
    }

    pub fn serialize(&self) -> Result<String> {
        Ok(serde_yaml::to_string(self)
            .upstream_with_context("Failed to deserialize stage2 config")?)
    }

    pub fn deserialze(config_str: &str) -> Result<Stage2Config> {
        Ok(serde_yaml::from_str(&config_str)
            .upstream_with_context("Failed to parse stage2 config")?)
    }

    pub fn get_flash_dev(&self) -> &PathBuf {
        &self.flash_dev
    }
}
