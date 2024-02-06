use log::{debug, error, info};
use regex::Regex;

use crate::stage1::device_impl::check_os;
use crate::{
    common::{options::Options, Error, ErrorKind, Result},
    stage1::{
        defs::{DeviceType, DEV_TYPE_RPI1, DEV_TYPE_RPI2, DEV_TYPE_RPI3, DEV_TYPE_RPI4_64},
        device::Device,
    },
};

// TODO: Frankly, at this point we should either use a much simpler regex or
// just allowlist the specific models we want...
const RPI_MODEL_REGEX: &str = r#"^Raspberry\s+Pi\s+(1|2|3|4|Compute Module 3|Zero)(\s+(Model\s+(\S+)|W))?(\s+Plus)?\s+(Rev\s+(\S+))$"#;
const RPI1_SLUGS: [&str; 1] = [DEV_TYPE_RPI1];
const RPI2_SLUGS: [&str; 1] = [DEV_TYPE_RPI2];
const RPI3_SLUGS: [&str; 1] = [DEV_TYPE_RPI3];
const RPI4_64_SLUGS: [&str; 1] = [DEV_TYPE_RPI4_64];

const SUPPORTED_OSSES: [&str; 4] = [
    "Raspbian GNU/Linux 8 (jessie)",
    "Raspbian GNU/Linux 9 (stretch)",
    "Raspbian GNU/Linux 10 (buster)",
    "Ubuntu 20.04 LTS",
];

pub(crate) fn is_rpi(opts: &Options, model_string: &str) -> Result<Option<Box<dyn Device>>> {
    debug!(
        "raspberrypi::is_rpi: entered with model string: '{}'",
        model_string
    );

    if let Some(captures) = Regex::new(RPI_MODEL_REGEX).unwrap().captures(model_string) {
        let pitype = captures.get(1).unwrap().as_str();
        let model = if let Some(model) = captures.get(3) {
            model.as_str().trim_matches(char::from(0))
        } else {
            captures
                .get(2)
                .unwrap()
                .as_str()
                .trim_matches(char::from(0))
        };

        let revision = captures
            .get(5)
            .unwrap()
            .as_str()
            .trim_matches(char::from(0));

        debug!(
            "raspberrypi::is_rpi: selection entered with string: '{}'",
            pitype
        );

        match pitype {
            "1" | "Zero" => {
                info!("Identified RaspberryPi 1/Zero",);
                Ok(Some(Box::new(RaspberryPi1::from_config(opts)?)))
            }
            "2" => {
                info!("Identified RaspberryPi 2",);
                Ok(Some(Box::new(RaspberryPi2::from_config(opts)?)))
            }
            "3" | "Compute Module 3" => {
                info!("Identified RaspberryPi 3");
                Ok(Some(Box::new(RaspberryPi3::from_config(opts)?)))
            }
            "4" => {
                info!("Identified RaspberryPi 4");
                Ok(Some(Box::new(RaspberryPi4_64::from_config(opts)?)))
            }
            _ => {
                debug!("unknown PI type: '{}'", pitype);
                let message = format!("The raspberry pi type reported by your device ('{} {} rev {}') is not supported by balena-migrate", pitype, model, revision);
                error!("{}", message);
                Err(Error::with_context(ErrorKind::InvParam, &message))
            }
        }
    } else {
        debug!("no match for Raspberry PI on: {}", model_string);
        Ok(None)
    }
}

pub(crate) struct RaspberryPi1;
impl RaspberryPi1 {
    pub fn from_config(opts: &Options) -> Result<RaspberryPi1> {
        if opts.migrate() && !check_os(&SUPPORTED_OSSES, opts, "Raspberry PI 1")? {
            return Err(Error::displayed());
        }

        Ok(RaspberryPi1 {})
    }
}

impl Device for RaspberryPi1 {
    fn supports_device_type(&self, dev_type: &str) -> bool {
        RPI1_SLUGS.contains(&dev_type)
    }

    fn get_device_type(&self) -> DeviceType {
        DeviceType::RaspberryPi1
    }
}

pub(crate) struct RaspberryPi2;
impl RaspberryPi2 {
    pub fn from_config(opts: &Options) -> Result<RaspberryPi2> {
        if opts.migrate() && !check_os(&SUPPORTED_OSSES, opts, "Raspberry PI 2")? {
            return Err(Error::displayed());
        }

        Ok(RaspberryPi2 {})
    }
}

impl Device for RaspberryPi2 {
    fn supports_device_type(&self, dev_type: &str) -> bool {
        RPI2_SLUGS.contains(&dev_type)
    }

    fn get_device_type(&self) -> DeviceType {
        DeviceType::RaspberryPi2
    }
}

pub(crate) struct RaspberryPi3;

impl RaspberryPi3 {
    pub fn from_config(opts: &Options) -> Result<RaspberryPi3> {
        if opts.migrate() && !check_os(&SUPPORTED_OSSES, opts, "Raspberry PI 3")? {
            return Err(Error::displayed());
        }

        Ok(RaspberryPi3)
    }
}

impl Device for RaspberryPi3 {
    fn supports_device_type(&self, dev_type: &str) -> bool {
        RPI3_SLUGS.contains(&dev_type)
    }

    fn get_device_type(&self) -> DeviceType {
        DeviceType::RaspberryPi3
    }
}

pub(crate) struct RaspberryPi4_64;

impl RaspberryPi4_64 {
    pub fn from_config(opts: &Options) -> Result<RaspberryPi4_64> {
        if opts.migrate() && !check_os(&SUPPORTED_OSSES, opts, "Raspberry PI 4")? {
            return Err(Error::displayed());
        }

        Ok(RaspberryPi4_64)
    }
}

impl Device for RaspberryPi4_64 {
    fn supports_device_type(&self, dev_type: &str) -> bool {
        RPI4_64_SLUGS.contains(&dev_type)
    }

    fn get_device_type(&self) -> DeviceType {
        DeviceType::RaspberryPi4
    }
}

#[cfg(test)]

mod tests {
    use super::*;

    #[test]
    fn test_model_regex() {
        let re = Regex::new(RPI_MODEL_REGEX).unwrap();

        assert!(re.is_match("Raspberry Pi Zero W Rev 1.1")); // RPI Zero W
        assert!(re.is_match("Raspberry Pi Compute Module 3 Plus Rev 1.0")); // balena Fin
        assert!(re.is_match("Raspberry Pi 4 Model B Rev 1.1")); // RPI 4
        assert!(re.is_match("Raspberry Pi 3 Model B Plus Rev 1.3")); // RPI 3
        assert!(re.is_match("Raspberry Pi 2 Model B Rev 1.1")); // RPI 2

        assert!(!re.is_match("Blueberry Pi Zero W Rev 1.1"));
        assert!(!re.is_match("Raspberry Pii Zero W Rev 1.1"));
        assert!(!re.is_match("Raspberry Pi 3 Model B Minus Rev 1.3"));
    }
}
