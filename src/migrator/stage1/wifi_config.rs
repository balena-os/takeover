use failure::{Fail, ResultExt};
use lazy_static::lazy_static;
use log::{debug, info, trace, warn};
use regex::Regex;
use std::fs::{read_dir, read_to_string, File};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

#[cfg(target_os = "windows")]
use crate::common::call;

use crate::common::{
    dir_exists, file_exists, is_migrator_file, path_append, MigErrCtx, MigError, MigErrorKind,
};

pub const BALENA_FILE_TAG: &str = "## created by balena-migrate";
const WPA_CONFIG_FILE: &str = "/etc/wpa_supplicant/wpa_supplicant.conf";
//const NWM_CONFIG_DIR: &str = "/etc/NetworkManager/system-connections/";
const CONNMGR_CONFIG_DIR: &str = "/var/lib/connman";

const NWMGR_CONFIG_DIR: &str = "/etc/NetworkManager/system-connections";
const NWMGR_SECTION_REGEX: &str = r##"^\s*\[([^\]]+)\]"##;

// TODO: can there be a # in a parameter value ?
const NWMGR_PARAM_REGEX: &str = r##"^\s*([^= #]+)\s*=\s*(\S.*)$"##;
const NWMGR_ID_REGEX: &str = r##"^\s*id\s*=.*"##;

const SKIP_REGEX: &str = r##"^(\s*#.*|\s*)$"##;
const WPA_NET_START_REGEX: &str = r#"^\s*network\s*=\s*\{\s*$"#;
const WPA_NET_PARAM1_REGEX: &str = r#"^\s*(\S+)\s*=\s*"([^"]+)"\s*$"#;
const WPA_NET_PARAM2_REGEX: &str = r#"^\s*(\S+)\s*=\s*(\S+)\s*$"#;
const WPA_NET_END_REGEX: &str = r#"^\s*\}\s*$"#;

#[cfg(target_os = "windows")]
const NETSH_USER_PROFILE_REGEX: &str = r#"^[^:]+:\s*((\S.*\S)|("[^"]+"))\s*$"#;

#[cfg(target_os = "windows")]
const NETSH_USER_SECRET_REGEX: &str = r#"^\s+Key\s+Content\s+:\s*((\S.*\S)|("[^"]+"))\s*$"#;

const CONNMGR_PARAM_REGEX: &str = r#"^\s*(\S+)\s*=\s*(\S+)\s*$"#;

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

#[derive(Debug, PartialEq, Clone)]
enum WpaState {
    Init,
    Network,
}

