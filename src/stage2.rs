use std::fs::{
    copy, create_dir, create_dir_all, read_dir, read_to_string, remove_dir, File, OpenOptions,
};
use std::io::{self, Read, Write};
use finder::Finder;
use std::os::unix::io::AsRawFd;
use std::process::{exit, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use nix::{
    mount::{mount, umount, MsFlags},
    unistd::sync,
};

use std::path::{Path, PathBuf};

use flate2::read::GzDecoder;
use libc::{ioctl, LINUX_REBOOT_CMD_RESTART, MS_RDONLY, MS_REMOUNT, SIGKILL, SIGTERM};
use log::{debug, error, info, trace, warn, Level};
use mod_logger::{LogDestination, Logger, NO_STREAM};

use crate::common::defs::MTD_DEBUG_CMD;
use crate::common::stage2_config::LogDevice;

use crate::common::{
    call,
    defs::{
        IoctlReq, BACKUP_ARCH_NAME, BALENA_BOOT_FSTYPE, BALENA_BOOT_MP, BALENA_BOOT_PART,
        BALENA_CONFIG_PATH, BALENA_DATA_FSTYPE, BALENA_DATA_PART, BALENA_ROOTA_FSTYPE, BALENA_IMAGE_NAME,
        BALENA_IMAGE_PATH, BALENA_PART_MP, DD_CMD, DISK_BY_LABEL_PATH, EFIBOOTMGR_CMD, NIX_NONE,
        OLD_ROOT_MP, STAGE2_CONFIG_NAME, SYSTEM_CONNECTIONS_DIR, SYS_EFI_DIR, JETSON_XAVIER_HW_PART_FORCE_RO_FILE, SYSTEM_PROXY_DIR,
        BOOT_BLOB_NAME_JETSON_XAVIER, BOOT_BLOB_NAME_JETSON_XAVIER_NX, BOOT_BLOB_PARTITION_JETSON_XAVIER, BOOT_BLOB_PARTITION_JETSON_XAVIER_NX
    },
    dir_exists,
    disk_util::{Disk, LabelType, PartInfo, PartitionIterator, DEF_BLOCK_SIZE},
    error::{Error, ErrorKind, Result, ToError},
    file_exists, format_size_with_unit, get_mem_info,
    loop_device::LoopDevice,
    options::Options,
    path_append,
    stage2_config::{Stage2Config, UmountPart},
    system::{fuser, get_process_infos},
};
use regex::Regex;

const DD_BLOCK_SIZE: usize = 128 * 1024; // 4_194_304;
const JETSON_XAVIER_NX_QSPI_SIZE: &str = "0x2000000";
const VALIDATE_MAX_ERR: usize = 20;
const DO_VALIDATE: bool = false;
const VALIDATE_BLOCK_SIZE: usize = 64 * 1024; // 4_194_304;

const IOCTL_BLK_RRPART: IoctlReq = 0x1295;

const TRANSFER_DIR: &str = "/transfer";

const S2_XTRA_FS_SIZE: u64 = 10 * 1024 * 1024;

pub(crate) fn reboot() -> ! {
    trace!("reboot entered");
    Logger::flush();
    sync();
    sleep(Duration::from_secs(3));
    info!("rebooting");
    let _res = unsafe { libc::reboot(LINUX_REBOOT_CMD_RESTART) };
    exit(1);
}

fn get_required_space(s2_cfg: &Stage2Config) -> Result<u64> {
    let curr_file = path_append(OLD_ROOT_MP, &s2_cfg.image_path);
    let mut req_size = curr_file
        .metadata()
        .upstream_with_context(&format!(
            "Failed to retrieve imagesize for '{}'",
            curr_file.display()
        ))?
        .len();

    let curr_file = path_append(OLD_ROOT_MP, &s2_cfg.config_path);
    req_size += curr_file
        .metadata()
        .upstream_with_context(&format!(
            "Failed to retrieve file size for '{}'",
            curr_file.display()
        ))?
        .len();

    if let Some(ref backup_path) = s2_cfg.backup_path {
        let curr_file = path_append(OLD_ROOT_MP, backup_path);
        req_size += curr_file
            .metadata()
            .upstream_with_context(&format!(
                "Failed to retrieve file size for '{}'",
                curr_file.display()
            ))?
            .len();
    }

    let nwmgr_path = path_append(
        OLD_ROOT_MP,
        path_append(&s2_cfg.work_dir, SYSTEM_CONNECTIONS_DIR),
    );

    for dir_entry in read_dir(&nwmgr_path).upstream_with_context(&format!(
        "Failed to read drectory '{}'",
        nwmgr_path.display()
    ))? {
        match dir_entry {
            Ok(dir_entry) => {
                req_size += dir_entry
                    .path()
                    .metadata()
                    .upstream_with_context(&format!(
                        "Failed to retrieve file size for: '{}'",
                        dir_entry.path().display()
                    ))?
                    .len()
            }
            Err(why) => {
                return Err(Error::from_upstream(
                    Box::new(why),
                    &format!(
                        "Failed to retrieve directory entry for '{}'",
                        nwmgr_path.display()
                    ),
                ));
            }
        }
    }
    Ok(req_size)
}

fn copy_files(s2_cfg: &Stage2Config) -> Result<()> {
    let (mem_tot, mem_free) = get_mem_info()?;
    info!(
        "Found {} total, {} free memory",
        format_size_with_unit(mem_tot),
        format_size_with_unit(mem_free)
    );

    let req_space = get_required_space(s2_cfg)?;

    if mem_free < req_space + S2_XTRA_FS_SIZE {
        error!(
            "Not enough memory space found to copy files to RAMFS, required size is {} free memory is {}",
            format_size_with_unit(req_space + S2_XTRA_FS_SIZE),
            format_size_with_unit(mem_free)
        );
        return Err(Error::displayed());
    }

    // TODO: check free mem against files to copy

    if !dir_exists(TRANSFER_DIR)? {
        create_dir(TRANSFER_DIR).upstream_with_context(&format!(
            "Failed to create transfer directory: '{}'",
            TRANSFER_DIR
        ))?;
    }

    // *********************************************************
    // write balena image to tmpfs

    let src_path = path_append(OLD_ROOT_MP, &s2_cfg.image_path);
    let to_path = path_append(TRANSFER_DIR, BALENA_IMAGE_NAME);
    copy(&src_path, &to_path).upstream_with_context(&format!(
        "Failed to copy '{}' to {}",
        src_path.display(),
        &to_path.display()
    ))?;
    info!("Copied image from {} to '{}'", src_path.display(), to_path.display());

    let src_path = path_append(OLD_ROOT_MP, &s2_cfg.config_path);
    let to_path = path_append(TRANSFER_DIR, BALENA_CONFIG_PATH);
    copy(&src_path, &to_path).upstream_with_context(&format!(
        "Failed to copy '{}' to {}",
        src_path.display(),
        &to_path.display()
    ))?;
    info!("Copied config to '{}'", to_path.display());

    if let Some(ref backup_path) = s2_cfg.backup_path {
        let src_path = path_append(OLD_ROOT_MP, backup_path);
        let to_path = path_append(TRANSFER_DIR, BACKUP_ARCH_NAME);
        copy(&src_path, &to_path).upstream_with_context(&format!(
            "Failed to copy '{}' to {}",
            src_path.display(),
            &to_path.display()
        ))?;
        info!("Copied backup to '{}'", to_path.display());
    }

    /* Copy system connection and system proxy files over to the new install */
    let system_config_dirs = vec![SYSTEM_CONNECTIONS_DIR, SYSTEM_PROXY_DIR];

    for system_config_dir in system_config_dirs.into_iter() {
        let config_file_path = path_append(
            OLD_ROOT_MP,
            path_append(&s2_cfg.work_dir, system_config_dir),
        );

        let to_dir = path_append(TRANSFER_DIR, system_config_dir);
        if !dir_exists(&to_dir)? {
            create_dir_all(&to_dir).upstream_with_context(&format!(
                "Failed to create directory: '{}'",
                to_dir.display()
            ))?;
        }

        for dir_entry in read_dir(&config_file_path).upstream_with_context(&format!(
            "Failed to read drectory '{}'",
            config_file_path.display()
        ))? {
            match dir_entry {
                Ok(dir_entry) => {
                    if let Some(filename) = dir_entry.path().file_name() {
                        let to_path = path_append(&to_dir, filename);
                        copy(dir_entry.path(), &to_path).upstream_with_context(&format!(
                            "Failed to copy '{}' to '{}'",
                            dir_entry.path().display(),
                            to_path.display()
                        ))?;
                        info!("Copied network config to '{}'", to_path.display());
                    } else {
                        return Err(Error::with_context(
                            ErrorKind::InvParam,
                            &format!(
                                "Failed to extract filename from path: '{}'",
                                dir_entry.path().display()
                            ),
                        ));
                    }
                }
                Err(why) => {
                    return Err(Error::from_upstream(
                        Box::new(why),
                        &format!(
                            "Failed to retrieve directory entry for '{}'",
                            config_file_path.display()
                        ),
                    ));
                }
            }
        }
    }

    Ok(())
}

pub(crate) fn read_stage2_config<P: AsRef<Path>>(path_prefix: Option<P>) -> Result<Stage2Config> {
    let s2_cfg_path = if let Some(path_prefix) = path_prefix {
        path_append(path_prefix, STAGE2_CONFIG_NAME)
    } else {
        PathBuf::from(STAGE2_CONFIG_NAME)
    };

    if file_exists(&s2_cfg_path) {
        let s2_cfg_txt = read_to_string(&s2_cfg_path).upstream_with_context(&format!(
            "Failed to read stage 2 config from '{}'",
            s2_cfg_path.display(),
        ))?;

        Ok(Stage2Config::deserialze(&s2_cfg_txt)
            .upstream_with_context("Failed to deserialize stage 2 config")?)
    } else {
        Err(Error::with_context(
            ErrorKind::FileNotFound,
            &format!(
                "Stage2 config file could not be found in '{}',",
                s2_cfg_path.display()
            ),
        ))
    }
}

fn setup_logging(log_dev: Option<&LogDevice>) {
    if log_dev.is_some() {
        // Device should have been mounted by stage2-init
        match dir_exists("/mnt/log/") {
            Ok(exists) => {
                if exists {
                    Logger::set_log_file(
                        &LogDestination::StreamStderr,
                        PathBuf::from("/mnt/log/stage2.log").as_path(),
                        false,
                    )
                    .unwrap_or_else(|why| {
                        error!(
                            "Failed to setup logging to /mnt/log/stage2.log, error: {:?}",
                            why
                        )
                    });
                    info!("Set logfile to /mnt/log/stage2.log");
                }
            }
            Err(why) => {
                warn!("Failed to check for log directory, error: {:?}", why);
            }
        }
    }

    Logger::flush();
    sync();
}

fn kill_procs(log_level: Level) -> Result<()> {
    trace!("kill_procs: entered");
    let mut killed = false;
    let mut signal = SIGTERM;
    loop {
        if fuser(OLD_ROOT_MP, signal, None)? > 0 {
            killed = true
        } else {
            warn!(
                "Failed to kill processes using '{}', signal: {}",
                OLD_ROOT_MP, signal
            );
        }

        if signal == SIGKILL {
            break;
        } else {
            signal = SIGKILL;
            sleep(Duration::from_secs(5));
        }
    }

    if log_level >= Level::Debug {
        debug!("active processes:");
        for proc_info in get_process_infos()? {
            let mut name = if let Some(name) = proc_info.status().get("Name") {
                name.to_owned()
            } else {
                "-".to_owned()
            };

            let ppid = if let Some(ppid) = proc_info.status().get("PPid") {
                ppid.as_ref()
            } else {
                "-"
            };

            if proc_info.process_id() != 1 && (ppid == "0" || ppid == "2") {
                name = format!("[{}]", name);
            }

            if let Some(executable) = proc_info.executable() {
                debug!(
                    "pid: {:6} name: {}\t executable: {}\t ppid: {}",
                    proc_info.process_id(),
                    name,
                    executable.display(),
                    ppid
                );
            } else {
                debug!(
                    "pid: {:6} name: '{}'\t executable: -\t ppid: {}",
                    proc_info.process_id(),
                    name,
                    ppid
                );
            }
        }
    }

    if killed {
        Ok(())
    } else {
        Err(Error::with_context(
            ErrorKind::InvState,
            &format!("Failed to kill any processes using using '{}'", OLD_ROOT_MP,),
        ))
    }
}

fn unmount_partitions(mountpoints: &[UmountPart]) -> Result<()> {
    for mpoint in mountpoints {
        let mountpoint = path_append(OLD_ROOT_MP, &mpoint.mountpoint);

        info!(
            "Attempting to unmount '{}' from '{}'",
            mpoint.dev_name.display(),
            mountpoint.display()
        );

        match umount(&mountpoint) {
            Ok(_) => {
                info!("Successfully unmounted '{}'", mountpoint.display());
            }
            Err(why) => {
                warn!(
                    "Failed to unmount partition '{}' from '{}', error : {:?} ",
                    mpoint.dev_name.display(),
                    mountpoint.display(),
                    why
                );

                info!(
                    "Trying to remount '{}' on '{}' as readonly",
                    mpoint.dev_name.display(),
                    mountpoint.display()
                );

                mount(
                    Some(mpoint.dev_name.as_path()),
                    &mountpoint,
                    Some(mpoint.fs_type.as_bytes()),
                    MsFlags::from_bits(MS_REMOUNT | MS_RDONLY).unwrap(),
                    NIX_NONE,
                )
                .upstream_with_context(&format!(
                    "Failed to remount '{}' on '{}' with fs type: {} as readonly",
                    mpoint.dev_name.display(),
                    mountpoint.display(),
                    mpoint.fs_type,
                ))?;

                info!(
                    "Successfully remounted '{}' on '{}' as readonly ",
                    mpoint.dev_name.display(),
                    mountpoint.display()
                );
            }
        }
    }
    Ok(())
}

#[allow(dead_code)]
fn part_reread(device: &Path) -> Result<()> {
    // try ioctrl #define BLKRRPART  _IO(0x12,95)	/* re-read partition table */
    let device_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(false)
        .open(device)
        .upstream_with_context(&format!("Failed to open device '{}'", device.display(),))?;

    let ioctl_res = unsafe { ioctl(device_file.as_raw_fd(), IOCTL_BLK_RRPART) };
    if ioctl_res == 0 {
        debug!(
            "Device BLKRRPART IOCTRL to '{}' returned {}",
            device.display(),
            ioctl_res
        );
        Ok(())
    } else {
        Err(Error::with_context(
            ErrorKind::Upstream,
            &format!(
                "Device BLKRRPART IOCTRL to '{}' failed with error: {}",
                device.display(),
                io::Error::last_os_error()
            ),
        ))
    }
}

fn transfer_boot_files<P: AsRef<Path>>(dev_root: P) -> Result<()> {
    let src_path = path_append(TRANSFER_DIR, BALENA_CONFIG_PATH);
    let target_path = path_append(dev_root.as_ref(), BALENA_CONFIG_PATH);
    copy(&src_path, &target_path).upstream_with_context(&format!(
        "Failed to copy {} to {}",
        src_path.display(),
        target_path.display()
    ))?;
    debug!("Source config json is '{}', target config.json is '{}'", src_path.display(),  target_path.display());
    info!("Successfully copied config.json to boot partition",);

    let boot_directories = vec![SYSTEM_CONNECTIONS_DIR, SYSTEM_PROXY_DIR];

    for boot_directory in boot_directories.into_iter() {
        let src_path = path_append(TRANSFER_DIR, boot_directory);
        let dir_list = read_dir(&src_path).upstream_with_context(&format!(
            "Failed to read directory '{}'",
            src_path.display()
        ))?;

        let target_dir = path_append(dev_root.as_ref(), boot_directory);
        debug!(
            "Transfering files from '{}' to '{}'",
            src_path.display(),
            target_dir.display()
        );

        for entry in dir_list {
            match entry {
                Ok(entry) => {
                    let curr_file = entry.path();
                    debug!("Found source file '{}'", curr_file.display());
                    if entry
                        .metadata()
                        .upstream_with_context(&format!(
                            "Failed to read metadata from file '{}'",
                            curr_file.display()
                        ))?
                        .is_file()
                    {
                        if let Some(filename) = curr_file.file_name() {
                            let target_path = path_append(&target_dir, filename);
                            copy(&curr_file, &target_path).upstream_with_context(&format!(
                                "Failed to copy '{}' to '{}'",
                                curr_file.display(),
                                target_path.display()
                            ))?;
                            info!(
                                "Successfully copied '{}' to boot partition as '{}",
                                curr_file.display(),
                                target_path.display()
                            );
                        } else {
                            warn!(
                                "Failed to extract filename from path '{}'",
                                curr_file.display()
                            );
                        }
                    }
                }
                Err(why) => {
                    return Err(Error::with_all(
                        ErrorKind::Upstream,
                        "Failed to read directory entry",
                        Box::new(why),
                    ));
                }
            }
        }
    }

    Ok(())
}

fn get_partition_infos(device: &Path) -> Result<(PartInfo, PartInfo, PartInfo)> {
    let mut disk = Disk::from_drive_file(device, None)?;

    let mut boot_part: Option<PartInfo> = None;
    let mut root_a_part: Option<PartInfo> = None;
    let mut data_part: Option<PartInfo> = None;

    // GPT provides a 'protective MBR' at LBA 0 to identify itself in a backward
    // compatible way. So, read the GPT header if so.
    let is_gpt = disk.get_label()? == LabelType::GPT;

    if !is_gpt {
        let part_iterator = PartitionIterator::new(&mut disk)?;
        for partition in part_iterator {
            debug!(
                "partition: {}, start: {}, sectors: {}",
                partition.index, partition.start_lba, partition.num_sectors
            );

            match partition.index {
                1 => {
                    boot_part = Some(partition);
                }
                2 => {
                    root_a_part = Some(partition);
                }
                3..=5 => debug!("Skipping partition {}", partition.index),
                6 => {
                    data_part = Some(partition);
                    break;
                }
                _ => {
                    return Err(Error::with_context(
                        ErrorKind::InvParam,
                        &format!("Invalid partition index encountered: {}", partition.index),
                    ));
                }
            }
        }
    } else {
        // Use the iterator built into gptman and populate the PartInfo structs
        // for the boot and data partitions as best we can.
        let gpt = match disk.read_gpt() {
            Ok(gpt_res) => gpt_res,
            Err(e) => {
                return Err(Error::with_context(
                    ErrorKind::InvState,
                    &format!("Failed to read GPT header, error: {} ", e)));
            }
        };
        for (i, p) in gpt.iter() {
            if p.is_used() {
                debug!("Partition #{}: type = {:?}, size = {} bytes, starting lba = {}, name = {}",
                    i,
                    p.partition_type_guid,
                    p.size().unwrap() * gpt.sector_size,
                    p.starting_lba,
                    p.partition_name.as_str());

                if  p.partition_name.as_str() == "resin-boot" {
                        boot_part = Some(PartInfo {
                            index: i as usize,
                            ptype: 0x83,  // MBR Linux byte; really this is the EFI system
                                          // partition, but MBR doesn't define this type.
                            status: 0,    // Not clear what this should be
                            start_lba: p.starting_lba,
                            num_sectors: p.size().unwrap()
                        })
                } else if  p.partition_name.as_str() == "resin-rootA" {
                            root_a_part = Some(PartInfo {
                                index: i as usize,
                                ptype: 0x83,  // MBR Linux byte; really this is the EFI system
                                              // partition, but MBR doesn't define this type.
                                status: 0,    // Not clear what this should be
                                start_lba: p.starting_lba,
                                num_sectors: p.size().unwrap()
                            })
                } else if p.partition_name.as_str() == "resin-data" {
                        data_part = Some(PartInfo {
                            index: i as usize,
                            ptype: 0x83,  // MBR Linux byte
                            status: 0,    // not clear what this should be
                            start_lba: p.starting_lba,
                            num_sectors: p.size().unwrap()
                        });
                }
            }
        }
    }

    if let Some(boot_part) = boot_part {
        if let Some(data_part) = data_part {
            if let Some(root_a_part) = root_a_part {
                Ok((boot_part, root_a_part, data_part))
            } else {
                Err(Error::with_context(
                    ErrorKind::NotFound,
                    &format!("RootA partition could not be found on '{}", device.display()),
                ))
            }
        } else {
            Err(Error::with_context(
                ErrorKind::NotFound,
                &format!("Data partition could not be found on '{}", device.display()),
            ))
        }
    } else {
        Err(Error::with_context(
            ErrorKind::NotFound,
            &format!("Boot partition could not be found on '{}", device.display()),
        ))
    }
}

fn efi_setup(device: &Path) -> Result<()> {
    let efi_boot_mgr = format!("/bin/{}", EFIBOOTMGR_CMD);
    if dir_exists(SYS_EFI_DIR)? {
        match call_command!(&efi_boot_mgr, &[], "Failed to execute efibootmgr") {
            Ok(cmd_stdout) => {
                // TODO: setup efi boot
                let efivar_regex =
                    Regex::new(r#"\s*Boot([0-9,a-f,A-F]{4})\*?\s+resinOS.*"#).unwrap();
                for line in cmd_stdout.lines() {
                    if let Some(captures) = efivar_regex.captures(line) {
                        let boot_num = captures.get(1).unwrap().as_str();
                        match call_command!(&efi_boot_mgr, &["-B", "-b", boot_num]) {
                            Ok(_) => (),
                            Err(why) => {
                                error!(
                                    "Failed to delete boot manager '{}' as {}, error: {}",
                                    line, boot_num, why
                                );
                            }
                        }
                    }
                }
                match call_command!(
                    &efi_boot_mgr,
                    &[
                        "-c",
                        "-d",
                        &*device.to_string_lossy(),
                        "-p",
                        "1",
                        "-L",
                        "resinOS",
                        "-l",
                        r"\EFI\BOOT\bootx64.efi"
                    ]
                ) {
                    Ok(_) => (),
                    Err(why) => error!("Failed to setup EFI boot, error {}", why),
                }
            }
            Err(why) => {
                error!("Failed to execute '{}', error: {}", efi_boot_mgr, why);
            }
        }
    } else {
        let efi_dir = path_append(BALENA_BOOT_MP, "EFI");
        if dir_exists(&efi_dir)? {
            match remove_dir(&efi_dir) {
                Ok(_) => {
                    debug!("Removed EFI directory from '{}'", BALENA_BOOT_MP);
                }
                Err(why) => {
                    warn!(
                        "Failed to remove EFI directory from '{}', error: {}",
                        BALENA_BOOT_MP, why
                    );
                }
            }
        }
    }

    Ok(())
}

fn write_boot_blob(s2_config: &Stage2Config, mount_path: PathBuf)
{
    let file_finder = Finder::new(mount_path.clone());
    let mut img_path = PathBuf::new();


    if s2_config.device_type.starts_with("Jetson Xavier AGX") {
        for i in file_finder.into_iter() {
            if i.path().to_string_lossy().contains(BOOT_BLOB_NAME_JETSON_XAVIER) {
                img_path = i.path().to_path_buf();
                break;
            }
        }

        debug!("boot0_image_path is: '{}'", img_path.display());
        debug!("target device is: '{}'", BOOT_BLOB_PARTITION_JETSON_XAVIER);

        /* Enable writing to /dev/mmcblk0boot0/ */
        let force_ro = "0";
        std::fs::write(JETSON_XAVIER_HW_PART_FORCE_RO_FILE, force_ro).expect("Could not set hw boot partition rw!");

        let boot0_data = std::fs::read(img_path).unwrap();
        debug!("boot blob - bytes read from disk: '{}' ", boot0_data.len());

        std::fs::write(BOOT_BLOB_PARTITION_JETSON_XAVIER, boot0_data).expect("Could not write hw boot partition!");
        debug!("Jetson Xavier AGX boot blob was written");
    } else if s2_config.device_type.starts_with("Jetson Xavier NX") {
        for i in file_finder.into_iter() {
            if i.path().to_string_lossy().contains(BOOT_BLOB_NAME_JETSON_XAVIER_NX) {
                img_path = i.path().to_path_buf();
                break;
            }
        }

        debug!("boot0_image_path is: '{}'", img_path.display());
        debug!("boot0_image_dev is: '{}'", BOOT_BLOB_PARTITION_JETSON_XAVIER_NX);

        match flash_qspi(&img_path) {
            FlashState::Success => {info!("Xavier NX QSPI written succesfully!")},
            _ => {warn!("Failed to write QSPI!")}
        }
    }
}

fn raw_mount_balena(s2_cfg: &Stage2Config) -> Result<()> {
    let device = &s2_cfg.flash_dev;
    debug!("raw_mount_balena called");

    if !dir_exists(BALENA_PART_MP)? {
        create_dir(BALENA_PART_MP).upstream_with_context(&format!(
            "Failed to create balena partition mountpoint: '{}'",
            BALENA_PART_MP
        ))?;
    }

    let (boot_part, root_a_part, data_part) = get_partition_infos(device)?;

    let mut loop_device = LoopDevice::get_free(true)?;
    info!("Create loop device: '{}'", loop_device.get_path().display());
    let byte_offset = boot_part.start_lba * DEF_BLOCK_SIZE as u64;
    let size_limit = boot_part.num_sectors * DEF_BLOCK_SIZE as u64;

    debug!(
        "Setting up device '{}' with offset {}, sizelimit {} on '{}'",
        device.display(),
        byte_offset,
        size_limit,
        loop_device.get_path().display()
    );

    loop_device.setup(device, Some(byte_offset), Some(size_limit))?;
    info!(
        "Setup device '{}' with offset {}, sizelimit {} on '{}'",
        device.display(),
        byte_offset,
        size_limit,
        loop_device.get_path().display()
    );

    mount(
        Some(loop_device.get_path()),
        BALENA_PART_MP,
        Some(BALENA_BOOT_FSTYPE.as_bytes()),
        MsFlags::empty(),
        NIX_NONE,
    )
    .upstream_with_context(&format!(
        "Failed to mount {} on {}",
        loop_device.get_path().display(),
        BALENA_PART_MP
    ))?;

    info!(
        "Mounted boot partition as {} on {}",
        loop_device.get_path().display(),
        BALENA_PART_MP
    );

    // TODO: copy files

    transfer_boot_files(BALENA_PART_MP)?;

    efi_setup(device)?;

    sync();

    umount(BALENA_PART_MP).upstream_with_context("Failed to unmount boot partition")?;

    info!("Unmounted boot partition from {}", BALENA_PART_MP);

    let mut loop_device = LoopDevice::get_free(true)?;
    info!("Create loop device: '{}'", loop_device.get_path().display());
    let byte_offset = root_a_part.start_lba * DEF_BLOCK_SIZE as u64;
    let size_limit = root_a_part.num_sectors * DEF_BLOCK_SIZE as u64;

    debug!(
        "Setting up device '{}' with offset {}, sizelimit {} on '{}'",
        device.display(),
        byte_offset,
        size_limit,
        loop_device.get_path().display()
    );

    loop_device.setup(device, Some(byte_offset), Some(size_limit))?;
    info!(
        "Setup device '{}' with offset {}, sizelimit {} on '{}'",
        device.display(),
        byte_offset,
        size_limit,
        loop_device.get_path().display()
    );

    mount(
        Some(loop_device.get_path()),
        BALENA_PART_MP,
        Some(BALENA_ROOTA_FSTYPE.as_bytes()),
        MsFlags::empty(),
        NIX_NONE,
    )
    .upstream_with_context(&format!(
        "Failed to mount {} on {}",
        loop_device.get_path().display(),
        BALENA_PART_MP
    ))?;

    info!(
        "Mounted resin-rootA partition as {} on {}",
        loop_device.get_path().display(),
        BALENA_PART_MP
    );

    write_boot_blob(s2_cfg, PathBuf::from(BALENA_PART_MP));

    sync();

    umount(BALENA_PART_MP).upstream_with_context("Failed to unmount boot partition")?;

    info!("Unmounted resin-rootA partition from {}", BALENA_PART_MP);

    let backup_path = path_append(TRANSFER_DIR, BACKUP_ARCH_NAME);

    if file_exists(&backup_path) {
        let byte_offset = data_part.start_lba * DEF_BLOCK_SIZE as u64;
        let size_limit = data_part.num_sectors * DEF_BLOCK_SIZE as u64;

        loop_device.modify_offset(byte_offset, size_limit)?;

        info!(
            "Setup device '{}' with offset {}, sizelimit {} on '{}'",
            device.display(),
            byte_offset,
            size_limit,
            loop_device.get_path().display()
        );

        mount(
            Some(loop_device.get_path()),
            BALENA_PART_MP,
            Some(BALENA_DATA_FSTYPE.as_bytes()),
            MsFlags::empty(),
            NIX_NONE,
        )
        .upstream_with_context(&format!(
            "Failed to mount {} on {}",
            loop_device.get_path().display(),
            BALENA_PART_MP
        ))?;

        info!(
            "Mounted data partition as {} on {}",
            loop_device.get_path().display(),
            BALENA_PART_MP
        );

        // TODO: copy files

        let target_path = path_append(BALENA_PART_MP, BACKUP_ARCH_NAME);
        copy(&backup_path, &target_path).upstream_with_context(&format!(
            "Failed to copy '{}' to '{}'",
            backup_path.display(),
            target_path.display()
        ))?;

        info!(
            "copied '{}' to '{}'",
            backup_path.display(),
            target_path.display()
        );

        sync();

        umount(BALENA_PART_MP).upstream_with_context("Failed to unmount boot partition")?;

        info!("Unmounted data partition from {}", BALENA_PART_MP);
    }

    loop_device.unset()?;

    Ok(())
}

#[allow(dead_code)]
fn sys_mount_balena() -> Result<()> {
    debug!("sys_mount_balena called");
    sleep(Duration::from_secs(1));

    let part_label = path_append(DISK_BY_LABEL_PATH, BALENA_BOOT_PART);
    if !file_exists(&part_label) {
        return Err(Error::with_context(
            ErrorKind::NotFound,
            &format!(
                "Failed to locate path to boot partition in '{}'",
                part_label.display()
            ),
        ));
    }
    create_dir(BALENA_BOOT_MP).upstream_with_context(&format!(
        "Failed to create balena-boot mountpoint: '{}'",
        BALENA_BOOT_MP
    ))?;

    mount(
        Some(&part_label),
        BALENA_BOOT_MP,
        Some(BALENA_BOOT_FSTYPE.as_bytes()),
        MsFlags::empty(),
        NIX_NONE,
    )
    .upstream_with_context(&format!(
        "Failed to mount '{}' to '{}'",
        part_label.display(),
        BALENA_BOOT_MP,
    ))?;

    transfer_boot_files(BALENA_BOOT_MP)?;

    umount(BALENA_BOOT_MP).upstream_with_context(&format!(
        "Failed to unmount '{}' from '{}'",
        part_label.display(),
        BALENA_BOOT_MP
    ))?;

    let backup_path = path_append(TRANSFER_DIR, BACKUP_ARCH_NAME);
    if file_exists(&backup_path) {
        let part_label = path_append(DISK_BY_LABEL_PATH, BALENA_DATA_PART);
        mount(
            Some(&part_label),
            BALENA_BOOT_MP,
            Some(BALENA_DATA_FSTYPE.as_bytes()),
            MsFlags::empty(),
            NIX_NONE,
        )
        .upstream_with_context(&format!(
            "Failed to mount '{}' to '{}'",
            part_label.display(),
            BALENA_BOOT_MP,
        ))?;

        let target_path = path_append(BALENA_PART_MP, BACKUP_ARCH_NAME);
        copy(&backup_path, &target_path).upstream_with_context(&format!(
            "Failed to copy '{}' to '{}'",
            backup_path.display(),
            target_path.display()
        ))?;

        sync();

        umount(BALENA_PART_MP).upstream_with_context(&format!(
            "Failed to unmount '{}' from '{}'",
            part_label.display(),
            BALENA_PART_MP
        ))?;
    }

    Ok(())
}

enum FlashState {
    Success,
    FailRecoverable,
    FailNonRecoverable,
}

fn fill_buffer<I: Read>(buffer: &mut [u8], input: &mut I) -> Result<usize> {
    // fill buffer
    let mut buff_fill: usize = 0;
    loop {
        let bytes_read = input
            .read(&mut buffer[buff_fill..])
            .upstream_with_context("Failed to read from  input stream")?;

        if bytes_read > 0 {
            buff_fill += bytes_read;
            if buff_fill < buffer.len() {
                continue;
            }
        }
        break;
    }

    Ok(buff_fill)
}

fn validate(target_path: &Path, image_path: &Path) -> Result<bool> {
    debug!("Validate: opening: '{}'", image_path.display());

    let mut decoder = GzDecoder::new(File::open(image_path).upstream_with_context(&format!(
        "Validate: Failed to open image file '{}'",
        image_path.display(),
    ))?);

    debug!("Validate: opening output file '{}'", target_path.display());
    let mut target = OpenOptions::new()
        .write(false)
        .read(true)
        .create(false)
        .open(target_path)
        .upstream_with_context(&format!(
            "Validate: Failed to open output file '{}'",
            target_path.display(),
        ))?;

    let mut gz_buffer: [u8; VALIDATE_BLOCK_SIZE] = [0; VALIDATE_BLOCK_SIZE];
    let mut tgt_buffer: [u8; VALIDATE_BLOCK_SIZE] = [0; VALIDATE_BLOCK_SIZE];

    let mut byte_offset: u64 = 0;
    let mut err_count = 0;

    loop {
        let gz_read = fill_buffer(&mut gz_buffer, &mut decoder)?;
        let tgt_read = fill_buffer(&mut tgt_buffer, &mut target)?;
        if gz_read == 0 {
            break;
        }

        if gz_read > tgt_read {
            warn!(
                "Validate: file size mismatch at offset {:x}:{}: gzip stream {} output stream {}",
                byte_offset,
                format_size_with_unit(byte_offset),
                gz_read,
                tgt_read
            );
            return Ok(false);
        } else {
            for idx in 0..gz_read {
                if gz_buffer[idx] != tgt_buffer[idx] {
                    warn!(
                        "Validate: byte mismatch at offset 0x{:x}:{}: {:x} != {:x}",
                        byte_offset + idx as u64,
                        format_size_with_unit(byte_offset + idx as u64),
                        gz_buffer[idx],
                        tgt_buffer[idx]
                    );
                    err_count += 1;
                    if err_count >= VALIDATE_MAX_ERR {
                        return Ok(false);
                    }
                }
            }
            byte_offset += gz_read as u64;
        }

        if gz_read < VALIDATE_BLOCK_SIZE {
            break;
        }
    }

    Ok(err_count == 0)
}

fn flash_qspi(image_path: &Path/* boot blob path */) -> FlashState {
    let mut flash_qspi_res = FlashState::Success;
    info!("entered flash_qspi");

    match call_command!(&format!("/bin/{}", MTD_DEBUG_CMD), &["erase", &format!("{}", BOOT_BLOB_PARTITION_JETSON_XAVIER_NX), "0", &format!("{}", JETSON_XAVIER_NX_QSPI_SIZE)], "Failed to execute mtdebug!") {
        Ok(cmd_stdout) => {
            for line in cmd_stdout.lines() {
                    info!("line: {}", line);
                    }
                },
        _ => { warn!("Error executing mtd_debug erase!")}
    }


    match call_command!(&format!("/bin/{}", MTD_DEBUG_CMD), &[
        "write",
        &format!("{}", BOOT_BLOB_PARTITION_JETSON_XAVIER_NX),
        "0",
        &format!("{}", JETSON_XAVIER_NX_QSPI_SIZE),
        &image_path.to_string_lossy()
    ], "Failed to execute mtdebug!") {
        Ok(cmd_stdout) => {
            for line in cmd_stdout.lines() {
                    info!("line: {}", line);
                    }
                },
        _ => { warn!("Error executing mtd_debug write!");
                flash_qspi_res = FlashState::FailRecoverable; // TODO: try flash back old boot0 image as fallback
        }
    }

    info!("Executed mtd_debug");

    info!("leaving flash_qspi()");
    return flash_qspi_res;
}

fn flash_external(target_path: &Path, image_path: &Path, dd_cmd: &str) -> FlashState {
    let mut fail_res = FlashState::FailRecoverable;

    let mut decoder = GzDecoder::new(match File::open(image_path) {
        Ok(file) => file,
        Err(why) => {
            error!(
                "Flash: Failed to open image file '{}', error: {:?}",
                image_path.display(),
                why
            );
            return fail_res;
        }
    });

    debug!("invoking dd");
    match Command::new(dd_cmd)
        .args([
            &format!("of={}", &target_path.to_string_lossy()),
            &format!("bs={}", DD_BLOCK_SIZE),
        ])
        .stdin(Stdio::piped())
        .spawn()
    {
        Ok(mut dd_cmd) => {
            if let Some(stdin) = dd_cmd.stdin.as_mut() {
                let mut buffer: [u8; DD_BLOCK_SIZE] = [0; DD_BLOCK_SIZE];
                let mut tot_bytes: u64 = 0;
                let start_time = Instant::now();
                fail_res = FlashState::FailNonRecoverable;

                loop {
                    // fill buffer
                    match fill_buffer(&mut buffer, &mut decoder) {
                        Ok(buff_fill) => {
                            if buff_fill > 0 {
                                match stdin.write_all(&buffer) {
                                    Ok(_) => {
                                        tot_bytes += buff_fill as u64;
                                        if buff_fill < DD_BLOCK_SIZE {
                                            break;
                                        }
                                    }
                                    Err(why) => {
                                        error!("Failed to write to dd stdin at offset 0x{:x}:{} error {:?}",
                                               tot_bytes,
                                               format_size_with_unit(tot_bytes),
                                               why);
                                        return fail_res;
                                    }
                                }
                            } else {
                                break;
                            }
                        }
                        Err(why) => {
                            error!(
                                "Failed to read compressed data from '{}' at offset 0x{:x}:{}, error: {}:?",
                                image_path.display(),
                                tot_bytes,
                                format_size_with_unit(tot_bytes),
                                why
                            );
                            return fail_res;
                        }
                    };
                }

                let elapsed = Instant::now().duration_since(start_time).as_secs();
                info!(
                    "Wrote {} bytes, {} to dd in {} seconds @ {}/sec",
                    tot_bytes,
                    format_size_with_unit(tot_bytes),
                    elapsed,
                    format_size_with_unit(tot_bytes / elapsed),
                );
            } else {
                error!("Failed to retrieve dd stdin");
                return FlashState::FailRecoverable;
            }

            match dd_cmd.wait() {
                Ok(status) => {
                    if status.success() {
                        info!("dd terminated successfully");
                        FlashState::Success
                    } else {
                        error!("dd terminated with exit code: {:?}", status.code());
                        FlashState::FailNonRecoverable
                    }
                }
                Err(why) => {
                    error!(
                        "Failure waiting for dd command termination, error: {:?}",
                        why
                    );
                    fail_res
                }
            }
        }
        Err(why) => {
            error!("Failed to execute '{}', error: {:?}", DD_CMD, why);
            fail_res
        }
    }
}

#[allow(clippy::cognitive_complexity)]
pub fn stage2(opts: &Options) -> ! {
    Logger::set_default_level(opts.s2_log_level());
    Logger::set_brief_info(false);
    Logger::set_color(true);

    if let Err(why) = Logger::set_log_dest(&LogDestination::BufferStderr, NO_STREAM) {
        error!("Failed to initialize logging, error: {:?}", why);
        reboot();
    }

    info!("Stage 2 migrate_worker entered");

    const NO_PREFIX: Option<&Path> = None;
    let s2_config: Stage2Config = match read_stage2_config(NO_PREFIX) {
        Ok(s2_config) => s2_config,
        Err(why) => {
            error!("Failed to read stage2 configuration, error: {:?}", why);
            reboot();
        }
    };

    info!("Stage 2 config was read successfully");

    setup_logging(s2_config.log_dev());

    match kill_procs(opts.s2_log_level()) {
        Ok(_) => (),
        Err(why) => {
            error!("kill_procs failed, error {}", why);
            reboot();
        }
    };

    match copy_files(&s2_config) {
        Ok(_) => (),
        Err(why) => {
            error!("Failed to copy files to RAMFS, error: {:?}", why);
            reboot();
        }
    }

    match unmount_partitions(&s2_config.umount_parts) {
        Ok(_) => (),
        Err(why) => {
            error!("unmount_partitions failed; {:?}", why);
            reboot();
        }
    }

    if s2_config.pretend {
        info!("Not flashing due to pretend mode");
        reboot();
    }

    sync();


    let image_path = path_append(TRANSFER_DIR, BALENA_IMAGE_PATH);
    debug!("OS image exists - {}", image_path.exists());

    match flash_external(
        &s2_config.flash_dev,
        &image_path,
        &format!("/bin/{}", DD_CMD),
    ) {
        FlashState::Success => (),
        _ => {
            sleep(Duration::from_secs(10));
            reboot();
        }
    }

    sync();
    sleep(Duration::from_secs(5));

    if DO_VALIDATE {
        match validate(&s2_config.flash_dev, &image_path) {
            Ok(res) => {
                if res {
                    info!("Image validated successfully");
                } else {
                    error!("Image validation failed");
                }
            }
            Err(why) => {
                error!("Image validation returned error: {:?}", why);
            }
        }
    }

    sleep(Duration::from_secs(5));


    if (opts.s2_log_level() == Level::Debug) || (opts.s2_log_level() == Level::Trace) {
        use crate::common::debug::check_loop_control;
        check_loop_control("Stage2 after flash", "/dev");
    }

    if let Err(why) = raw_mount_balena(&s2_config) {
        error!("Failed to transfer files to balena OS, error: {:?}", why);
    } else {
        info!("Migration succeded successfully");
    }

    sync();

    reboot();
}
