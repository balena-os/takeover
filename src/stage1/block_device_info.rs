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
// For an LVM volume we expect a block device to use a major in the experimental/local range.
// For now, conservatively requiring a value known to be used.
// See https://www.kernel.org/doc/Documentation/admin-guide/devices.txt
const LVM_DEV_MAJOR_NUMBER: u64 = 254;

type DeviceMap = HashMap<PathBuf, Rc<dyn BlockDevice>>;

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

/// Information about block devices for a host, including the "root" device.
///
/// The 'devices' member includes the devices themselves as well as partitions
/// defined on them.
#[derive(Clone)]
pub(crate) struct BlockDeviceInfo {
    root_device: Rc<dyn BlockDevice>,
    root_partition: Option<Rc<dyn BlockDevice>>,
    devices: DeviceMap,
}

impl BlockDeviceInfo {
    /// Create a BlockDeviceInfo, which uses the root directory to identify the
    /// partitions that must be unmounted to permit reflashing to the underlying
    /// storage device.
    ///
    /// Requires a hint to expect the root directory is mounted as an LVM logical
    /// volume.
    pub fn new(is_lvm_root: bool) -> Result<BlockDeviceInfo> {
        BlockDeviceInfo::new_for_dir("/", is_lvm_root)
    }

    /// Create a BlockDeviceInfo where the provided directory is mounted on a
    /// partition that must be unmounted to permit reflashing to the underlying
    /// storage device.
    ///
    /// Reviews all devices as found in `/sys/block`. The device major number must
    /// be included in BLOC_DEV_SUPP_MAJ_NUMBERS. For the devices member of the
    /// info struct, includes the Partition instances for devices as well as the
    /// Device instances themselves. Both implement the BlockDevice trait.
    ///
    /// Typically finds the device and partition for the provided directory, and
    /// specifies them in the returned struct. However may not find them, for
    /// example when root directory is on a partition managed by LVM. In this
    /// case the root device and partition in the returned struct will be None.
    pub fn new_for_dir(dir: &str, is_lvm_root: bool) -> Result<BlockDeviceInfo> {
        let stat_res = stat(dir).upstream_with_context(&format!("Failed to stat for {}", dir))?;
        let root_number = DeviceNum::new(stat_res.st_dev);
        // Collect mapping of /dev mounts to mountpoints from /etc/mtab.
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
                        // We expect LVM root is 254:0, but other minors that pass
                        // through here should get filtered out below.
                        if is_lvm_root && curr_number.major() == LVM_DEV_MAJOR_NUMBER {
                            trace!(
                                "Possible LVM root device '{}' with block device major {}",
                                curr_dev,
                                curr_number.major()
                            );
                        } else {
                            trace!(
                                "Skipping device '{}' with block device major {}",
                                curr_dev,
                                curr_number.major()
                            );
                            continue;
                        }
                    }

                    let dev_path = path_append("/dev", &curr_dev);
                    if !dev_path.exists() {
                        return Err(Error::with_context(
                            ErrorKind::DeviceNotFound,
                            &format!("device path does not exist: '{}'", dev_path.display()),
                        ));
                    }

                    // TODO: fill mounted
                    // Presently, likely to be None. The 'mounts' map is keyed
                    // on a partition entry not a device entry, but we expect device
                    // entries from /sys/block.
                    let mounted: Option<Mount> = if let Some(mount) = mounts.get(&dev_path) {
                        Some(mount.clone())
                    } else if root_number == curr_number {
                        // If LVM root, use the mount for the provided dir, if any.
                        // Likely is not keyed on dev_path, and so not handled
                        // in the preceding 'if'.
                        // For example, dev_path: /dev/dm-0 (from /sys/block/dm-0);
                        //              mount key: /dev/mapper/debnuc--vg-root
                        // The search here allows the root LVM volume to serve as
                        // both a device and a partition for our purposes. The call
                        // below for read_partitions() will not find partitions for
                        // a logical volume like /dev/dm-0.
                        if is_lvm_root {
                            let mut root_mount: Option<Mount> = None;
                            for mount in mounts.values() {
                                if mount.get_mountpoint() == Path::new(dir) {
                                    root_mount = Some(mount.clone());
                                    trace!("new: found root mount for LVM root");
                                }
                            }
                            root_mount
                        } else {
                            mounts.get(PathBuf::from("/dev/root").as_path()).cloned()
                        }
                    } else {
                        None
                    };

