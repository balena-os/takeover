use log::{info, trace, warn};
use std::fs::{read_to_string, File};
use std::io::Write;
use std::path::{Path, PathBuf};

#[cfg(target_os = "windows")]
use crate::common::call;

mod connmgr_parser;
mod nwmgr_parser;
mod wpa_parser;

use crate::{
    common::{dir_exists, file_exists, path_append, pidof, Result, ToError},
    stage1::wifi_config::{
        connmgr_parser::{parse_connmgr_config, CONNMGR_CONFIG_DIR},
        nwmgr_parser::NWMGR_CONFIG_DIR,
        nwmgr_parser::{parse_nwmgr_config, replace_nwmgr_id},
        wpa_parser::{WpaParser, WPA_CONFIG_FILE},
    },
};

pub const BALENA_FILE_TAG: &str = "## created by balena-migrate";
//const NWM_CONFIG_DIR: &str = "/etc/NetworkManager/system-connections/";

const NWMGR_CONTENT: &str = r##"## created by balena-migrate
[connection]
id=__FILE_NAME__
type=wifi

[wifi]
hidden=true
mode=infrastructure
ssid=__SSID__

[ipv4]
method=auto

[ipv6]
addr-gen-mode=stable-privacy
method=auto
"##;

const NWMGR_CONTENT_PSK: &str = r##"[wifi-security]
auth-alg=open
key-mgmt=wpa-psk
psk=__PSK__
"##;

#[derive(Debug)]
pub(crate) struct Params {
    ssid: String,
    psk: Option<String>,
    // TODO: prepare for static config
}

#[derive(Debug)]
pub(crate) struct NwmgrFile {
    ssid: String,
    file: PathBuf,
    // TODO: prepare for static config
}

#[derive(Debug)]
pub(crate) enum WifiConfig {
    Params(Params),
    NwMgrFile(NwmgrFile),
}

impl<'a> WifiConfig {
    pub fn scan(ssid_filter: &[String]) -> Result<Vec<WifiConfig>> {
        trace!("WifiConfig::scan: entered with {:?}", ssid_filter);
        if !pidof("NetworkManager")?.is_empty() && dir_exists(NWMGR_CONFIG_DIR)? {
            Ok(parse_nwmgr_config(ssid_filter)?)
        } else if !pidof("wpa_supplicant")?.is_empty() && file_exists(WPA_CONFIG_FILE) {
            Ok(WpaParser::parse_config(ssid_filter)?)
        } else if !pidof("wpa_supplicant")?.is_empty() && dir_exists(CONNMGR_CONFIG_DIR)? {
            Ok(parse_connmgr_config(ssid_filter)?)
        } else {
            warn!("No supported network managers found, no wifis will be migrated");
            Ok(Vec::new())
        }
    }

    pub fn get_ssid(&'a self) -> &'a str {
        match self {
            WifiConfig::NwMgrFile(file) => &file.ssid,
            WifiConfig::Params(params) => &params.ssid,
        }
    }

    pub(crate) fn create_nwmgr_file<P: AsRef<Path>>(
        &self,
        base_path: P,
        index: u64,
    ) -> Result<u64> {
        let base_path = base_path.as_ref();
        let path = path_append(base_path, format!("resin-wifi-{}", index));

        info!("Creating NetworkManager file in '{}'", path.display());
        let mut nwmgr_file = File::create(&path)
            .upstream_with_context(&format!("Failed to create file in '{}'", path.display()))?;

        let name = path.file_name().unwrap().to_string_lossy();

        let content = match self {
            WifiConfig::Params(config) => {
                let mut content = NWMGR_CONTENT.replace("__SSID__", &config.ssid);
                content = content.replace("__FILE_NAME__", &name);

                if let Some(ref psk) = config.psk {
                    content.push_str(&NWMGR_CONTENT_PSK.replace("__PSK__", psk));
                }
                content
            }
            WifiConfig::NwMgrFile(nwmgr_file) => {
                let mut content = format!("{}\n", BALENA_FILE_TAG);

                content.push_str(
                    replace_nwmgr_id(
                        read_to_string(&nwmgr_file.file)
                            .upstream_with_context(&format!(
                                "Failed to read file '{}'",
                                nwmgr_file.file.display()
                            ))?
                            .as_str(),
                        &name,
                    )?
                    .as_str(),
                );
                content
            }
        };

        trace!("writing nwmgr file as: \n{}", content);

        nwmgr_file
            .write_all(content.as_bytes())
            .upstream_with_context(&format!("failed to write new '{:?}'", path.display()))?;
        Ok(index)
    }
}
