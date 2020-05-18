use crate::common::{path_append, MigErrCtx, MigError, MigErrorKind};

use failure::ResultExt;
use lazy_static::lazy_static;
use log::{debug, error, trace};
use nix::sys::stat::{major, minor, stat};
use regex::Regex;
use std::collections::HashMap;
use std::fs::{read_dir, read_to_string};
use std::path::{Path, PathBuf};
use std::rc::Rc;

mod mount;
use mount::{Mount, MountTab};

pub(crate) mod block_device;
pub(crate) use block_device::BlockDevice;

mod device;
use device::Device;

mod partition;
use partition::Partition;

// TODO: add mountpoints for  partitions

const BLOC_DEV_SUPP_MAJ_NUMBERS: [u64; 45] = [
    3, 8, 9, 21, 33, 34, 44, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 64, 65, 66, 67, 68, 69,
    70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 179, 180, 259,
];

type DeviceMap = HashMap<PathBuf, Rc<Box<dyn BlockDevice>>>;

#[derive(Clone)]
pub(crate) struct BlockDeviceInfo {
    root_device: Rc<Box<dyn BlockDevice>>,
    root_partition: Option<Rc<Box<dyn BlockDevice>>>,
    devices: DeviceMap,
}

impl BlockDeviceInfo {
    pub fn new() -> Result<BlockDeviceInfo, MigError> {
        let stat_res = stat("/").context(upstream_context!(&format!("Failed to stat root")))?;
        let root_major = major(stat_res.st_dev);
        let root_minor = minor(stat_res.st_dev);
        let mounts = Mount::from_mtab()?;

        debug!("new: Root device number is: {}:{}", root_major, root_minor);

        let sys_path = PathBuf::from("/sys/block/");
        let read_dir = read_dir(&sys_path).context(upstream_context!(&format!(
            "Failed to read directory '{}'",
            sys_path.display()
        )))?;

        let mut device_map: DeviceMap = DeviceMap::new();
        for entry in read_dir {
            match entry {
                Ok(entry) => {
                    let curr_path = entry.path();
                    let curr_dev = BlockDeviceInfo::path_to_device_name(&curr_path)?;
                    let (curr_major, curr_minor) = BlockDeviceInfo::get_maj_minor(&curr_path)?;
                    trace!(
                        "new: Looking at path '{}', device '{}' number: {}:{}",
                        curr_path.display(),
                        curr_dev,
                        curr_major,
                        curr_minor
                    );

                    if !BLOC_DEV_SUPP_MAJ_NUMBERS.contains(&curr_major) {
                        trace!(
                            "Skipping device '{}' with block device major {}",
                            curr_dev,
                            curr_major
                        );
                        continue;
                    }

                    let dev_path = path_append("/dev", &curr_dev);
                    if !dev_path.exists() {
                        error!("device path does not exist: '{}'", dev_path.display());
                        return Err(MigError::displayed());
                    }

                    let dev_path = path_append("/dev", &curr_dev);

                    // TODO: fill mounted
                    let device = Rc::new(Box::new(Device {
                        name: curr_dev,
                        major: curr_major,
                        minor: curr_minor,
                        mounted: None,
                    }) as Box<dyn BlockDevice>);

                    BlockDeviceInfo::read_partitions(
                        &device,
                        &mounts,
                        &curr_path,
                        &mut device_map,
                    )?;
                    device_map.insert(dev_path, device.clone());

                    debug!("new: got device: {:?}", device);
                }
                Err(why) => {
                    error!(
                        "Failed to read directory entry from '{}', error: {:?}",
                        sys_path.display(),
                        why
                    );
                    return Err(MigError::displayed());
                }
            }
        }

        let mut root_device: Option<Rc<Box<dyn BlockDevice>>> = None;
        let mut root_partition: Option<Rc<Box<dyn BlockDevice>>> = None;

        for (_dev_path, device_rc) in &device_map {
            let device = device_rc.as_ref();
            if (device.get_major() == root_major) && (device.get_minor() == root_minor) {
                if let Some(parent) = device.get_parent() {
                    root_device = Some(parent.clone());
                    root_partition = Some(device_rc.clone())
                } else {
                    root_device = Some(device_rc.clone());
                    root_partition = None;
                }
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

        error!("Failed to find root device");
        Err(MigError::displayed())
    }

    fn read_partitions<P: AsRef<Path>>(
        device: &Rc<Box<dyn BlockDevice>>,
        mounts: &MountTab,
        dev_path: P,
        device_map: &mut DeviceMap,
    ) -> Result<(), MigError> {
        let dev_path = dev_path.as_ref();
        let dir_entries = read_dir(dev_path).context(upstream_context!(&format!(
            "Failed to read directory '{}'",
            dev_path.display()
        )))?;

        for entry in dir_entries {
            match entry {
                Ok(entry) => {
                    let currdir = entry.path();
                    if entry
                        .metadata()
                        .context(upstream_context!(&format!(
                            "Failed toretrieve metadata for '{}'",
                            currdir.display()
                        )))?
                        .is_dir()
                    {
                        let part_name = BlockDeviceInfo::path_to_device_name(&currdir)?;

                        if !part_name.starts_with(&device.as_ref().get_name()) {
                            trace!("Skipping folder '{}", currdir.display());
                            continue;
                        }

                        let (curr_major, curr_minor) = BlockDeviceInfo::get_maj_minor(&currdir)?;
                        let dev_path = path_append("/dev", &part_name);

                        let mounted = if let Some(mount) = mounts.get(dev_path.as_path()) {
                            Some(mount.clone())
                        } else {
                            None
                        };

                        let partition = Rc::new(Box::new(Partition {
                            name: part_name,
                            major: curr_major,
                            minor: curr_minor,
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
                    error!(
                        "Failed to read directory entry from '{}', error {:?}",
                        dev_path.display(),
                        why
                    );
                    return Err(MigError::displayed());
                }
            }
        }

        Ok(())
    }

    pub fn get_root_device(&self) -> &Rc<Box<dyn BlockDevice>> {
        &self.root_device
    }

    pub fn get_root_partition(&self) -> &Option<Rc<Box<dyn BlockDevice>>> {
        &self.root_partition
    }

    pub fn get_devices(&self) -> &DeviceMap {
        &self.devices
    }

    fn get_maj_minor<P: AsRef<Path>>(dev_path: P) -> Result<(u64, u64), MigError> {
        lazy_static! {
            static ref DEVNUM_RE: Regex = Regex::new(r#"^(\d+):(\d+)$"#).unwrap();
        }

        let dev_info_path = path_append(dev_path.as_ref(), "dev");
        let dev_info = read_to_string(&dev_info_path).context(upstream_context!(&format!(
            "Failed to read file '{}'",
            dev_info_path.display()
        )))?;

        if let Some(captures) = DEVNUM_RE.captures(dev_info.as_str().trim()) {
            Ok((
                captures
                    .get(1)
                    .unwrap()
                    .as_str()
                    .parse::<u64>()
                    .context(upstream_context!(&format!(
                        "Failed to parse device major number from '{}'",
                        dev_info
                    )))?,
                captures
                    .get(2)
                    .unwrap()
                    .as_str()
                    .parse::<u64>()
                    .context(upstream_context!(&format!(
                        "Failed to parse major device major number from '{}'",
                        dev_info
                    )))?,
            ))
        } else {
            error!(
                "Failed to parse block device major:minor numbers from '{}', '{}'",
                dev_info_path.display(),
                dev_info
            );
            Err(MigError::displayed())
        }
    }

    /// extract last element of path as string
    fn path_to_device_name<P: AsRef<Path>>(path: P) -> Result<String, MigError> {
        let path = path.as_ref();
        if let Some(dev_name) = path.file_name() {
            if let Some(dev_name) = dev_name.to_str() {
                Ok(String::from(dev_name))
            } else {
                error!("Invalid device name '{}'", path.display());
                Err(MigError::displayed())
            }
        } else {
            error!("Failed to retrieve filename from path '{}'", path.display());
            Err(MigError::displayed())
        }
    }
}
