use crate::{
    common::{dir_exists, path_append, Error, ErrorKind, Result, ToError},
    stage1::wifi_config::{Params, WifiConfig},
};

use std::fs::{read_dir, File};
use std::io::{BufRead, BufReader};
use std::path::Path;

use log::{debug, info, warn};
use regex::Regex;

pub(crate) const CONNMGR_CONFIG_DIR: &str = "/var/lib/connman";

struct ConnMgrParser {
    skip_re: Regex,
    param_re: Regex,
}

impl ConnMgrParser {
    fn new() -> ConnMgrParser {
        ConnMgrParser {
            skip_re: Regex::new(r##"^(\s*#.*|\s*)$"##).unwrap(),
            param_re: Regex::new(r#"^\s*(\S+)\s*=\s*(\S+)\s*$"#).unwrap(),
        }
    }

    fn parse_conmgr_file(&self, file_path: &Path) -> Result<Option<WifiConfig>> {
        let mut ssid = String::from("");
        let mut psk: Option<String> = None;

        let file = File::open(file_path)
            .upstream_with_context(&format!("failed to open file {}", file_path.display()))?;

        for line in BufReader::new(file).lines() {
            match line {
                Ok(line) => {
                    if self.skip_re.is_match(&line) {
                        debug!("parse_conmgr_file: skipping line: '{}'", line);
                        continue;
                    }

                    debug!("parse_conmgr_file: processing line '{}'", line);

                    if let Some(captures) = self.param_re.captures(&line) {
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
                    return Err(Error::from_upstream(
                        Box::new(why),
                        &format!("unexpected read error from {}", file_path.display()),
                    ));
                }
            }
        }

        if !ssid.is_empty() {
            Ok(Some(WifiConfig::Params(Params { ssid, psk })))
        } else {
            Ok(None)
        }
    }
}

pub(crate) fn parse_connmgr_config(ssid_filter: &[String]) -> Result<Vec<WifiConfig>> {
    let mut wifis: Vec<WifiConfig> = Vec::new();

    if dir_exists(CONNMGR_CONFIG_DIR)? {
        let paths = read_dir(CONNMGR_CONFIG_DIR).upstream_with_context(&format!(
            "Failed to list directory '{}'",
            CONNMGR_CONFIG_DIR
        ))?;

        let parser = ConnMgrParser::new();

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
                            if let Some(wifi) = parser.parse_conmgr_file(&settings_path)? {
                                if ssid_filter.is_empty()
                                    || ssid_filter
                                        .iter()
                                        .any(|curr| curr.as_str() == wifi.get_ssid())
                                {
                                    wifis.push(wifi);
                                } else {
                                    info!("ignoring wifi config for ssid: '{}'", wifi.get_ssid());
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
                return Err(Error::with_context(
                    ErrorKind::Upstream,
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
