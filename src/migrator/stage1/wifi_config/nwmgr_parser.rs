use log::{debug, error, warn};
use regex::Regex;
use std::fs::{read_dir, read_to_string};
use std::path::Path;

use crate::stage1::wifi_config::NwmgrFile;
use crate::{
    common::{dir_exists, Error, ErrorKind, Result, ToError},
    stage1::wifi_config::WifiConfig,
};

pub(crate) const NWMGR_CONFIG_DIR: &str = "/etc/NetworkManager/system-connections";

#[derive(Debug, PartialEq, Clone)]
enum NwMgrSection {
    Connection,
    Wifi,
    Other,
}

enum ParseResult {
    TermFound,
    TermNoWifi,
    Continue,
}

struct ParserState {
    skip_re: Regex,
    section_re: Regex,
    param_re: Regex,
    // id_re: Regex,
    section: NwMgrSection,
    is_wifi: bool,
    ssid: Option<String>,
    line: usize,
}

impl ParserState {
    fn new() -> ParserState {
        ParserState {
            skip_re: Regex::new(r##"^(\s*#.*|\s*)$"##).unwrap(),
            section_re: Regex::new(r##"^\s*\[([^]]+)]"##).unwrap(),
            param_re: Regex::new(r##"^\s*([^= #]+)\s*=\s*(\S.*)$"##).unwrap(),
            // id_re: Regex::new(r##"^\s*id\s*=.*"##).unwrap(),
            section: NwMgrSection::Other,
            line: 0,
            is_wifi: false,
            ssid: None,
        }
    }

    fn reset(&mut self) {
        self.ssid = None;
        self.line = 0;
        self.is_wifi = false;
        self.section = NwMgrSection::Other;
    }

    fn is_valid_ssid(ssid: &str, ssid_filter: &[String]) -> bool {
        ssid_filter.is_empty() || ssid_filter.iter().any(|curr| curr.as_str() == ssid)
    }

    fn parse_file<P: AsRef<Path>>(
        &mut self,
        cfg_file: P,
        ssid_filter: &[String],
        wifis: &mut Vec<WifiConfig>,
    ) -> Result<()> {
        let cfg_file = cfg_file.as_ref();
        if cfg_file.is_file() {
            for line in read_to_string(cfg_file)
                .upstream_with_context(&format!("failed to read file: '{}'", cfg_file.display()))?
                .lines()
            {
                match self.parse_line(line) {
                    ParseResult::Continue => {
                        continue;
                    }
                    ParseResult::TermFound => {
                        if let Some(ssid) = self.ssid.take() {
                            if ParserState::is_valid_ssid(ssid.as_str(), ssid_filter) {
                                wifis.push(WifiConfig::NwMgrFile(NwmgrFile {
                                    ssid,
                                    file: cfg_file.to_path_buf(),
                                }));
                            }
                            return Ok(());
                        }
                    }
                    ParseResult::TermNoWifi => {
                        return Ok(());
                    }
                }
            }

            if self.is_wifi && self.ssid.is_some() {}
        }
        Ok(())
    }

    fn is_id_line(&mut self, line: &str) -> bool {
        if let Some(captures) = self.section_re.captures(line) {
            self.section = match captures.get(1).unwrap().as_str() {
                "connection" => NwMgrSection::Connection,
                _ => NwMgrSection::Other,
            };
            false
        } else if self.section == NwMgrSection::Connection {
            if let Some(captures) = self.param_re.captures(line) {
                captures.get(1).unwrap().as_str() == "id"
            } else {
                false
            }
        } else {
            false
        }
    }

    fn parse_line(&mut self, line: &str) -> ParseResult {
        self.line += 1;

        if self.skip_re.is_match(line) {
            ParseResult::Continue
        } else if let Some(captures) = self.section_re.captures(line) {
            self.section = match captures.get(1).unwrap().as_str() {
                "connection" => NwMgrSection::Connection,
                "wifi" => NwMgrSection::Wifi,
                _ => NwMgrSection::Other,
            };
            ParseResult::Continue
        } else if let Some(captures) = self.param_re.captures(line) {
            let param = captures.get(1).unwrap().as_str();
            let value = captures.get(2).unwrap().as_str();

            match self.section {
                NwMgrSection::Connection => {
                    if param == "type" {
                        if value == "wifi" {
                            self.is_wifi = true;
                            if self.ssid.is_some() {
                                ParseResult::TermFound
                            } else {
                                ParseResult::Continue
                            }
                        } else {
                            ParseResult::TermNoWifi
                        }
                    } else {
                        ParseResult::Continue
                    }
                }
                NwMgrSection::Wifi => {
                    if param == "ssid" {
                        debug!("Found ssid: '{}'", value);
                        self.ssid = Some(String::from(value));
                        if self.is_wifi {
                            ParseResult::TermFound
                        } else {
                            ParseResult::Continue
                        }
                    } else {
                        ParseResult::Continue
                    }
                }
                NwMgrSection::Other => ParseResult::Continue,
            }
        } else {
            warn!("Ignoring line: '{}'", line);
            ParseResult::Continue
        }
    }
}

pub(crate) fn replace_nwmgr_id(content: &str, id: &str) -> Result<String> {
    let mut res = String::new();
    let mut parser = ParserState::new();
    let mut found = false;
    for line in content.lines() {
        if !found && parser.is_id_line(line) {
            res.push_str(&format!("id={}\n", id));
            found = true;
            continue;
        }
        res.push_str(&format!("{}\n", line));
    }
    if found {
        Ok(res)
    } else {
        Err(Error::with_context(
            ErrorKind::InvState,
            "No NetworkManager connection Id found",
        ))
    }
}

pub(crate) fn parse_nwmgr_config(ssid_filter: &[String]) -> Result<Vec<WifiConfig>> {
    if dir_exists(NWMGR_CONFIG_DIR)? {
        let mut wifis: Vec<WifiConfig> = Vec::new();
        let paths = read_dir(NWMGR_CONFIG_DIR)
            .upstream_with_context(&format!("Failed to list directory '{}'", NWMGR_CONFIG_DIR))?;

        let mut parser = ParserState::new();
        for dir_entry in paths {
            match dir_entry {
                Ok(dir_entry) => {
                    parser.parse_file(dir_entry.path(), ssid_filter, &mut wifis)?;
                    parser.reset();
                }
                Err(why) => {
                    error!(
                        "Failed to read directory entry of '{}', error: {:?}",
                        NWMGR_CONFIG_DIR, why
                    );
                    return Err(Error::displayed());
                }
            }
        }
        Ok(wifis)
    } else {
        error!(
            "Network manager configuration directory could not be found: '{}'",
            NWMGR_CONFIG_DIR
        );
        Err(Error::displayed())
    }
}
