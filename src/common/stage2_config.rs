use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::common::error::{Result, ToError};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub(crate) struct UmountPart {
    pub dev_name: PathBuf,
    pub mountpoint: PathBuf,
    pub fs_type: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub(crate) struct LogDevice {
    pub dev_name: PathBuf,
    pub fs_type: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub(crate) struct Stage2Config {
    pub log_dev: Option<LogDevice>,
    pub log_level: String,
    pub flash_dev: PathBuf,
    pub pretend: bool,
    pub umount_parts: Vec<UmountPart>,
    pub work_dir: PathBuf,
    pub image_path: PathBuf,
    pub boot0_image_path: PathBuf,
    pub boot0_image_dev: PathBuf,
    pub config_path: PathBuf,
    pub backup_path: Option<PathBuf>,
    pub device_type: String,
    pub tty: PathBuf,
}

#[allow(dead_code)]
impl Stage2Config {
    pub fn log_dev(&self) -> Option<&LogDevice> {
        if let Some(log_device) = &self.log_dev {
            Some(log_device)
        } else {
            None
        }
    }

    pub fn serialize(&self) -> Result<String> {
        serde_yaml::to_string(self).upstream_with_context("Failed to deserialize stage2 config")
    }

    pub fn deserialze(config_str: &str) -> Result<Stage2Config> {
        serde_yaml::from_str(config_str).upstream_with_context("Failed to parse stage2 config")
    }

    pub fn flash_dev(&self) -> &PathBuf {
        &self.flash_dev
    }
}
