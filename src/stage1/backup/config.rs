use crate::common::error::{Result, ToError};

use serde::Deserialize;
use std::fs::read_to_string;
use std::path::Path;

#[derive(Debug, Deserialize)]
pub(crate) struct ItemConfig {
    pub source: String,
    pub target: Option<String>,
    // TODO: filter.allow, filter.deny
    pub filter: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct VolumeConfig {
    pub volume: String,
    pub items: Vec<ItemConfig>,
}

pub(crate) fn backup_cfg_from_file<P: AsRef<Path>>(file: P) -> Result<Vec<VolumeConfig>> {
    Ok(serde_yaml::from_str(
        &read_to_string(file.as_ref()).upstream_with_context(&format!(
            "Failed to read backup configuration from file: '{}'",
            file.as_ref().display()
        ))?,
    )
    .upstream_with_context(&format!(
        "Failed to parse backup configuration from file: '{}'",
        file.as_ref().display()
    ))?)
}
