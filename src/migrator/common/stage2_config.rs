use std::path::PathBuf;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use mod_logger::Level;
use log::warn;
use serde_yaml;
use failure::ResultExt;

use crate::{
    common::{MigError, MigErrCtx, MigErrorKind}
};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub(crate) struct Stage2Config {
    pub log_level: String,
    pub log_dev: Option<PathBuf>,
    pub flash_dev: PathBuf,
    pub pretend: bool,
    pub umount_parts: Vec<PathBuf>,
}

impl Stage2Config {
    pub fn get_log_level(&self) -> Level {
        match Level::from_str(&self.log_level) {
            Ok(level) => level,
            Err(why) => {
                warn!("Failed to read error level from stage2 config, error: {:?}", why);
                Level::Info
            }
        }
    }

    pub fn get_log_dev(&self) -> &Option<PathBuf> {
        &self.log_dev
    }

    pub fn serialize(&self) -> Result<String, MigError> {
        Ok(serde_yaml::to_string(self)
            .context(MigErrCtx::from_remark(MigErrorKind::Upstream, "Failed to deserialize stage2 config"))?)
    }

    pub  fn deserialze(config_str: &str) -> Result<Stage2Config, MigError> {
        Ok(
            serde_yaml::from_str(&config_str).context(MigErrCtx::from_remark(
                MigErrorKind::Upstream,
                "Failed to parse stage2 config",
            ))?,
        )
    }

    pub fn get_flash_dev(&self) -> &PathBuf {
        &self.flash_dev
    }

}
