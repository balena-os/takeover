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
    pub fallback_log: bool,
    pub fallback_log_filename: String,
    pub fallback_log_dirname: String,
    pub flash_dev: PathBuf,
    pub pretend: bool,
    pub umount_parts: Vec<UmountPart>,
    pub work_dir: PathBuf,
    pub image_path: PathBuf,
    pub config_path: PathBuf,
    pub backup_path: Option<PathBuf>,
    pub device_type: String,
    pub tty: PathBuf,
    pub api_endpoint: String,
    pub api_key: String,
    pub uuid: String,
    pub report_hup_progress: bool,
    pub change_dt_to: Option<String>,
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

    /// Remove value for api_key from serialization output. Useful for logging.
    /// Expects input is a multiline string.
    pub fn sanitize_text(serialized: &str) -> String {
        let mut clean_txt = String::new();
        for element in serialized.lines() {
            if element.starts_with("api_key") {
                clean_txt.push_str("api_key: <hidden>");
            } else {
                clean_txt.push_str(element);
            }
            clean_txt.push('\n');
        }
        clean_txt
    }
}
