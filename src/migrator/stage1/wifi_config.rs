use failure::{Fail, ResultExt};
use log::{debug, info, trace, warn};
use regex::Regex;
use std::fs::{read_dir, read_to_string, File};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

#[cfg(target_os = "windows")]
use crate::common::call;

use crate::common::{dir_exists, path_append, pidof, MigErrCtx, MigError, MigErrorKind};

mod wpa_parser;
use wpa_parser::WpaParser;

mod nwmgr_parser;
use crate::stage1::wifi_config::nwmgr_parser::replace_nwmgr_id;
use nwmgr_parser::parse_nwmgr_config;

pub const BALENA_FILE_TAG: &str = "## created by balena-migrate";
//const NWM_CONFIG_DIR: &str = "/etc/NetworkManager/system-connections/";
const CONNMGR_CONFIG_DIR: &str = "/var/lib/connman";

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
    pub fn scan(ssid_filter: &[String]) -> Result<Vec<WifiConfig>, MigError> {
        trace!("WifiConfig::scan: entered with {:?}", ssid_filter);
        if !pidof("NetworkManager")?.is_empty() {
            Ok(parse_nwmgr_config(ssid_filter)?)
        } else if !pidof("wpa_supplicant")?.is_empty() {
            Ok(WpaParser::parse_config(ssid_filter)?)
        } else if !pidof("connman")?.is_empty() {
            Ok(WifiConfig::from_connman(ssid_filter)?)
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

    fn parse_conmgr_file(file_path: &Path) -> Result<Option<WifiConfig>, MigError> {
        let mut ssid = String::from("");
        let mut psk: Option<String> = None;

        let skip_re = Regex::new(r##"^(\s*#.*|\s*)$"##).unwrap();
        let param_re = Regex::new(r#"^\s*(\S+)\s*=\s*(\S+)\s*$"#).unwrap();
        let file = File::open(file_path).context(MigErrCtx::from_remark(
            MigErrorKind::Upstream,
            &format!("failed to open file {}", file_path.display()),
        ))?;

        for line in BufReader::new(file).lines() {
            match line {
                Ok(line) => {
                    if skip_re.is_match(&line) {
                        debug!("parse_conmgr_file: skipping line: '{}'", line);
                        continue;
                    }

                    debug!("parse_conmgr_file: processing line '{}'", line);

                    if let Some(captures) = param_re.captures(&line) {
                        let param = captures.get(1).unwrap().as_str();
                        let value = captures.get(2).unwrap().as_str();

                        if param == "Name" {
                            ssid = String::from(value);
                            continue;
                        }

                        if param == "Passphrase" {
                            psk = Some(String::from(value));
                            continue;
                        }
                    }

                    debug!("ignoring line '{}' from '{}'", line, file_path.display());
                    continue;
                }
                Err(why) => {
                    return Err(MigError::from(why.context(MigErrCtx::from_remark(
                        MigErrorKind::Upstream,
                        &format!("unexpected read error from {}", file_path.display()),
                    ))));
                }
            }
        }

        if !ssid.is_empty() {
            Ok(Some(WifiConfig::Params(Params { ssid, psk })))
        } else {
            Ok(None)
        }
    }

    fn from_connman(ssid_filter: &[String]) -> Result<Vec<WifiConfig>, MigError> {
        let mut wifis: Vec<WifiConfig> = Vec::new();

        if dir_exists(CONNMGR_CONFIG_DIR)? {
            let paths = read_dir(CONNMGR_CONFIG_DIR).context(MigErrCtx::from_remark(
                MigErrorKind::Upstream,
                &format!("Failed to list directory '{}'", CONNMGR_CONFIG_DIR),
            ))?;

            for path in paths {
                if let Ok(path) = path {
                    let dir_path = path.path();
                    debug!("got path '{}'", dir_path.display());
                    if let Some(dir_name) = dir_path.file_name() {
                        if dir_name.to_string_lossy().starts_with("wifi_")
                            && dir_path.metadata().unwrap().is_dir()
                        {
                            debug!("examining connmgr path '{}'", dir_path.display());
                            let settings_path = path_append(dir_path, "settings");
                            if settings_path.exists() {
                                debug!("examining connmgr path '{}'", settings_path.display());
                                if let Some(wifi) = WifiConfig::parse_conmgr_file(&settings_path)? {
                                    let mut valid = ssid_filter.is_empty();
                                    if !valid {
                                        if let Some(_pos) = ssid_filter
                                            .iter()
                                            .position(|r| r.as_str() == wifi.get_ssid())
                                        {
                                            valid = true;
                                        }
                                    }
                                    if valid {
                                        if let Some(_pos) = wifis
                                            .iter()
                                            .position(|r| r.get_ssid() == wifi.get_ssid())
                                        {
                                            debug!("Network '{}' is already contained in wifi list, skipping duplicate definition", wifi.get_ssid());
                                        } else {
                                            wifis.push(wifi);
                                        }
                                    } else {
                                        info!(
                                            "ignoring wifi config for ssid: '{}'",
                                            wifi.get_ssid()
                                        );
                                    }
                                }
                            }
                        } else {
                            debug!(
                                "no match on '{}' starts_with(wifi_): {} is_dir: {}",
                                dir_path.display(),
                                dir_name.to_string_lossy().starts_with("wifi_"),
                                dir_path.metadata().unwrap().is_dir()
                            );
                        }
                    } else {
                        warn!("Not processing invalid path '{}'", path.path().display());
                    }
                } else {
                    return Err(MigError::from_remark(
                        MigErrorKind::Upstream,
                        &format!(
                            "Error reading entry from directory '{}'",
                            CONNMGR_CONFIG_DIR
                        ),
                    ));
                }
            }
        } else {
            debug!(
                "WifiConfig::from_connman: directory not found: '{}'",
                CONNMGR_CONFIG_DIR
            );
        }

        Ok(wifis)
    }

    pub(crate) fn create_nwmgr_file<P: AsRef<Path>>(
        &self,
        base_path: P,
        index: u64,
    ) -> Result<u64, MigError> {
        let base_path = base_path.as_ref();
        let path = path_append(base_path, &format!("resin-wifi-{}", index));

        info!("Creating NetworkManager file in '{}'", path.display());
        let mut nwmgr_file = File::create(&path).context(MigErrCtx::from_remark(
            MigErrorKind::Upstream,
            &format!("Failed to create file in '{}'", path.display()),
        ))?;

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
                            .context(upstream_context!(&format!(
                                "Failed to read file '{}'",
                                nwmgr_file.file.display()
                            )))?
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
            .context(MigErrCtx::from_remark(
                MigErrorKind::Upstream,
                &format!("failed to write new '{:?}'", path.display()),
            ))?;
        Ok(index)
    }
}
