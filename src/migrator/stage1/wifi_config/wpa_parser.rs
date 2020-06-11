use crate::{
    common::{MigErrCtx, MigError, MigErrorKind},
    stage1::wifi_config::{Params, WifiConfig},
};

use regex::Regex;

use log::{debug, info, trace, warn};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use crate::common::file_exists;
use failure::{Fail, ResultExt};

pub(crate) const WPA_CONFIG_FILE: &str = "/etc/wpa_supplicant/wpa_supplicant.conf";

#[derive(Debug, PartialEq, Clone)]
enum WpaState {
    Init,
    Network,
}

pub(crate) struct WpaParser<'a> {
    ssid_filter: &'a [String],
    skip_re: Regex,
    net_start_re: Regex,
    net_param1_re: Regex,
    net_param2_re: Regex,
    net_end_re: Regex,
    state: WpaState,
    last_state: WpaState,
    ssid: Option<String>,
    psk: Option<String>,
}

impl<'a> WpaParser<'a> {
    pub fn parse_config(ssid_filter: &[String]) -> Result<Vec<WifiConfig>, MigError> {
        trace!("from_wpa: entered with {:?}", ssid_filter);

        if file_exists(WPA_CONFIG_FILE) {
            debug!("parse_config: scanning '{}'", WPA_CONFIG_FILE);

            let mut parser = WpaParser::new(ssid_filter);
            parser.parse_file(WPA_CONFIG_FILE)
        } else {
            debug!("parse_config: file not found: '{}'", WPA_CONFIG_FILE);
            Err(MigError::displayed())
        }
    }

    pub fn new(ssid_filter: &'a [String]) -> WpaParser {
        WpaParser {
            ssid_filter,
            skip_re: Regex::new(r##"^(\s*#.*|\s*)$"##).unwrap(),
            net_start_re: Regex::new(r#"^\s*network\s*=\s*\{\s*$"#).unwrap(),
            net_param1_re: Regex::new(r#"^\s*(\S+)\s*=\s*"([^"]+)"\s*$"#).unwrap(),
            net_param2_re: Regex::new(r#"^\s*(\S+)\s*=\s*(\S+)\s*$"#).unwrap(),
            net_end_re: Regex::new(r#"^\s*\}\s*$"#).unwrap(),
            state: WpaState::Init,
            last_state: WpaState::Init,
            ssid: None,
            psk: None,
        }
    }

    pub fn parse_file<P: AsRef<Path>>(&mut self, wpa_path: P) -> Result<Vec<WifiConfig>, MigError> {
        let mut wifis: Vec<WifiConfig> = Vec::new();
        let wpa_path = wpa_path.as_ref();
        let file = File::open(wpa_path).context(MigErrCtx::from_remark(
            MigErrorKind::Upstream,
            &format!("failed to open file {}", wpa_path.display()),
        ))?;

        for line in BufReader::new(file).lines() {
            if self.last_state != self.state {
                debug!("parse_file:  {:?} -> {:?}", self.last_state, self.state);
                self.last_state = self.state.clone()
            }

            match line {
                Ok(line) => {
                    if self.skip_re.is_match(&line) {
                        debug!("skipping line: '{}'", line);
                        continue;
                    }

                    debug!("parse_file: processing line '{}'", line);
                    match self.state {
                        WpaState::Init => {
                            self.in_init_state(&line);
                        }
                        WpaState::Network => self.in_network_state(&line, &mut wifis),
                    }
                }
                Err(why) => {
                    return Err(from_upstream!(
                        why,
                        &format!("unexpected read error from {}", wpa_path.display())
                    ));
                }
            }
        }
        Ok(wifis)
    }

    fn init_state(&mut self) {
        self.state = WpaState::Init;
        self.last_state = WpaState::Init;
        self.ssid = None;
        self.psk = None;
    }

    fn in_init_state(&mut self, line: &str) {
        if self.skip_re.is_match(&line) {
            debug!("skipping line: '{}'", line);
            return;
        }

        if self.net_start_re.is_match(line) {
            self.state = WpaState::Network;
        } else {
            warn!("skipping line '{}' in state {:?} ", &line, self.state);
        }
    }

    fn in_network_state(&mut self, line: &str, wifis: &mut Vec<WifiConfig>) {
        if self.skip_re.is_match(&line) {
            debug!("skipping line: '{}'", line);
            return;
        }

        if self.net_end_re.is_match(line) {
            self.end_network(wifis);
            return;
        }

        let mut captures = self.net_param1_re.captures(&line);
        if captures.is_none() {
            captures = self.net_param2_re.captures(&line)
        }

        if let Some(captures) = captures {
            if !self.set_wpa_param(
                captures.get(1).unwrap().as_str(),
                captures.get(2).unwrap().as_str(),
            ) {
                debug!("in state {:?} ignoring line '{}'", self.state, line);
            }
        } else {
            warn!("in state {:?} ignoring line '{}'", self.state, line);
        }
    }

    fn end_network(&mut self, wifis: &mut Vec<WifiConfig>) {
        debug!("in state {:?} found end of network", self.state);

        if let Some(ssid) = self.ssid.take() {
            // TODO: check if ssid is in filter list

            let mut valid = self.ssid_filter.is_empty();
            if !valid {
                if let Some(_pos) = self.ssid_filter.iter().position(|r| r.as_str() == ssid) {
                    valid = true;
                }
            }

            if valid {
                if let Some(_pos) = wifis.iter().position(|r| r.get_ssid() == ssid) {
                    debug!("Network '{}' is already contained in wifi list, skipping duplicate definition", ssid);
                } else {
                    wifis.push(WifiConfig::Params(Params {
                        ssid,
                        psk: self.psk.take(),
                    }));
                }
            } else {
                info!("ignoring wifi config for ssid: '{}'", ssid);
            }
        } else {
            warn!("empty network config encountered");
        }

        self.init_state();
    }

    fn set_wpa_param(&mut self, param: &str, value: &str) -> bool {
        match param {
            "ssid" => {
                debug!("in state {:?} set ssid to '{}'", self.state, value);
                self.ssid = Some(String::from(value));
                true
            }
            "psk" => {
                debug!("in state {:?} set psk to '{}'", self.state, value);
                self.psk = Some(String::from(value));
                true
            }
            _ => false,
        }
    }
}
