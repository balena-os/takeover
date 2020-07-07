use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::stage1::block_device_info::DeviceNum;
use crate::{
    common::{
        call,
        defs::BLKID_CMD,
        error::{Error, ErrorKind, Result, ToError},
        path_append,
    },
    stage1::block_device_info::{block_device::BlockDevice, mount::Mount},
};
use lazy_static::lazy_static;
use log::{debug, warn};
use regex::Regex;

#[derive(Clone, Debug)]
pub(crate) struct PartitionInfo {
    uuid: Option<String>,
    block_size: Option<u32>,
    fs_type: Option<String>,
    label: Option<String>,
    part_uuid: Option<String>,
}

impl PartitionInfo {
    pub fn new<P: AsRef<Path>>(device: P) -> Result<PartitionInfo> {
        lazy_static! {
            static ref START_REGEX: Regex = Regex::new(r"^([^:]+):\s+(.+)$").unwrap();
            static ref NEXT_REGEX: Regex =
                Regex::new(r##"^([^=]+)="([^"]*)"(\s+(.+))?$"##).unwrap();
        }

        let cmd_res = call_command!(
            BLKID_CMD,
            &[&*device.as_ref().to_string_lossy()],
            "Failed to call blkid"
        )?;

        if let Some(captures) = START_REGEX.captures(cmd_res.as_str()) {
            let mut next_params = captures.get(2);

            let mut uuid: Option<String> = None;
            let mut block_size: Option<u32> = None;
            let mut fs_type: Option<String> = None;
            let mut label: Option<String> = None;
            let mut part_uuid: Option<String> = None;

            while let Some(params) = next_params {
                if let Some(captures) = NEXT_REGEX.captures(params.as_str()) {
                    let param_name = captures.get(1).unwrap().as_str();
                    let param_value = captures.get(2).unwrap().as_str();
                    debug!(
                        "PartitionInfo::new: {} got param name: {}, value: {}",
                        device.as_ref().display(),
                        param_name,
                        param_value
                    );
                    match param_name {
                        "UUID" => {
                            uuid = Some(param_value.to_owned());
                        }
                        "BLOCK_SIZE" => {
                            block_size = Some(param_value.parse::<u32>().upstream_with_context(
                                &format!("Failed to parse block size from {}", param_value),
                            )?);
                        }
                        "TYPE" => {
                            fs_type = Some(param_value.to_owned());
                        }
                        "PARTLABEL" => {
                            label = Some(param_value.to_owned());
                        }
                        "PARTUUID" => {
                            part_uuid = Some(param_value.to_owned());
                        }
                        _ => {
                            warn!("unexpected parameter name found: '{}'", param_name);
                        }
                    }
                    next_params = captures.get(4);
                } else {
                    break;
                }
            }

            let part_info = PartitionInfo {
                uuid,
                block_size,
                part_uuid,
                fs_type,
                label,
            };
            debug!(
                "PartitionInfo::new: for {} got {:?}",
                device.as_ref().display(),
                part_info,
            );
            Ok(part_info)
        } else {
            Err(Error::with_context(
                ErrorKind::InvParam,
                &format!("Could not parse blkid output: '{}'", cmd_res),
            ))
        }
    }
    pub fn fs_type(&self) -> Option<&str> {
        if let Some(fs_type) = &self.fs_type {
            Some(fs_type)
        } else {
            None
        }
    }
}

#[derive(Clone)]
pub(crate) struct Partition {
    name: String,
    device_num: DeviceNum,
    mounted: Option<Mount>,
    parent: Rc<Box<dyn BlockDevice>>,
    partition_info: PartitionInfo,
}

impl Partition {
    pub fn new(
        name: &str,
        device_num: DeviceNum,
        mounted: Option<Mount>,
        parent: Rc<Box<dyn BlockDevice>>,
    ) -> Result<Partition> {
        Ok(Partition {
            name: name.to_owned(),
            device_num,
            mounted,
            parent,
            partition_info: PartitionInfo::new(&format!("/dev/{}", name))?,
        })
    }
}

impl BlockDevice for Partition {
    fn get_device_num(&self) -> &DeviceNum {
        &self.device_num
    }

    fn get_mountpoint(&self) -> &Option<Mount> {
        &self.mounted
    }

    fn get_name(&self) -> &str {
        self.name.as_str()
    }

    fn get_dev_path(&self) -> PathBuf {
        path_append("/dev", &self.name)
    }

    fn get_parent(&self) -> Option<&Rc<Box<dyn BlockDevice>>> {
        Some(&self.parent)
    }

    fn is_partition(&self) -> bool {
        true
    }

    fn set_mountpoint(&mut self, mountpoint: Mount) {
        self.mounted = Some(mountpoint);
    }

    fn get_partition_info(&self) -> Option<&PartitionInfo> {
        Some(&self.partition_info)
    }
}
