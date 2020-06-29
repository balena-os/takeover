use crate::common::{path_append, Error, Result, ToError};

use lazy_static::lazy_static;
use log::{debug, trace};
use nix::sys::stat::{major, minor, stat};
use regex::Regex;
use std::collections::HashMap;
use std::fmt;
use std::fs::{read_dir, read_to_string};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::result;

mod mount;
use mount::{Mount, MountTab};

pub(crate) mod block_device;
pub(crate) use block_device::BlockDevice;

mod device;
use device::Device;

mod partition;
use crate::ErrorKind;
use partition::Partition;
use std::str::FromStr;

// TODO: add mountpoints for  partitions

const BLOC_DEV_SUPP_MAJ_NUMBERS: [u64; 45] = [
    3, 8, 9, 21, 33, 34, 44, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 64, 65, 66, 67, 68, 69,
    70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 179, 180, 259,
];

type DeviceMap = HashMap<PathBuf, Rc<Box<dyn BlockDevice>>>;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DeviceNum {
    major: u64,
    minor: u64,
}

impl DeviceNum {
    pub fn new(raw_num: u64) -> DeviceNum {
        DeviceNum {
            major: major(raw_num),
            minor: minor(raw_num),
        }
    }

    pub fn major(&self) -> u64 {
        self.major
    }

    pub fn minor(&self) -> u64 {
        self.minor
    }
}

impl fmt::Display for DeviceNum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.major, self.minor)
    }
}

impl FromStr for DeviceNum {
    type Err = Error;