#[derive(Debug, PartialEq, Clone)]
enum NwMgrSection {
    Connection,
    Wifi,
    Other,
}

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
        let mut list: Vec<WifiConfig> = Vec::new();
        WifiConfig::from_wpa(&mut list, ssid_filter)?;
        WifiConfig::from_connman(&mut list, ssid_filter)?;
        WifiConfig::from_nwmgr(&mut list, ssid_filter)?;
        Ok(list)
    }

    pub fn get_ssid(&'a self) -> &'a str {
        match self {
            WifiConfig::NwMgrFile(file) => &file.ssid,
            WifiConfig::Params(params) => &params.ssid,
        }
    }

    #[cfg(target_os = "linux")]
    fn parse_conmgr_file(file_path: &Path) -> Result<Option<WifiConfig>, MigError> {
        let mut ssid = String::from("");
        let mut psk: Option<String> = None;

        let skip_re = Regex::new(SKIP_REGEX).unwrap();
        let param_re = Regex::new(CONNMGR_PARAM_REGEX).unwrap();
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

    #[cfg(target_os = "linux")]
    fn from_connman(wifis: &mut Vec<WifiConfig>, ssid_filter: &[String]) -> Result<(), MigError> {
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

        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[allow(clippy::cognitive_complexity)] //TODO refactor this function to fix the clippy warning
    fn from_wpa(wifis: &mut Vec<WifiConfig>, ssid_filter: &[String]) -> Result<(), MigError> {
        trace!("WifiConfig::from_wpa: entered with {:?}", ssid_filter);

        if file_exists(WPA_CONFIG_FILE) {
            debug!("WifiConfig::from_wpa: scanning '{}'", WPA_CONFIG_FILE);

            lazy_static! {
                static ref SKIP_RE: Regex = Regex::new(SKIP_REGEX).unwrap();
                static ref NET_START_RE: Regex = Regex::new(WPA_NET_START_REGEX).unwrap();
                static ref NET_END_RE: Regex = Regex::new(WPA_NET_END_REGEX).unwrap();
                static ref NET_PARAM1_RE: Regex = Regex::new(WPA_NET_PARAM1_REGEX).unwrap();
                static ref NET_PARAM2_RE: Regex = Regex::new(WPA_NET_PARAM2_REGEX).unwrap();
            }

            let file = File::open(WPA_CONFIG_FILE).context(MigErrCtx::from_remark(
                MigErrorKind::Upstream,
                &format!("failed to open file {}", WPA_CONFIG_FILE),
            ))?;
            let mut state = WpaState::Init;
            let mut last_state = state.clone();
            let mut ssid: Option<String> = None;
            let mut psk: Option<String> = None;

            for line in BufReader::new(file).lines() {
                if last_state != state {
                    debug!("from_wpa:  {:?} -> {:?}", last_state, state);
                    last_state = state.clone()
                }

                match line {
                    Ok(line) => {
                        if SKIP_RE.is_match(&line) {
                            debug!("skipping line: '{}'", line);
                            continue;
                        }

                        debug!("from_wpa: processing line '{}'", line);
                        match state {
                            WpaState::Init => {
                                if NET_START_RE.is_match(&line) {
                                    state = WpaState::Network;
                                } else {
                                    debug!("unexpected line '{}' in state {:?} while parsing file '{}'", &line, state, WPA_CONFIG_FILE);
                                }
                            }
                            WpaState::Network => {
                                if NET_END_RE.is_match(&line) {
                                    debug!("in state {:?} found end of network", state);
                                    if let Some(ssid) = ssid {
                                        // TODO: check if ssid is in filter list

                                        let mut valid = ssid_filter.is_empty();
                                        if !valid {
                                            if let Some(_pos) =
                                                ssid_filter.iter().position(|r| r.as_str() == ssid)
                                            {
                                                valid = true;
                                            }
                                        }

                                        if valid {
                                            if let Some(_pos) =
                                                wifis.iter().position(|r| r.get_ssid() == ssid)
                                            {
                                                debug!("Network '{}' is already contained in wifi list, skipping duplicate definition", ssid);
                                            } else {
                                                wifis
                                                    .push(WifiConfig::Params(Params { ssid, psk }));
                                            }
                                        } else {
                                            info!("ignoring wifi config for ssid: '{}'", ssid);
                                        }
                                    } else {
                                        warn!("empty network config encountered");
                                    }

                                    state = WpaState::Init;
                                    ssid = None;
                                    psk = None;
                                    continue;
                                }

                                if let Some(captures) = NET_PARAM1_RE.captures(&line) {
                                    let param = captures.get(1).unwrap().as_str();
                                    let value = captures.get(2).unwrap().as_str();
                                    debug!(
                                        "in state {:?} got param: '{}', value: '{}'",
                                        state, param, value
                                    );
                                    match param {
                                        "ssid" => {
                                            debug!("in state {:?} set ssid to '{}'", state, value);
                                            ssid = Some(String::from(value));
                                        }
                                        "psk" => {
                                            debug!("in state {:?} set psk to '{}'", state, value);
                                            psk = Some(String::from(value));
                                        }
                                        _ => {
                                            debug!("in state {:?} ignoring line '{}'", state, line);
                                        }
                                    }
                                    continue;
                                }

                                if let Some(captures) = NET_PARAM2_RE.captures(&line) {
                                    let param = captures.get(1).unwrap().as_str();
                                    let value = captures.get(2).unwrap().as_str();
                                    debug!(
                                        "in state {:?} got param: '{}', value: '{}'",
                                        state, param, value
                                    );
                                    match param {
                                        "ssid" => {
                                            debug!("in state {:?} set ssid to '{}'", state, value);
                                            ssid = Some(String::from(value));
                                        }
                                        "psk" => {
                                            debug!("in state {:?} set psk to '{}'", state, value);
                                            psk = Some(String::from(value));
                                        }
                                        _ => {
                                            debug!("in state {:?} ignoring line '{}'", state, line);
                                        }
                                    }
                                    continue;
                                }

                                warn!("in state {:?} ignoring line '{}'", state, line);
                            }
                        }
                    }
                    Err(why) => {
                        return Err(MigError::from(why.context(MigErrCtx::from_remark(
                            MigErrorKind::Upstream,
                            &format!("unexpected read error from {}", WPA_CONFIG_FILE),
                        ))));
                    }
                }
            }
        } else {
            debug!(
                "WifiConfig::from_wpa: file not found: '{}'",
                WPA_CONFIG_FILE
            );
        }

        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[allow(clippy::cognitive_complexity)] //TODO refactor this function to fix the clippy warning
    fn from_nwmgr(wifis: &mut Vec<WifiConfig>, ssid_filter: &[String]) -> Result<(), MigError> {
        trace!("WifiConfig::from_nwmgr: entered with {:?}", ssid_filter);
        if dir_exists(NWMGR_CONFIG_DIR)? {
            let paths = read_dir(NWMGR_CONFIG_DIR).context(MigErrCtx::from_remark(
                MigErrorKind::Upstream,
                &format!("Failed to list directory '{}'", NWMGR_CONFIG_DIR),
            ))?;

            lazy_static! {
                static ref NWMGR_SECTION_RE: Regex = Regex::new(NWMGR_SECTION_REGEX).unwrap();
                static ref NWMGR_PARAM_RE: Regex = Regex::new(NWMGR_PARAM_REGEX).unwrap();
            }

            for path in paths {
                if let Ok(path) = path {
                    let dir_path = path.path();
                    if dir_path.is_file() {
                        debug!("got path '{}'", dir_path.display());
                        let mut section: NwMgrSection = NwMgrSection::Other;
                        let mut is_wifi = false;
                        let mut ssid: Option<String> = None;

                        for line in read_to_string(&dir_path)
                            .context(MigErrCtx::from_remark(
                                MigErrorKind::Upstream,
                                &format!("failed to read file: '{}'", dir_path.display()),
                            ))?
                            .lines()
                        {
                            trace!("processing line: '{}'", line);
                            if let Some(captures) = NWMGR_SECTION_RE.captures(line) {
                                section = match captures.get(1).unwrap().as_str() {
                                    "connection" => NwMgrSection::Connection,
                                    "wifi" => NwMgrSection::Wifi,
                                    _ => NwMgrSection::Other,
                                };

                                debug!("got section: '{:?}'", section);
                            } else if let Some(captures) = NWMGR_PARAM_RE.captures(line) {
                                let param = captures.get(1).unwrap().as_str();
                                let value = captures.get(2).unwrap().as_str();
                                debug!("got param: '{}' : '{}'", param, value);
                                match section {
                                    NwMgrSection::Connection => {
                                        // TODO: lowercase this ?
                                        if param == "type" && value == "wifi" {
                                            debug!("Found wifi config");
                                            is_wifi = true;
                                            if let Some(ref _ssid) = ssid {
                                                break;
                                            }
                                        }
                                    }
                                    NwMgrSection::Wifi => {
                                        // TODO: look for ssid=
                                        if param == "ssid" {
                                            debug!("Found ssid: '{}'", value);
                                            ssid = Some(String::from(value));
                                            if is_wifi {
                                                break;
                                            }
                                        }
                                    }
                                    NwMgrSection::Other => (),
                                }
                            }
                        }

                        if is_wifi {
                            if let Some(ssid) = ssid {
                                if ssid_filter.is_empty() {
                                    wifis.push(WifiConfig::NwMgrFile(NwmgrFile {
                                        ssid,
                                        file: dir_path,
                                    }));
                                } else if let Some(_pos) =
                                    ssid_filter.iter().position(|r| r.as_str() == ssid)
                                {
                                    wifis.push(WifiConfig::NwMgrFile(NwmgrFile {
                                        ssid,
                                        file: dir_path,
                                    }));
                                } else {
                                    info!("ignoring wifi config for ssid: '{}'", ssid);
                                }
                            } else {
                                warn!(
                                    "from_nwmgr: no ssid found in wifi config: '{}'",
                                    dir_path.display()
                                );
                            }
                        } else {
                            debug!("from_nwmgr: not a wifi config: '{}'", dir_path.display());
                        }
                    }
                }
            }
        }

        Ok(())
    }

    pub(crate) fn create_nwmgr_file<P: AsRef<Path>>(
        &self,
        base_path: P,
        last_index: u64,
    ) -> Result<u64, MigError> {
        let mut index = last_index + 1;
        let mut file_ok = false;
        let base_path = base_path.as_ref();
        let mut path = path_append(base_path, &format!("resin-wifi-{}", index));

        while !file_ok {
            if file_exists(&path) {
                if is_migrator_file(&path)? {
                    file_ok = true;
                } else {
                    index += 1;
                    path = path_append(base_path, &format!("resin-wifi-{}", index));
                }
            } else {
                file_ok = true;
            }
        }

        info!("Creating NetworkManager file in '{}'", path.display());
        let mut nwmgr_file = File::create(&path).context(MigErrCtx::from_remark(
            MigErrorKind::Upstream,
            &format!("Failed to create file in '{}'", path.display()),
        ))?;

        let name = path.file_name().unwrap().to_string_lossy();

        lazy_static! {
            static ref NWMGR_SECTION_RE: Regex = Regex::new(NWMGR_SECTION_REGEX).unwrap();
            static ref NWMGR_ID_RE: Regex = Regex::new(NWMGR_ID_REGEX).unwrap();
        }

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
                let mut found = false;
                let mut conn_section = false;

                let mut content = format!("{}\n", BALENA_FILE_TAG);

                // copy file to dest folder updating the id.
                for line in read_to_string(&nwmgr_file.file)
                    .context(MigErrCtx::from_remark(
                        MigErrorKind::Upstream,
                        &format!("Failed to read file '{}'", nwmgr_file.file.display()),
                    ))?
                    .lines()
                {
                    if let Some(captures) = NWMGR_SECTION_RE.captures(line) {
                        if captures.get(1).unwrap().as_str() == "connection" {
                            conn_section = true;
                            content += &format!("{}\n", line);
                            if !found {
                                // add id once to connection section
                                content += &format!("id={}\n", &name);
                                found = true;
                                continue;
                            }
                        }
                    }

                    if NWMGR_ID_RE.is_match(line) && conn_section {
                        // uncomment id= lines in connection section
                        content += &format!("# {}\n", line);
                        continue;
                    }

                    // all not handled are cloned
                    content += &format!("{}\n", &line);
                }

                if !found {
                    warn!(
                        "No [connection] section found in NetworkManager file: '{}', skipping file",
                        nwmgr_file.file.display()
                    );
                    return Ok(last_index);
                }
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
