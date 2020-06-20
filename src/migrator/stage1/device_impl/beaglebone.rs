use log::{debug, error, trace};
use regex::Regex;

use crate::{
    common::{Error, ErrorKind, Options, Result},
    stage1::{
        defs::{DeviceType, DEV_TYPE_BBB, DEV_TYPE_BBG, DEV_TYPE_BBXM},
        device::Device,
        device_impl::check_os,
    },
};

const SUPPORTED_OSSES: [&str; 4] = [
    "Ubuntu 18.04.2 LTS",
    "Ubuntu 14.04.1 LTS",
    "Debian GNU/Linux 9 (stretch)",
    "Debian GNU/Linux 7 (wheezy)",
];

// Supported models
// TI OMAP3 BeagleBoard xM
const BB_MODEL_REGEX: &str = r#"^((\S+\s+)*(\S+))\s+Beagle(Bone|Board)\s+(\S+)$"#;

const BBG_SLUGS: [&str; 1] = [DEV_TYPE_BBG];
const BBB_SLUGS: [&str; 1] = [DEV_TYPE_BBB];
const BBXM_SLUGS: [&str; 1] = [DEV_TYPE_BBXM];

// TODO: check location of uEnv.txt or other files files to improve reliability

pub(crate) fn is_bb(opts: &Options, model_string: &str) -> Result<Option<Box<dyn Device>>> {
    trace!(
        "Beaglebone::is_bb: entered with model string: '{}'",
        model_string
    );

    if model_string.eq("TI AM335x BeagleBone") {
        // TODO: found this device model string on a beaglebone-green running debian wheezy
        debug!("match found for BeagleboneGreen");
        Ok(Some(Box::new(BeagleboneGreen::from_config(opts)?)))
    } else if let Some(captures) = Regex::new(BB_MODEL_REGEX).unwrap().captures(model_string) {
        let model = captures
            .get(5)
            .unwrap()
            .as_str()
            .trim_matches(char::from(0));

        match model {
            "xM" => {
                debug!("match found for BeagleboardXM");
                // TODO: dtb-name is a guess replace with real one
                Ok(Some(Box::new(BeagleboardXM::from_config(opts)?)))
            }
            "Green" => {
                debug!("match found for BeagleboneGreen");
                Ok(Some(Box::new(BeagleboneGreen::from_config(opts)?)))
            }
            "Black" => {
                debug!("match found for BeagleboneBlack");
                Ok(Some(Box::new(BeagleboneBlack::from_config(opts)?)))
            }
            _ => {
                let message = format!("The beaglebone model reported by your device ('{}') is not supported by balena-migrate", model);
                error!("{}", message);
                Err(Error::with_context(ErrorKind::InvParam, &message))
            }
        }
    } else {
        debug!("no match for beaglebone on: <{}>", model_string);
        Ok(None)
    }
}

pub(crate) struct BeagleboneGreen {}

impl BeagleboneGreen {
    // this is used in stage1
    fn from_config(opts: &Options) -> Result<BeagleboneGreen> {
        if !check_os(&SUPPORTED_OSSES, opts, "Beaglebone Green")? {
            return Err(Error::displayed());
        }

        Ok(BeagleboneGreen {})
    }
}

impl Device for BeagleboneGreen {
    fn supports_device_type(&self, dev_type: &str) -> bool {
        BBG_SLUGS.contains(&dev_type)
    }

    fn get_device_type(&self) -> DeviceType {
        DeviceType::BeagleboneGreen
    }
}

pub(crate) struct BeagleboneBlack {}

impl BeagleboneBlack {
    // this is used in stage1
    fn from_config(opts: &Options) -> Result<BeagleboneBlack> {
        if !check_os(&SUPPORTED_OSSES, opts, "Beaglebone Black")? {
            return Err(Error::displayed());
        }

        Ok(BeagleboneBlack {})
    }
}

impl Device for BeagleboneBlack {
    fn supports_device_type(&self, dev_type: &str) -> bool {
        BBB_SLUGS.contains(&dev_type)
    }

    fn get_device_type(&self) -> DeviceType {
        DeviceType::BeagleboneBlack
    }
}

pub(crate) struct BeagleboardXM {}

impl BeagleboardXM {
    // this is used in stage1
    fn from_config(opts: &Options) -> Result<BeagleboardXM> {
        if opts.is_migrate() && !check_os(&SUPPORTED_OSSES, opts, "Beagleboard XM")? {
            return Err(Error::displayed());
        }

        Ok(BeagleboardXM {})
    }
}

impl Device for BeagleboardXM {
    fn supports_device_type(&self, dev_type: &str) -> bool {
        BBXM_SLUGS.contains(&dev_type)
    }

    fn get_device_type(&self) -> DeviceType {
        DeviceType::BeagleboardXM
    }
}