                    // Create the Device and its Partition entries.
                    let device = Rc::new(Device {
                        name: curr_dev,
                        device_num: curr_number,
                        mounted,
                    }) as Rc<dyn BlockDevice>;

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

        let mut root_device: Option<Rc<dyn BlockDevice>> = None;
        let mut root_partition: Option<Rc<dyn BlockDevice>> = None;

        // Find the partition in the device map whose device major:minor matches
        // the major:minor for the directory provided to this method.
        for device_rc in device_map.values_mut() {
            let device = device_rc.as_ref();
            if device.get_device_num() == &root_number {
                // Handling for the partition that matches device major/minor
                // for the directory provided to this method.
                if let Some(parent) = device.get_parent() {
                    // partition entry handling
                    root_device = Some(parent.clone());
                    root_partition = Some(device_rc.clone())

                // If LVM root, should have found the root directory above.
                } else if is_lvm_root {
                    if let Some(mp) = device_rc.get_mountpoint() {
                        if mp.get_mountpoint() == Path::new("/") {
                            root_device = Some(device_rc.clone());
                            root_partition = Some(device_rc.clone())
                        }
                    }
                } else {
                    // device entry handling; not sure when this path is used
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

    /// Read the partitions for the given device, where a valid path for a partition
    /// is like "/sys/block/{device}/{device}p...", and add them as Partition
    /// instances to the provided device_map.
    ///
    /// Example:
    ///   dev_path: /dev/nvme0n1
    ///   partitions: nvme0n1p1, nvme0n1p2, nvme0n1p3
    fn read_partitions<P: AsRef<Path>>(
        device: &Rc<dyn BlockDevice>,
        mounts: &MountTab,
        dev_path: P,
        root_number: &DeviceNum,
        device_map: &mut DeviceMap,
    ) -> Result<()> {
        trace!(
            "read_partitions: for device: {} dev_path: {}",
            device.get_name(),
            dev_path.as_ref().display()
        );

        let dev_path = dev_path.as_ref();
        let dir_entries = read_dir(dev_path).upstream_with_context(&format!(
            "Failed to read directory '{}'",
            dev_path.display()
        ))?;

        let regex_str = format!(r"^{}p?\d+$", device.get_name());
        let part_regex = Regex::new(regex_str.as_str())
            .upstream_with_context(&format!("Failed to create regex from '{}'", regex_str))?;

        for entry in dir_entries {
            match entry {
                Ok(entry) => {
                    let currdir = entry.path();
                    if entry
                        .metadata()
                        .upstream_with_context(&format!(
                            "Failed to retrieve metadata for '{}'",
                            currdir.display()
                        ))?
                        .is_dir()
                    {
                        let part_name = BlockDeviceInfo::path_filename_as_string(&currdir)?;

                        if !part_regex.is_match(part_name.as_str()) {
                            trace!("new: Skipping folder '{}", currdir.display());
                            continue;
                        }

                        let curr_number = BlockDeviceInfo::get_maj_minor(&currdir)?;
                        let dev_path = path_append("/dev", &part_name);

                        let mounted = if let Some(mount) = mounts.get(dev_path.as_path()) {
                            Some(mount.clone())
                        } else if curr_number == *root_number {
                            mounts.get(PathBuf::from("/dev/root").as_path()).cloned()
                        } else {
                            None
                        };

                        let partition = Rc::new(Partition::new(
                            part_name.as_str(),
                            curr_number,
                            mounted,
                            device.clone(),
                        )?) as Rc<dyn BlockDevice>;

                        debug!(
                            "read_partitions: found partition '{:?}' in '{}'",
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

    pub fn get_root_device(&self) -> &Rc<dyn BlockDevice> {
        &self.root_device
    }

    #[allow(dead_code)]
    pub fn get_root_partition(&self) -> &Option<Rc<dyn BlockDevice>> {
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

        DeviceNum::from_str(dev_info.as_str())
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
