use crate::common::path_append;
use crate::common::{MigErrCtx, MigError, MigErrorKind};
use failure::ResultExt;
use lazy_static::lazy_static;
use log::{debug, error, trace};
use nix::sys::stat::{major, minor, stat};
use regex::Regex;
use std::collections::HashMap;
use std::fs::read_dir;
use std::fs::read_to_string;
use std::path::{Path, PathBuf};

// TODO: add mountpoints for  partitions

const BLOC_DEV_SUPP_MAJ_NUMBERS: [u64; 45] = [
    3, 8, 9, 21, 33, 34, 44, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 64, 65, 66, 67, 68, 69,
    70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 179, 180, 259,
];

#[derive(Clone, Debug)]
pub(crate) struct Mount {
    mountpoint: PathBuf,
    fs_type: String,
}

impl Mount {
    pub fn get_mountpoint(&self) -> &Path {
        self.mountpoint.as_path()
    }

    pub fn get_fs_type(&self) -> &str {
        self.fs_type.as_str()
    }
}

type MountTab = HashMap<PathBuf, Mount>;

impl Mount {
    pub fn from_mtab() -> Result<MountTab, MigError> {
        let mtab_str = read_to_string("/etc/mtab")
            .context(upstream_context!("Failed to read from '/etc/mtab'"))?;

        let mut mounts: MountTab = MountTab::new();

        for (line_no, line) in mtab_str.lines().enumerate() {
            let columns: Vec<&str> = line.split_whitespace().collect();
            if columns.len() < 3 {
                error!("Failed to parse /etc/mtab line {} : '{}'", line_no, line);
                return Err(MigError::displayed());
            }

            let device_name = columns[0];
            if device_name.starts_with("/dev/") {
                let mount = Mount {
                    mountpoint: PathBuf::from(columns[1]),
                    fs_type: columns[2].to_string(),
                };

                debug!("from_mtab: processing mount {:?}", mount);
                mounts.insert(PathBuf::from(device_name), mount);
            } else {
                trace!("from_mtab: not processing line {}", line);
            }
        }

        Ok(mounts)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Partition {
    name: String,
    major: u64,
    minor: u64,
    mounted: Option<Mount>,
}

impl Partition {
    pub fn get_major(&self) -> u64 {
        self.major
    }
    pub fn get_minor(&self) -> u64 {
        self.minor
    }

    pub fn get_mountpoint(&self) -> &Option<Mount> {
        &self.mounted
    }
}

#[derive(Clone, Debug)]
pub(crate) struct BlockDevice {
    name: String,
    dev_path: PathBuf,
    major: u64,
    minor: u64,
    partitions: HashMap<PathBuf, Partition>,
}

impl BlockDevice {
    pub fn get_dev_path(&self) -> &Path {
        &self.dev_path
    }

    pub fn get_partitions(&self) -> &HashMap<PathBuf, Partition> {
        &self.partitions
    }
}

#[derive(Clone, Debug)]
pub(crate) struct BlockDeviceInfo {
    root_device_idx: usize,
    root_partition: PathBuf,
    devices: Vec<BlockDevice>,
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

        let mut device_list: Vec<BlockDevice> = Vec::new();
        let mut root_device_idx: Option<usize> = None;
        let mut root_partition: Option<PathBuf> = None;
        let mut device_idx: usize = 0;
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

                    let partitions =
                        BlockDeviceInfo::read_partitions(&curr_dev, &mounts, &curr_path)?;
                    let dev_path = path_append("/dev", &curr_dev);

                    if !dev_path.exists() {
                        error!("device path does not exist: '{}'", dev_path.display());
                        return Err(MigError::displayed());
                    }

                    let mut device = BlockDevice {
                        name: curr_dev,
                        dev_path,
                        major: curr_major,
                        minor: curr_minor,
                        partitions,
                    };

                    debug!("new: got device: {:?}", device);

                    if root_partition.is_none() {
                        for (idx, (dev_path, partition)) in device.partitions.iter_mut().enumerate()
                        {
                            if (partition.get_major() == root_major)
                                && (partition.get_minor() == root_minor)
                            {
                                debug!(
                                    "new: device: {}, number: {}:{} is root device",
                                    dev_path.display(),
                                    partition.major,
                                    partition.minor
                                );

                                if partition.mounted.is_none() {
                                    if let Some(mount) =
                                        mounts.get(PathBuf::from("/dev/root").as_path())
                                    {
                                        partition.mounted = Some(mount.clone())
                                    } else {
                                        error!(
                                            "detected root partition '{}' is not mounted",
                                            dev_path.display()
                                        );
                                        return Err(MigError::displayed());
                                    }
                                }
                                // TODO: check mountpoint
                                root_device_idx = Some(device_idx);
                                root_partition = Some(dev_path.to_path_buf());
                            }
                        }
                    }
                    device_list.push(device);
                    device_idx += 1;
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

        if let Some(root_device_idx) = root_device_idx {
            if let Some(root_partition) = root_partition {
                return Ok(BlockDeviceInfo {
                    root_device_idx,
                    root_partition,
                    devices: device_list,
                });
            }
        }

        error!("Failed to find root device");
        Err(MigError::displayed())
    }

    fn read_partitions<P: AsRef<Path>>(
        dev_name: &str,
        mounts: &MountTab,
        dev_path: P,
    ) -> Result<HashMap<PathBuf, Partition>, MigError> {
        let dev_path = dev_path.as_ref();
        let dir_entries = read_dir(dev_path).context(upstream_context!(&format!(
            "Failed to read directory '{}'",
            dev_path.display()
        )))?;

        let mut part_list: HashMap<PathBuf, Partition> = HashMap::new();
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
                        if !part_name.starts_with(dev_name) {
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

                        let partition = Partition {
                            name: part_name,
                            major: curr_major,
                            minor: curr_minor,
                            mounted,
                        };

                        debug!(
                            "found  partition '{:?}' in '{}'",
                            partition,
                            currdir.display(),
                        );

                        part_list.insert(dev_path, partition);
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

        Ok(part_list)
    }

    pub fn get_root_device(&self) -> &BlockDevice {
        &self.devices[self.root_device_idx]
    }

    pub fn get_root_partition(&self) -> &Partition {
        &self.devices[self.root_device_idx]
            .partitions
            .get(&self.root_partition)
            .unwrap()
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
