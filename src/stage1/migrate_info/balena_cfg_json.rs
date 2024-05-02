use crate::{
    common::{Error, ErrorKind, Options, Result, ToError},
    stage1::{device::Device, utils::check_tcp_connect},
};

use log::{debug, error, info};
use reqwest::blocking::Client;
use serde_json::Value;
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::BufReader;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub(crate) struct BalenaCfgJson {
    config: HashMap<String, Value>,
    file: PathBuf,
    modified: bool,
}

impl BalenaCfgJson {
    pub fn new<P: AsRef<Path>>(cfg_file: P) -> Result<BalenaCfgJson> {
        let cfg_file = cfg_file
            .as_ref()
            .canonicalize()
            .upstream_with_context(&format!(
                "Failed to canonicalize path: '{}'",
                cfg_file.as_ref().display()
            ))?;

        Ok(BalenaCfgJson {
            config: serde_json::from_reader(BufReader::new(
                File::open(&cfg_file).upstream_with_context(&format!(
                    "new: cannot open file '{}'",
                    cfg_file.display()
                ))?,
            ))
            .upstream_with_context(&format!(
                "Failed to parse json from file '{}'",
                cfg_file.display()
            ))?,
            file: cfg_file,
            modified: false,
        })
    }

    pub fn write<P: AsRef<Path>>(&mut self, target_path: P) -> Result<()> {
        let target_path = target_path.as_ref();
        let out_file = OpenOptions::new()
            .create(true)
            .write(true)
            .open(target_path)
            .upstream_with_context(&format!(
                "Failed to open file for writing: '{}'",
                target_path.display()
            ))?;

        serde_json::to_writer(out_file, &self.config).upstream_with_context(&format!(
            "Failed save modified config.json to '{}'",
            target_path.display()
        ))?;

        self.modified = false;
        self.file = target_path.canonicalize().upstream_with_context(&format!(
            "Failed to canonicalize path: '{}'",
            target_path.display()
        ))?;

        Ok(())
    }

    pub fn check(&self, opts: &Options, device: &dyn Device) -> Result<()> {
        info!("Configured for fleet id: {}", self.get_app_id()?);

        let device_type = self.get_device_type()?;
        if opts.dt_check() {
            if !device.supports_device_type(device_type.as_str()) {
                error!("The device type configured in config.json ({}) is not supported by the detected device type {:?}",
                   device_type, device.get_device_type());
                return Err(Error::displayed());
            }
        } else {
            info!("Device type configured in config.json is {}; skipping compatibility check due to --no-dt-check option",
            device_type)
        }

        if opts.api_check() {
            let api_endpoint = &self.get_api_endpoint()?;
            check_api(api_endpoint)?;
        }

        if opts.vpn_check() {
            let vpn_endpoint = self.get_vpn_endpoint()?;
            let vpn_port = self.get_vpn_port()? as u16;
            if let Ok(_v) = check_tcp_connect(&vpn_endpoint, vpn_port, opts.check_timeout()) {
                // TODO: call a command on API instead of just connecting
                info!("connection to vpn: {}:{} is ok", vpn_endpoint, vpn_port);
            } else {
                return Err(Error::with_context(
                    ErrorKind::InvState,
                    &format!(
                        "failed to connect to vpn server @ {}:{} your device might not come online",
                        vpn_endpoint, vpn_port
                    ),
                ));
            }
        }

        Ok(())
    }

    pub fn is_modified(&self) -> bool {
        self.modified
    }

    fn get_str_val(&self, name: &str) -> Result<String> {
        if let Some(value) = self.config.get(name) {
            if let Some(value) = value.as_str() {
                Ok(value.to_string())
            } else {
                Err(Error::with_context(
                    ErrorKind::InvParam,
                    &format!(
                        "Invalid type encountered for '{}', expected String, found {:?} in config.json",
                        name, value
                    ),
                ))
            }
        } else {
            Err(Error::with_context(
                ErrorKind::NotFound,
                &format!("Key could not be found in config.json: '{}'", name),
            ))
        }
    }

