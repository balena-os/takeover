use failure::ResultExt;
use log::{debug, trace, warn};
use regex::Regex;

use std::path::{Path, PathBuf};

use crate::common::{
    call,
    defs::{DISK_BY_LABEL_PATH, DISK_BY_PARTUUID_PATH, DISK_BY_UUID_PATH, LSBLK_CMD},
    path_append, to_std_device_path, MigErrCtx, MigError, MigErrorKind,
};
use std::collections::HashMap;

// const GPT_EFI_PART: &str = "C12A7328-F81F-11D2-BA4B-00A0C93EC93B";

const BLOC_DEV_SUPP_MAJ_NUMBERS: [&str; 45] = [
    "3", "8", "9", "21", "33", "34", "44", "48", "49", "50", "51", "52", "53", "54", "55", "56",
    "57", "58", "64", "65", "66", "67", "68", "69", "70", "71", "72", "73", "74", "75", "76", "77",
    "78", "79", "80", "81", "82", "83", "84", "85", "86", "87", "179", "180", "259",
];

#[derive(Debug, Clone)]
pub(crate) struct LsblkPartition {
    pub name: String,
    pub kname: String,
    pub maj_min: String,
    pub ro: String,
    pub uuid: Option<String>,
    pub fstype: Option<String>,
    pub mountpoint: Option<PathBuf>,
    pub label: Option<String>,
    pub parttype: Option<String>,
    pub partlabel: Option<String>,
    pub partuuid: Option<String>,
    pub size: Option<u64>,
    pub index: Option<u16>,
}

#[allow(dead_code)]
impl LsblkPartition {
    pub fn get_path(&self) -> PathBuf {
        path_append("/dev", &self.name)
    }

    pub fn get_alt_path(&self) -> PathBuf {
        if let Some(ref partuuid) = self.partuuid {
            path_append(DISK_BY_PARTUUID_PATH, partuuid)
        } else if let Some(ref uuid) = self.uuid {
            path_append(DISK_BY_UUID_PATH, uuid)
        } else if let Some(ref label) = self.label {
            path_append(DISK_BY_LABEL_PATH, label)
        } else {
            path_append("/dev", &self.name)
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct LsblkDevice {
    pub name: String,
    pub kname: String,
    pub maj_min: String,
    pub uuid: Option<String>,
    pub size: Option<u64>,
    pub children: Option<Vec<LsblkPartition>>,
}

impl<'a> LsblkDevice {
    pub fn get_devinfo_from_part_name(
        &'a self,
        part_name: &str,
    ) -> Result<&'a LsblkPartition, MigError> {
        if let Some(ref children) = self.children {
            if let Some(part_info) = children.iter().find(|&part| part.name == part_name) {
                Ok(part_info)
            } else {
                Err(MigError::from_remark(
                    MigErrorKind::NotFound,
                    &format!(
                        "The partition was not found in lsblk output '{}'",
                        part_name
                    ),
                ))
            }
        } else {
            Err(MigError::from_remark(
                MigErrorKind::NotFound,
                &format!("The device was not found in lsblk output '{}'", part_name),
            ))
        }
    }

    pub fn get_path(&self) -> PathBuf {
        PathBuf::from(&format!("/dev/{}", self.name))
    }
}

#[derive(Debug)]
pub(crate) struct LsblkInfo {
    blockdevices: Vec<LsblkDevice>,
}

#[allow(dead_code)]
impl<'a> LsblkInfo {
    pub fn for_device(device: &Path) -> Result<LsblkDevice, MigError> {
        let lsblk_info = LsblkInfo::call_lsblk(Some(device))?;
        if lsblk_info.blockdevices.len() == 1 {
            Ok(lsblk_info.blockdevices[0].clone())
        } else {
            Err(MigError::from_remark(
                MigErrorKind::InvState,
                &format!(
                    "Invalid number of devices found for device query: {}",
                    lsblk_info.blockdevices.len()
                ),
            ))
        }
    }

