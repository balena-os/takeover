use log::{debug, error, info};
use regex::Regex;

use crate::stage1::device_impl::check_os;
use crate::{
    common::{options::Options, Error, ErrorKind, Result},
    stage1::{
        defs::{DeviceType, DEV_TYPE_RPI2, DEV_TYPE_RPI3, DEV_TYPE_RPI4_64},
        device::Device,
    },
};

const RPI_MODEL_REGEX: &str = r#"^Raspberry\s+Pi\s+(\S+)\s+Model\s+(.*)$"#;
const RPI2_SLUGS: [&str; 1] = [DEV_TYPE_RPI2];
const RPI3_SLUGS: [&str; 1] = [DEV_TYPE_RPI3];
const RPI4_64_SLUGS: [&str; 1] = [DEV_TYPE_RPI4_64];

pub(crate) fn is_rpi(opts: &Options, model_string: &str) -> Result<Option<Box<dyn Device>>> {
    debug!(
        "raspberrypi::is_rpi: entered with model string: '{}'",
        model_string
    );

    if let Some(captures) = Regex::new(RPI_MODEL_REGEX).unwrap().captures(model_string) {
        let pitype = captures.get(1).unwrap().as_str();
        let model = captures
            .get(2)
            .unwrap()
            .as_str()
            .trim_matches(char::from(0));

        debug!(
            "raspberrypi::is_rpi: selection entered with string: '{}'",
            pitype
        );

        match pitype {
            "2" => {
                info!("Identified RaspberryPi2: model {}", model);
                Ok(Some(Box::new(RaspberryPi2::from_config(opts)?)))
            }
            "3" => {
                info!("Identified RaspberryPi3: model {}", model);
                Ok(Some(Box::new(RaspberryPi3::from_config(opts)?)))
            }
            "4" => {
                info!("Identified RaspberryPi4: model {}", model);
                Ok(Some(Box::new(RaspberryPi4_64::from_config(opts)?)))
            }
            _ => {
                let message = format!("The raspberry pi type reported by your device ('{} {}') is not supported by balena-migrate", pitype, model);
                error!("{}", message);
                Err(Error::with_context(ErrorKind::InvParam, &message))
            }
        }
    } else {
        debug!("no match for Raspberry PI on: {}", model_string);
        Ok(None)
    }
}

pub(crate) struct RaspberryPi2;

impl RaspberryPi2 {
    pub fn from_config(opts: &Options) -> Result<RaspberryPi2> {
        const SUPPORTED_OSSES: &[&str] = &["Raspbian GNU/Linux 10 (buster)"];

        if opts.is_migrate() && !check_os(SUPPORTED_OSSES, opts, "Raspberry PI 2")? {
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
        const SUPPORTED_OSSES: &[&str] = &[
            "Raspbian GNU/Linux 8 (jessie)",
            "Raspbian GNU/Linux 9 (stretch)",
            "Raspbian GNU/Linux 10 (buster)",
        ];

        if opts.is_migrate() && !check_os(SUPPORTED_OSSES, opts, "Raspberry PI 3")? {
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
        const SUPPORTED_OSSES: &[&str] = &[
            "Raspbian GNU/Linux 8 (jessie)",
            "Raspbian GNU/Linux 9 (stretch)",
            "Raspbian GNU/Linux 10 (buster)",
        ];

        if opts.is_migrate() && !check_os(SUPPORTED_OSSES, opts, "Raspberry PI 4")? {
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