    fn get_uint_val(&self, name: &str) -> Result<u64> {
        if let Some(value) = self.config.get(name) {
            if let Some(value) = value.as_u64() {
                Ok(value)
            } else if let Some(str_val) = value.as_str() {
                Ok(str_val.parse::<u64>().upstream_with_context(&format!(
                    "Failed to parse uint value for '{}' from config.json",
                    name
                ))?)
            } else {
                Err(Error::with_context(
                    ErrorKind::InvParam,
                    &format!(
                        "Invalid type encountered for '{}', expected uint, found {:?}",
                        name, value
                    ),
                ))
            }
        } else {
            Err(Error::with_context(
                ErrorKind::NotFound,
                &format!("Key could not be found in config.json: '{}'", name),
            ))
        }
    }

    /*pub fn get_hostname(&self) -> Result<String, Error> {
        self.get_str_val("hostname")
    }*/

    pub fn set_host_name(&mut self, hostname: &str) -> Option<String> {
        self.modified = true;

        self.config
            .insert("hostname".to_string(), Value::String(hostname.to_string()))
            .map(|value| value.to_string())
    }

    pub fn get_app_id(&self) -> Result<u64> {
        self.get_uint_val("applicationId")
    }

    pub fn get_api_key(&self) -> Result<String> {
        // The API Key required can exist with key `apiKey` or in case of an already provisioned device, `deviceApikey`
        match self.get_str_val("apiKey") {
            Ok(value) => Ok(value),
            Err(e) => {
                if let ErrorKind::NotFound = e.kind() {
                    // If the error kind is NotFound, try "deviceApiKey"
                    match self.get_str_val("deviceApiKey") {
                        Ok(value) => Ok(value),
                        Err(e) => Err(e), // Propagate any errors from the second attempt
                    }
                } else {
                    Err(e) // Propagate any other kinds of errors
                }
            }
        }
    }

    pub fn get_api_endpoint(&self) -> Result<String> {
        self.get_str_val("apiEndpoint")
    }

    fn get_vpn_endpoint(&self) -> Result<String> {
        self.get_str_val("vpnEndpoint")
    }

    fn get_vpn_port(&self) -> Result<u64> {
        self.get_uint_val("vpnPort")
    }

    pub fn get_device_type(&self) -> Result<String> {
        self.get_str_val("deviceType")
    }

    pub fn get_path(&self) -> &Path {
        &self.file
    }

    pub fn override_device_type(&mut self, change_to: &str) -> Option<String> {
        self.modified = true;

        self.config
            .insert(
                "deviceType".to_string(),
                Value::String(change_to.to_string()),
            )
            .map(|value| value.to_string())
    }

    pub fn get_uuid(&self) -> Result<String> {
        self.get_str_val("uuid")
    }
}

fn check_api(api_endpoint: &str) -> Result<()> {
    let ping_endpoint = format!("{}/ping", api_endpoint);
    let res = Client::builder()
        .build()
        .upstream_with_context("Failed to create https client")?
        .get(&ping_endpoint)
        .send()
        .upstream_with_context(&format!(
            "Failed to send https request url: {}",
            &api_endpoint
        ))?;
    debug!("Result = {:?}", res);
    let status = res.status();
    let response = res
        .text()
        .upstream_with_context("Failed to read response")?;

    if status.is_success() && response.trim() == "OK" {
        info!("connection to api: {} is ok", &api_endpoint);
        Ok(())
    } else {
        Err(Error::with_context(
            ErrorKind::InvState,
            &format!(
                "Got an unexpected reply from the API server @ {} : {}",
                &ping_endpoint, &response
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;

    #[test]
    fn test_get_api_key_found_as_api_key() {
        let mut config: HashMap<String, Value> = HashMap::new();
        config.insert("apiKey".to_string(), "abcd".into());
        let balena_cfg = BalenaCfgJson {
            config,
            file: PathBuf::new(),
            modified: false,
        };
        assert_eq!(balena_cfg.get_api_key().unwrap(), "abcd");
    }

    #[test]
    fn test_get_api_key_found_as_device_api_key() {
        let mut config: HashMap<String, Value> = HashMap::new();
        config.insert("deviceApiKey".to_string(), "abcd".into());
        let balena_cfg = BalenaCfgJson {
            config,
            file: PathBuf::new(),
            modified: false,
        };
        assert_eq!(balena_cfg.get_api_key().unwrap(), "abcd");
    }

    #[test]
    fn test_get_api_key_not_found() {
        let config = HashMap::new(); // No API keys present
        let balena_cfg = BalenaCfgJson {
            config,
            file: PathBuf::new(),
            modified: false,
        };
        assert!(balena_cfg.get_api_key().is_err());
    }
}