    pub fn all() -> Result<LsblkInfo, MigError> {
        let mut lsblk_info = LsblkInfo::call_lsblk(None)?;

        // filter by maj block device numbers from https://www.kernel.org/doc/Documentation/admin-guide/devices.txt
        // other candidates:
        // 31 block	ROM/flash memory card
        // 45 block	Parallel port IDE disk devices
        // TODO: add more

        let maj_min_re = Regex::new(r#"^(\d+):\d+$"#).unwrap();

        lsblk_info.blockdevices.retain(|dev| {
            if let Some(captures) = maj_min_re.captures(&dev.maj_min) {
                let dev_maj = captures.get(1).unwrap().as_str();
                if let Some(_pos) = BLOC_DEV_SUPP_MAJ_NUMBERS
                    .iter()
                    .position(|&maj| maj == dev_maj)
                {
                    true
                } else {
                    debug!(
                        "rejecting device '{}', maj:min: '{}'",
                        dev.name, dev.maj_min
                    );
                    false
                }
            } else {
                warn!(
                    "Unable to parse device major/minor number from '{}'",
                    dev.maj_min
                );
                false
            }
        });

        debug!("lsblk_info: {:?}", lsblk_info);
        Ok(lsblk_info)
    }

    pub fn get_path_devs<P: AsRef<Path>>(
        &'a self,
        path: P,
    ) -> Result<(&'a LsblkDevice, &'a LsblkPartition), MigError> {
        let path = path.as_ref();
        debug!("get_path_devs: '{}", path.display());
        let abs_path = path.canonicalize().context(upstream_context!(&format!(
            "failed to canonicalize path: '{}'",
            path.display()
        )))?;

        let mut mp_match: Option<(&LsblkDevice, &LsblkPartition)> = None;

        for device in &self.blockdevices {
            trace!(
                "get_path_devs: looking at device '{}",
                device.get_path().display()
            );
            if let Some(ref children) = device.children {
                for part in children {
                    trace!(
                        "get_path_devs: looking at partition '{}",
                        part.get_path().display()
                    );
                    if let Some(ref mountpoint) = part.mountpoint {
                        trace!(
                            "Comparing search path '{}' to mountpoint '{}'",
                            abs_path.display(),
                            mountpoint.display()
                        );
                        if abs_path == PathBuf::from(mountpoint) {
                            trace!(
                                "get_path_devs: partition mountpoint is search path '{}'",
                                mountpoint.display()
                            );
                            return Ok((&device, part));
                        } else if abs_path.starts_with(mountpoint) {
                            trace!(
                                "get_path_devs: partition mountpoint starts with search path '{}'",
                                mountpoint.display()
                            );

                            if let Some((_last_dev, last_part)) = mp_match {
                                if let Some(ref last_mp) = last_part.mountpoint {
                                    if last_mp.to_string_lossy().len()
                                        < mountpoint.to_string_lossy().len()
                                    {
                                        trace!(
                                            "get_path_devs: new best match for '{}' -> '{}'",
                                            abs_path.display(),
                                            mountpoint.display()
                                        );
                                        mp_match = Some((&device, part))
                                    }
                                } else {
                                    trace!(
                                        "get_path_devs: new best match for '{}' -> '{}'",
                                        abs_path.display(),
                                        mountpoint.display()
                                    );
                                    mp_match = Some((&device, part))
                                }
                            } else {
                                trace!(
                                    "get_path_devs: first match for '{}' -> '{}'",
                                    abs_path.display(),
                                    mountpoint.display()
                                );
                                mp_match = Some((&device, part))
                            }
                        }
                    }
                }
            }
        }

        if let Some(res) = mp_match {
            Ok(res)
        } else {
            Err(MigError::from_remark(
                MigErrorKind::NotFound,
                &format!(
                    "A mountpoint could not be found for path: '{}'",
                    abs_path.display()
                ),
            ))
        }
    }

    pub fn get_blk_devices(&'a self) -> &'a Vec<LsblkDevice> {
        &self.blockdevices
    }

    // get the LsblkDevice & LsblkPartition from partition device path as in /dev/sda1
    pub fn get_devinfo_from_partition<P: AsRef<Path>>(
        &'a self,
        part_path: P,
    ) -> Result<(&'a LsblkDevice, &'a LsblkPartition), MigError> {
        let part_path = part_path.as_ref();
        trace!("get_devinfo_from_partition: '{}", part_path.display());

        let part_path = to_std_device_path(part_path)?;

        if let Some(part_name) = part_path.file_name() {
            let cmp_name = part_name.to_string_lossy();
            if let Some(lsblk_dev) = self
                .blockdevices
                .iter()
                .find(|&dev| cmp_name.starts_with(&dev.name))
            {
                Ok((lsblk_dev, lsblk_dev.get_devinfo_from_part_name(&cmp_name)?))
            } else {
                Err(MigError::from_remark(
                    MigErrorKind::NotFound,
                    &format!(
                        "The device was not found in lsblk output '{}'",
                        part_path.display()
                    ),
                ))
            }
        } else {
            Err(MigError::from_remark(
                MigErrorKind::InvParam,
                &format!("The device path is not valid '{}'", part_path.display()),
            ))
        }
    }

    fn call_lsblk(device: Option<&Path>) -> Result<LsblkInfo, MigError> {
        #[allow(unused_assignments)]
        let mut dev_name = String::new();
        let args = if let Some(device) = device {
            dev_name = String::from(&*device.to_string_lossy());
            vec![
                "-b",
                "-P",
                "-o",
                "NAME,KNAME,MAJ:MIN,FSTYPE,MOUNTPOINT,LABEL,UUID,RO,SIZE,TYPE",
                dev_name.as_str(),
            ]
        } else {
            vec![
                "-b",
                "-P",
                "-o",
                "NAME,KNAME,MAJ:MIN,FSTYPE,MOUNTPOINT,LABEL,UUID,RO,SIZE,TYPE",
            ]
        };

        let cmd_res = call(LSBLK_CMD, &args, true)?;
        if cmd_res.status.success() {
            Ok(LsblkInfo::from_list(&cmd_res.stdout)?)
        } else {
            Err(MigError::from_remark(
                MigErrorKind::ExecProcess,
                "new: failed to determine block device attributes for",
            ))
        }
    }

    fn from_list(list: &str) -> Result<LsblkInfo, MigError> {
        let param_re = Regex::new(r##"^([\S^=]+)="([^"]*)"(\s+(.*))?$"##).unwrap();

        let mut lsblk_info: LsblkInfo = LsblkInfo {
            blockdevices: Vec::new(),
        };

        let mut curr_dev: Option<LsblkDevice> = None;

        for line in list.lines() {
            trace!("from_list: processing line: '{}'", line);
            let mut curr_pos = line;
            let mut params: HashMap<String, String> = HashMap::new();

            let get_str = |p: &HashMap<String, String>, s: &str| -> Result<String, MigError> {
                if let Some(res) = p.get(s) {
                    Ok(res.clone())
                } else {
                    Err(MigError::from_remark(
                        MigErrorKind::NotFound,
                        &format!("Parameter '{}' not found in '{}'", s, line),
                    ))
                }
            };

            let get_u64 = |p: &HashMap<String, String>, s: &str| -> Result<Option<u64>, MigError> {
                if let Some(res) = p.get(s) {
                    Ok(Some(res.parse::<u64>().context(upstream_context!(
                        &format!("Failed to parse u64 from '{}'", s)
                    ))?))
                } else {
                    Ok(None)
                }
            };

            let get_pathbuf_or_none = |p: &HashMap<String, String>, s: &str| -> Option<PathBuf> {
                if let Some(res) = p.get(s) {
                    if res.is_empty() {
                        None
                    } else {
                        Some(PathBuf::from(res))
                    }
                } else {
                    None
                }
            };

            // parse current line into hashmap
            loop {
                trace!("looking at '{}'", curr_pos);
                if let Some(captures) = param_re.captures(curr_pos) {
                    let param_name = captures.get(1).unwrap().as_str();
                    let param_value = captures.get(2).unwrap().as_str();

                    if !param_value.is_empty() {
                        params.insert(String::from(param_name), String::from(param_value));
                    }

                    if let Some(ref rest) = captures.get(4) {
                        curr_pos = rest.as_str();
                        trace!(
                            "Found param: '{}', value '{}', rest '{}'",
                            param_name,
                            param_value,
                            curr_pos
                        );
                    } else {
                        trace!(
                            "Found param: '{}', value '{}', rest None",
                            param_name,
                            param_value
                        );
                        break;
                    }
                } else {
                    warn!("Failed to parse '{}'", curr_pos);
                    break;
                }
            }

            let dev_type = get_str(&params, "TYPE")?;

            trace!("got type: '{}'", dev_type);

            match dev_type.as_str() {
                "disk" => {
                    if let Some(curr_dev) = curr_dev {
                        lsblk_info.blockdevices.push(curr_dev);
                    }

                    curr_dev = Some(LsblkDevice {
                        name: get_str(&params, "NAME")?,
                        kname: get_str(&params, "KNAME")?,
                        maj_min: get_str(&params, "MAJ:MIN")?,
                        uuid: if let Some(uuid) = params.get("UUID") {
                            Some(uuid.clone())
                        } else {
                            None
                        },
                        size: get_u64(&params, "SIZE")?,
                        children: None,
                    });
                }
                "part" => {
                    if let Some(ref mut curr_dev) = curr_dev {
                        let children = if let Some(ref mut children) = curr_dev.children {
                            children
                        } else {
                            curr_dev.children = Some(Vec::new());
                            curr_dev.children.as_mut().unwrap()
                        };

                        children.push(LsblkPartition {
                            name: get_str(&params, "NAME")?,
                            kname: get_str(&params, "KNAME")?,
                            maj_min: get_str(&params, "MAJ:MIN")?,
                            fstype: if let Some(fstype) = params.get("FSTYPE") {
                                Some(fstype.clone())
                            } else {
                                None
                            },
                            mountpoint: get_pathbuf_or_none(&params, "MOUNTPOINT"),
                            label: if let Some(label) = params.get("LABEL") {
                                Some(label.clone())
                            } else {
                                None
                            },
                            uuid: if let Some(uuid) = params.get("UUID") {
                                Some(uuid.clone())
                            } else {
                                None
                            },
                            ro: get_str(&params, "RO")?,
                            size: get_u64(&params, "SIZE")?,
                            parttype: None,
                            partlabel: None,
                            partuuid: None,
                            // TODO: bit dodgy this one
                            index: Some((children.len() + 1) as u16),
                        });
                    } else {
                        return Err(MigError::from_remark(
                            MigErrorKind::InvState,
                            &format!(
                                "Invalid state while parsing lsblk output line '{}', no device",
                                line
                            ),
                        ));
                    };
                }

                _ => debug!("not processing line, type unknown: '{}'", line),
            }
        }

        if let Some(curr_dev) = curr_dev {
            lsblk_info.blockdevices.push(curr_dev);
            // curr_dev = None;
        }

        Ok(lsblk_info)
    }
}

#[cfg(test)]
mod tests {
    use crate::stage1::lsblk_info::LsblkInfo;

    const LSBLK_OUTPUT1: &str = r##"NAME="loop0" KNAME="loop0" MAJ:MIN="7:0" FSTYPE="squashfs" MOUNTPOINT="/snap/core/7270" LABEL="" UUID="" RO="1" SIZE="92778496" TYPE="loop"
NAME="loop1" KNAME="loop1" MAJ:MIN="7:1" FSTYPE="squashfs" MOUNTPOINT="/snap/core18/1066" LABEL="" UUID="" RO="1" SIZE="57069568" TYPE="loop"
NAME="loop2" KNAME="loop2" MAJ:MIN="7:2" FSTYPE="squashfs" MOUNTPOINT="/snap/core18/1074" LABEL="" UUID="" RO="1" SIZE="57069568" TYPE="loop"
NAME="loop3" KNAME="loop3" MAJ:MIN="7:3" FSTYPE="squashfs" MOUNTPOINT="/snap/gnome-3-28-1804/71" LABEL="" UUID="" RO="1" SIZE="157192192" TYPE="loop"
NAME="loop4" KNAME="loop4" MAJ:MIN="7:4" FSTYPE="squashfs" MOUNTPOINT="/snap/core/7396" LABEL="" UUID="" RO="1" SIZE="92983296" TYPE="loop"
NAME="loop5" KNAME="loop5" MAJ:MIN="7:5" FSTYPE="squashfs" MOUNTPOINT="/snap/gnome-logs/61" LABEL="" UUID="" RO="1" SIZE="1032192" TYPE="loop"
NAME="loop6" KNAME="loop6" MAJ:MIN="7:6" FSTYPE="squashfs" MOUNTPOINT="/snap/gtk-common-themes/1313" LABEL="" UUID="" RO="1" SIZE="44879872" TYPE="loop"
NAME="loop7" KNAME="loop7" MAJ:MIN="7:7" FSTYPE="squashfs" MOUNTPOINT="/snap/vlc/1049" LABEL="" UUID="" RO="1" SIZE="212713472" TYPE="loop"
NAME="loop8" KNAME="loop8" MAJ:MIN="7:8" FSTYPE="squashfs" MOUNTPOINT="/snap/gnome-3-28-1804/67" LABEL="" UUID="" RO="1" SIZE="157184000" TYPE="loop"
NAME="loop9" KNAME="loop9" MAJ:MIN="7:9" FSTYPE="squashfs" MOUNTPOINT="/snap/gnome-system-monitor/100" LABEL="" UUID="" RO="1" SIZE="3825664" TYPE="loop"
NAME="loop10" KNAME="loop10" MAJ:MIN="7:10" FSTYPE="squashfs" MOUNTPOINT="/snap/gtk2-common-themes/5" LABEL="" UUID="" RO="1" SIZE="135168" TYPE="loop"
NAME="loop11" KNAME="loop11" MAJ:MIN="7:11" FSTYPE="squashfs" MOUNTPOINT="/snap/gimp/189" LABEL="" UUID="" RO="1" SIZE="229728256" TYPE="loop"
NAME="loop12" KNAME="loop12" MAJ:MIN="7:12" FSTYPE="squashfs" MOUNTPOINT="/snap/spotify/36" LABEL="" UUID="" RO="1" SIZE="189870080" TYPE="loop"
NAME="loop13" KNAME="loop13" MAJ:MIN="7:13" FSTYPE="squashfs" MOUNTPOINT="/snap/gnome-characters/296" LABEL="" UUID="" RO="1" SIZE="15462400" TYPE="loop"
NAME="loop14" KNAME="loop14" MAJ:MIN="7:14" FSTYPE="squashfs" MOUNTPOINT="/snap/gnome-calculator/406" LABEL="" UUID="" RO="1" SIZE="4218880" TYPE="loop"
NAME="nvme0n1" KNAME="nvme0n1" MAJ:MIN="259:0" FSTYPE="" MOUNTPOINT="" LABEL="" UUID="" RO="0" SIZE="512110190592" TYPE="disk"
NAME="nvme0n1p1" KNAME="nvme0n1p1" MAJ:MIN="259:1" FSTYPE="vfat" MOUNTPOINT="/boot/efi" LABEL="ESP SPACE" UUID="42D3-AAB8" RO="0" SIZE="713031680" TYPE="part"
NAME="nvme0n1p2" KNAME="nvme0n1p2" MAJ:MIN="259:2" FSTYPE="" MOUNTPOINT="" LABEL="" UUID="" RO="0" SIZE="134217728" TYPE="part"
NAME="nvme0n1p3" KNAME="nvme0n1p3" MAJ:MIN="259:3" FSTYPE="" MOUNTPOINT="" LABEL="" UUID="" RO="0" SIZE="79322677248" TYPE="part"
NAME="nvme0n1p4" KNAME="nvme0n1p4" MAJ:MIN="259:4" FSTYPE="ntfs" MOUNTPOINT="" LABEL="WINRETOOLS" UUID="500EC0840EC06516" RO="0" SIZE="1038090240" TYPE="part"
NAME="nvme0n1p5" KNAME="nvme0n1p5" MAJ:MIN="259:5" FSTYPE="ntfs" MOUNTPOINT="" LABEL="Image mit Space" UUID="C614C0AC14C0A0B3" RO="0" SIZE="10257170432" TYPE="part"
NAME="nvme0n1p6" KNAME="nvme0n1p6" MAJ:MIN="259:6" FSTYPE="ntfs" MOUNTPOINT="" LABEL="DELLSUPPORT" UUID="AA88E9D888E9A2D5" RO="0" SIZE="1212153856" TYPE="part"
NAME="nvme0n1p7" KNAME="nvme0n1p7" MAJ:MIN="259:7" FSTYPE="ext4" MOUNTPOINT="/" LABEL="" UUID="b305522d-faa7-49fc-a7d1-70dae48bcc3e" RO="0" SIZE="419430400000" TYPE="part"
"##;

    #[test]
    fn read_output_ok1() {
        let _lsblk_info = LsblkInfo::from_list(LSBLK_OUTPUT1).unwrap();
    }
}