    fn from_str(s: &str) -> result::Result<Self, Self::Err> {
        lazy_static! {
            static ref DEVNUM_RE: Regex = Regex::new(r#"^(\d+):(\d+)$"#).unwrap();
        }

        if let Some(captures) = DEVNUM_RE.captures(s.trim()) {
            Ok(Self {
                major: captures
                    .get(1)
                    .unwrap()
                    .as_str()
                    .parse::<u64>()
                    .upstream_with_context(&format!(
                        "Failed to parse device major number from '{}'",
                        s
                    ))?,
                minor: captures
                    .get(2)
                    .unwrap()
                    .as_str()
                    .parse::<u64>()
                    .upstream_with_context(&format!(
                        "Failed to parse major device major number from '{}'",
                        s
                    ))?,
            })
        } else {
            Err(Error::with_context(
                ErrorKind::InvState,
                &format!(
                    "Failed to parse block device major:minor numbers from '{}'",
                    s
                ),
            ))
        }
    }
}

#[derive(Clone)]
pub(crate) struct BlockDeviceInfo {
    root_device: Rc<Box<dyn BlockDevice>>,
    root_partition: Option<Rc<Box<dyn BlockDevice>>>,
    devices: DeviceMap,
}

impl BlockDeviceInfo {
    pub fn new() -> Result<BlockDeviceInfo> {
        let stat_res = stat("/").upstream_with_context("Failed to stat root")?;
        let root_number = DeviceNum::new(stat_res.st_dev);
        let mounts = Mount::from_mtab()?;

        debug!(
            "new: Root device number is: {}:{}",
            root_number.major(),
            root_number.minor()
        );

        let sys_path = PathBuf::from("/sys/block/");
        let read_dir = read_dir(&sys_path).upstream_with_context(&format!(
            "Failed to read directory '{}'",
            sys_path.display()
        ))?;

        let mut device_map: DeviceMap = DeviceMap::new();
        for entry in read_dir {
            match entry {
                Ok(entry) => {
                    let curr_path = entry.path();
                    let curr_dev = BlockDeviceInfo::path_filename_as_string(&curr_path)?;
                    let curr_number = BlockDeviceInfo::get_maj_minor(&curr_path)?;
                    trace!(
                        "new: Looking at path '{}', device '{}' number: {}",
                        curr_path.display(),
                        curr_dev,
                        curr_number,
                    );

                    if !BLOC_DEV_SUPP_MAJ_NUMBERS.contains(&curr_number.major()) {
                        trace!(
                            "Skipping device '{}' with block device major {}",
                            curr_dev,
                            curr_number.major()
                        );
                        continue;
                    }

                    let dev_path = path_append("/dev", &curr_dev);
                    if !dev_path.exists() {
                        return Err(Error::with_context(
                            ErrorKind::DeviceNotFound,
                            &format!("device path does not exist: '{}'", dev_path.display()),
                        ));
                    }

                    // TODO: fill mounted

                    let mounted: Option<Mount> = if let Some(mount) = mounts.get(&dev_path) {
                        Some(mount.clone())
                    } else if root_number == curr_number {
                        if let Some(mount) = mounts.get(PathBuf::from("/dev/root").as_path()) {
                            Some(mount.clone())
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    let device = Rc::new(Box::new(Device {
                        name: curr_dev,
                        device_num: curr_number,
                        mounted,
                    }) as Box<dyn BlockDevice>);

                    BlockDeviceInfo::read_partitions(
                        &device,
                        &mounts,
                        &curr_path,
                        &root_number,
                        &mut device_map,
                    )?;
                    device_map.insert(dev_path, device.clone());

                    debug!("new: got device: {:?}", device);
                }
                Err(why) => {
                    return Err(Error::with_all(
                        ErrorKind::Upstream,
                        &format!(
                            "Failed to read directory entry from '{}'",
                            sys_path.display(),
                        ),
                        Box::new(why),
                    ));
                }
            }
        }

        let mut root_device: Option<Rc<Box<dyn BlockDevice>>> = None;
        let mut root_partition: Option<Rc<Box<dyn BlockDevice>>> = None;

        for device_rc in device_map.values_mut() {
            let device = device_rc.as_ref();
            if device.get_device_num() == &root_number {
                if let Some(parent) = device.get_parent() {
                    root_device = Some(parent.clone());
                    root_partition = Some(device_rc.clone())
                } else {
                    root_device = Some(device_rc.clone());
                    root_partition = None;
                }
                break;
            }
        }

        if let Some(root_device) = root_device {
            if let Some(root_partition) = root_partition {
                return Ok(BlockDeviceInfo {
                    root_device,
                    root_partition: Some(root_partition),
                    devices: device_map,
                });
            }
        }

        Err(Error::with_context(
            ErrorKind::InvState,
            "Failed to find root device",
        ))
    }

    fn read_partitions<P: AsRef<Path>>(
        device: &Rc<Box<dyn BlockDevice>>,
        mounts: &MountTab,
        dev_path: P,
        root_number: &DeviceNum,
        device_map: &mut DeviceMap,
    ) -> Result<()> {
        let dev_path = dev_path.as_ref();
        let dir_entries = read_dir(dev_path).upstream_with_context(&format!(
            "Failed to read directory '{}'",
            dev_path.display()
        ))?;

        for entry in dir_entries {
            match entry {
                Ok(entry) => {
                    let currdir = entry.path();
                    if entry
                        .metadata()
                        .upstream_with_context(&format!(
                            "Failed toretrieve metadata for '{}'",
                            currdir.display()
                        ))?
                        .is_dir()
                    {
                        let part_name = BlockDeviceInfo::path_filename_as_string(&currdir)?;

                        if !part_name.starts_with(&device.as_ref().get_name()) {
                            trace!("Skipping folder '{}", currdir.display());
                            continue;
                        }

                        let curr_number = BlockDeviceInfo::get_maj_minor(&currdir)?;
                        let dev_path = path_append("/dev", &part_name);

                        let mounted = if let Some(mount) = mounts.get(dev_path.as_path()) {
                            Some(mount.clone())
                        } else if curr_number == *root_number {
                            if let Some(mount) = mounts.get(PathBuf::from("/dev/root").as_path()) {
                                Some(mount.clone())
                            } else {
                                None
                            }
                        } else {
                            None
                        };

                        let partition = Rc::new(Box::new(Partition {
                            name: part_name,
                            device_num: curr_number,
                            mounted,
                            parent: device.clone(),
                        }) as Box<dyn BlockDevice>);

                        debug!(
                            "found  partition '{:?}' in '{}'",
                            partition,
                            currdir.display(),
                        );
                        device_map.insert(dev_path, partition);
                    }
                }
                Err(why) => {
                    return Err(Error::with_all(
                        ErrorKind::Upstream,
                        &format!(
                            "Failed to read directory entry from '{}'",
                            dev_path.display(),
                        ),
                        Box::new(why),
                    ));
                }
            }
        }

        Ok(())
    }

    pub fn get_root_device(&self) -> &Rc<Box<dyn BlockDevice>> {
        &self.root_device
    }

    #[allow(dead_code)]
    pub fn get_root_partition(&self) -> &Option<Rc<Box<dyn BlockDevice>>> {
        &self.root_partition
    }

    pub fn get_devices(&self) -> &DeviceMap {
        &self.devices
    }

    fn get_maj_minor<P: AsRef<Path>>(dev_path: P) -> Result<DeviceNum> {
        let dev_info_path = path_append(dev_path.as_ref(), "dev");
        let dev_info = read_to_string(&dev_info_path).upstream_with_context(&format!(
            "Failed to read file '{}'",
            dev_info_path.display()
        ))?;

        Ok(DeviceNum::from_str(dev_info.as_str())?)
    }

    /// extract last element of path as string
    fn path_filename_as_string<P: AsRef<Path>>(path: P) -> Result<String> {
        let path = path.as_ref();
        if let Some(dev_name) = path.file_name() {
            if let Some(dev_name) = dev_name.to_str() {
                Ok(String::from(dev_name))
            } else {
                Err(Error::with_context(
                    ErrorKind::InvParam,
                    &format!(
                        "Invalid characters found in device name '{}'",
                        path.display()
                    ),
                ))
            }
        } else {
            Err(Error::with_context(
                ErrorKind::InvParam,
                &format!("Failed to retrieve filename from path '{}'", path.display()),
            ))
        }
    }
}
