use std::fs::{copy, create_dir, create_dir_all, read_dir, read_to_string, File, OpenOptions};
use std::io::{self, Read, Write};

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
use libc::{ioctl, MS_RDONLY, MS_REMOUNT};
use log::{debug, error, info, trace, warn, Level};
use mod_logger::{LogDestination, Logger, NO_STREAM};

use crate::common::defs::IoctlReq;
use crate::common::{
    call,
    defs::{
        BACKUP_ARCH_NAME, BALENA_BOOT_FSTYPE, BALENA_BOOT_MP, BALENA_BOOT_PART, BALENA_CONFIG_PATH,
        BALENA_DATA_FSTYPE, BALENA_DATA_PART, BALENA_IMAGE_NAME, BALENA_IMAGE_PATH, BALENA_PART_MP,
        DD_CMD, DISK_BY_LABEL_PATH, FUSER_CMD, NIX_NONE, OLD_ROOT_MP, PS_CMD, REBOOT_CMD,
        STAGE2_CONFIG_NAME, SYSTEM_CONNECTIONS_DIR,
    },
    dir_exists,
    disk_util::{Disk, PartInfo, PartitionIterator, DEF_BLOCK_SIZE},
    error::{Error, ErrorKind, Result, ToError},
    file_exists, format_size_with_unit, get_mem_info,
    loop_device::LoopDevice,
    options::Options,
    path_append,
    stage2_config::{Stage2Config, UmountPart},
};

const DD_BLOCK_SIZE: usize = 128 * 1024; // 4_194_304;

const VALIDATE_MAX_ERR: usize = 20;
const DO_VALIDATE: bool = true;
const VALIDATE_BLOCK_SIZE: usize = 64 * 1024; // 4_194_304;

const IOCTL_BLK_RRPART: IoctlReq = 0x1295;

const TRANSFER_DIR: &str = "/transfer";

const S2_XTRA_FS_SIZE: u64 = 10 * 1024 * 1024;

pub(crate) const BUSYBOX_CMD: &str = "/busybox";

pub(crate) fn busybox_reboot() {
    trace!("reboot entered");
    Logger::flush();
    sync();
    sleep(Duration::from_secs(3));
    info!("rebooting");
    let _cmd_res = call_busybox!(&[REBOOT_CMD, "-f"]);
    sleep(Duration::from_secs(1));
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
        .len() as u64;

    let curr_file = path_append(OLD_ROOT_MP, &s2_cfg.config_path);
    req_size += curr_file
        .metadata()
        .upstream_with_context(&format!(
            "Failed to retrieve file size for '{}'",
            curr_file.display()
        ))?
        .len() as u64;

    if let Some(ref backup_path) = s2_cfg.backup_path {
        let curr_file = path_append(OLD_ROOT_MP, backup_path);
        req_size += curr_file
            .metadata()
            .upstream_with_context(&format!(
                "Failed to retrieve file size for '{}'",
                curr_file.display()
            ))?
            .len() as u64;
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
                    From::from(why),
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
    info!("Copied image to '{}'", to_path.display());

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

    let nwmgr_path = path_append(
        OLD_ROOT_MP,
        path_append(&s2_cfg.work_dir, SYSTEM_CONNECTIONS_DIR),
    );

    let to_dir = path_append(TRANSFER_DIR, SYSTEM_CONNECTIONS_DIR);
    if !dir_exists(&to_dir)? {
        create_dir_all(&to_dir).upstream_with_context(&format!(
            "Failed to create directory: '{}'",
            to_dir.display()
        ))?;
    }

    for dir_entry in read_dir(&nwmgr_path).upstream_with_context(&format!(
        "Failed to read drectory '{}'",
        nwmgr_path.display()
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
                    error!(
                        "Failed to extract filename from path: '{}'",
                        dir_entry.path().display()
                    );
                    return Err(Error::displayed());
                }
            }
            Err(why) => {
                return Err(Error::from_upstream(
                    From::from(why),
                    &format!(
                        "Failed to retrieve directory entry for '{}'",
                        nwmgr_path.display()
                    ),
                ));
            }
        }
    }

    Ok(())
}

pub(crate) fn read_stage2_config() -> Result<Stage2Config> {
    let s2_cfg_path = PathBuf::from(&format!("/{}", STAGE2_CONFIG_NAME));
    if file_exists(&s2_cfg_path) {
        let s2_cfg_txt = match read_to_string(&s2_cfg_path) {
            Ok(s2_config_txt) => s2_config_txt,
            Err(why) => {
                error!(
                    "Failed to read stage 2 config from '{}', error: {}",
                    s2_cfg_path.display(),
                    why
                );
                return Err(Error::displayed());
            }
        };
        match Stage2Config::deserialze(&s2_cfg_txt) {
            Ok(s2_config) => Ok(s2_config),
            Err(why) => {
                error!("Failed to deserialize stage 2 config: error {}", why);
                Err(Error::displayed())
            }
        }
    } else {
        error!(
            "Stage2 config file could not be found in '{}',",
            s2_cfg_path.display()
        );
        Err(Error::displayed())
    }
}

fn setup_logging<P: AsRef<Path>>(log_dev: &Option<P>) {
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
    let mut signal: &str = "TERM";
    loop {
        let cmd_res = call(
            BUSYBOX_CMD,
            &[FUSER_CMD, "-k", &format!("-{}", signal), "-m", OLD_ROOT_MP],
            true,
        )?;

        if cmd_res.status.success() {
            killed = true;
        } else {
            warn!(
                "Failed to kill processes using '{}', signal: {}, stderr: {}",
                OLD_ROOT_MP, signal, cmd_res.stderr
            );
        }

        if signal == "KILL" {
            break;
        } else {
            signal = "KILL";
            sleep(Duration::from_secs(5));
        }
    }

    if let Level::Trace = log_level {
        if let Ok(res) = call_busybox!(&[PS_CMD, "-A"], "") {
            trace!("ps: {}", res);
        }
    }

    if !killed {
        Ok(())
    } else {
        error!("Failed to kill any processes using using '{}'", OLD_ROOT_MP,);
        Err(Error::displayed())
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

                match mount(
                    Some(mpoint.dev_name.as_path()),
                    &mountpoint,
                    Some(mpoint.fs_type.as_bytes()),
                    MsFlags::from_bits(MS_REMOUNT | MS_RDONLY).unwrap(),
                    NIX_NONE,
                ) {
                    Ok(_) => {
                        info!(
                            "Successfully remounted '{}' on '{}' as readonly ",
                            mpoint.dev_name.display(),
                            mountpoint.display()
                        );
                    }
                    Err(why) => {
                        error!(
                            "Failed to remount '{}' on '{}' with fs type: {} as readonly, error: {:?}",
                            mpoint.dev_name.display(),
                            mountpoint.display(),
                            mpoint.fs_type,
                            why
                        );
                        return Err(Error::displayed());
                    }
                }
            }
        }
    }
    Ok(())
}

#[allow(dead_code)]
fn part_reread(device: &Path) -> Result<()> {
    // try ioctrl #define BLKRRPART  _IO(0x12,95)	/* re-read partition table */
    match OpenOptions::new()
        .read(true)
        .write(true)
        .create(false)
        .open(device)
    {
        Ok(file) => {
            let ioctl_res = unsafe { ioctl(file.as_raw_fd(), IOCTL_BLK_RRPART) };
            if ioctl_res == 0 {
                debug!(
                    "Device BLKRRPART IOCTRL to '{}' returned {}",
                    device.display(),
                    ioctl_res
                );
                Ok(())
            } else {
                error!(
                    "Device BLKRRPART IOCTRL to '{}' failed with error: {}",
                    device.display(),
                    io::Error::last_os_error()
                );
                Err(Error::displayed())
            }
        }
        Err(why) => {
            error!(
                "Failed to open device '{}', error: {:?}",
                device.display(),
                why
            );
            Err(Error::displayed())
        }
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

    info!("Successfully copied config.json to boot partition",);

    let src_path = path_append(TRANSFER_DIR, SYSTEM_CONNECTIONS_DIR);
    let dir_list = read_dir(&src_path).upstream_with_context(&format!(
        "Failed to read directory '{}'",
        src_path.display()
    ))?;

    let target_dir = path_append(dev_root.as_ref(), SYSTEM_CONNECTIONS_DIR);
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
                error!("Failed to read directory entry, error: {:?}", why);
                return Err(Error::displayed());
            }
        }
    }

    Ok(())
}
/*
fn raw_mount_partition<P1: AsRef<Path>, P2: AsRef<Path>>(
    loop_device: &LoopDevice,
    partition: &PartInfo,
    device: P2,
    mountpoint: P1,
    fs_type: &str,
) -> Result<String> {
    let byte_offset = partition.start_lba * DEF_BLOCK_SIZE as u64;
    let size_limit = partition.num_sectors * DEF_BLOCK_SIZE as u64;

    debug!(
        "file '{}' exists: {}",
        device.as_ref().display(),
        file_exists(&device)
    );

    let mut loop_device =
        match LoopDevice::for_file(device, Some(byte_offset), Some(size_limit), None) {
            Ok(loop_device) => loop_device,
            Err(why) => {
                error!(
                    "Failed to loop mount device '{}' with offset {}, sizelimit {}, error {:?}",
                    device.as_ref().display(),
                    byte_offset,
                    size_limit,
                    why
                );
                return Err(Error::displayed());
            }
        };

    info!(
        "Loop-mounted {} on '{}'",
        loop_device.get_path().display(),
        mountpoint.as_ref().display()
    );
    mount(
        Some(loop_device.get_path()),
        mountpoint.as_ref(),
        Some(fs_type.as_bytes()),
        MsFlags::empty(),
        NIX_NONE,
    )
    .upstream_with_context(&format!(
        "Failed to mount {} on {}",
        loop_dev,
        mountpoint.as_ref().display()
    )))?;
    Ok(loop_dev)
}

fn raw_umount_partition<P: AsRef<Path>>(device: &str, mountpoint: P) -> Result<()> {
    umount(mountpoint.as_ref()).upstream_with_context(&format!(
        "Failed to unmount {}",
        mountpoint.as_ref().display()
    )))?;

    call_command!(
        BUSYBOX_CMD,
        &[LOSETUP_CMD, "-d", device],
        &format!("Failed to remove loop device {}", device,)
    )?;

    Ok(())
}

 */

fn raw_mount_balena(device: &Path) -> Result<()> {
    debug!("raw_mount_balena called");

    let backup_path = path_append(TRANSFER_DIR, BACKUP_ARCH_NAME);

    if !dir_exists(BALENA_PART_MP)? {
        create_dir(BALENA_PART_MP).upstream_with_context(&format!(
            "Failed to create balena partition mountpoint: '{}'",
            BALENA_PART_MP
        ))?;
    }

    let (boot_part, data_part) = {
        let mut disk = Disk::from_drive_file(device, None)?;
        let part_iterator = PartitionIterator::new(&mut disk)?;
        let mut boot_part: Option<PartInfo> = None;
        let mut data_part: Option<PartInfo> = None;

        for partition in part_iterator {
            debug!(
                "partition: {}, start: {}, sectors: {}",
                partition.index, partition.start_lba, partition.num_sectors
            );

            match partition.index {
                1 => {
                    boot_part = Some(partition);
                }
                2..=4 => debug!("Skipping partition {}", partition.index),
                5 => {
                    data_part = Some(partition);
                    break;
                }
                _ => {
                    error!("Invalid partition index encountered: {}", partition.index);
                    return Err(Error::displayed());
                }
            }
        }

        if let Some(boot_part) = boot_part {
            if let Some(data_part) = data_part {
                (boot_part, data_part)
            } else {
                error!("Data partition could not be found on '{}", device.display());
                return Err(Error::displayed());
            }
        } else {
            error!("Boot partition could not be found on '{}", device.display());
            return Err(Error::displayed());
        }
    };

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

    loop_device.setup(&device, Some(byte_offset), Some(size_limit))?;
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

    sync();

    umount(BALENA_PART_MP).upstream_with_context("Failed to unmount boot partition")?;

    info!("Unmounted boot partition from {}", BALENA_PART_MP);

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
        error!(
            "Failed to locate path to boot partition in '{}'",
            part_label.display()
        );
        return Err(Error::displayed());
    }
    create_dir(BALENA_BOOT_MP).upstream_with_context(&format!(
        "Failed to create balena-boot mountpoint: '{}'",
        BALENA_BOOT_MP
    ))?;

    if let Err(why) = mount(
        Some(&part_label),
        BALENA_BOOT_MP,
        Some(BALENA_BOOT_FSTYPE.as_bytes()),
        MsFlags::empty(),
        NIX_NONE,
    ) {
        error!(
            "Failed to mount '{}' to '{}', errr: {:?}",
            part_label.display(),
            BALENA_BOOT_MP,
            why
        );
        return Err(Error::displayed());
    }

    transfer_boot_files(BALENA_BOOT_MP)?;

    umount(BALENA_BOOT_MP).upstream_with_context(&format!(
        "Failed to unmount '{}' from '{}'",
        part_label.display(),
        BALENA_BOOT_MP
    ))?;

    let backup_path = path_append(TRANSFER_DIR, BACKUP_ARCH_NAME);
    if file_exists(&backup_path) {
        let part_label = path_append(DISK_BY_LABEL_PATH, BALENA_DATA_PART);
        if let Err(why) = mount(
            Some(&part_label),
            BALENA_BOOT_MP,
            Some(BALENA_DATA_FSTYPE.as_bytes()),
            MsFlags::empty(),
            NIX_NONE,
        ) {
            error!(
                "Failed to mount '{}' to '{}', error: {:?}",
                part_label.display(),
                BALENA_BOOT_MP,
                why
            );
            return Err(Error::displayed());
        }

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

fn transfer_files(device: &Path) -> Result<()> {
    debug!(
        "file '{}' exists: {}",
        device.display(),
        file_exists(device)
    );

    raw_mount_balena(device)
    /*
    match part_reread(device) {
        Ok(_) => {
            if file_exists(path_append(DISK_BY_LABEL_PATH, BALENA_BOOT_PART)) {
                sys_mount_balena()
            } else {
                raw_mount_balena(device)
            }
        }
        Err(_) => raw_mount_balena(device),
    }
    */
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

    let mut decoder = GzDecoder::new(match File::open(&image_path) {
        Ok(file) => file,
        Err(why) => {
            error!(
                "Validate: Failed to open image file '{}', error: {:?}",
                image_path.display(),
                why
            );
            return Err(Error::displayed());
        }
    });

    debug!("Validate: opening output file '{}'", target_path.display());
    let mut target = match OpenOptions::new()
        .write(false)
        .read(true)
        .create(false)
        .open(&target_path)
    {
        Ok(file) => file,
        Err(why) => {
            error!(
                "Validate: Failed to open output file '{}', error: {:?}",
                target_path.display(),
                why
            );
            return Err(Error::displayed());
        }
    };

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

fn flash_external(target_path: &Path, image_path: &Path) -> FlashState {
    let mut fail_res = FlashState::FailRecoverable;

    let mut decoder = GzDecoder::new(match File::open(&image_path) {
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
    match Command::new(BUSYBOX_CMD)
        .args(&[
            DD_CMD,
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

pub fn stage2(opts: &Options) {
    Logger::set_default_level(opts.get_s2_log_level());
    Logger::set_brief_info(false);
    Logger::set_color(true);

    if let Err(why) = Logger::set_log_dest(&LogDestination::BufferStderr, NO_STREAM) {
        error!("Failed to initialize logging, error: {:?}", why);
        busybox_reboot();
        return;
    }

    info!("Stage 2 migrate_worker entered");

    let s2_config = match read_stage2_config() {
        Ok(s2_config) => s2_config,
        Err(why) => {
            error!("Failed to read stage2 configuration, error: {:?}", why);
            busybox_reboot();
            return;
        }
    };

    info!("Stage 2 config was read successfully");

    setup_logging(&s2_config.log_dev);

    if (opts.get_s2_log_level() == Level::Debug) || (opts.get_s2_log_level() == Level::Trace) {
        use crate::common::debug::check_loop_control;
        check_loop_control("Stage2 init", "/dev");
    }

    //match kill_procs1(&["takeover"], 15) {

    let _res = kill_procs(opts.get_s2_log_level());

    match copy_files(&s2_config) {
        Ok(_) => (),
        Err(why) => {
            error!("Failed to copy files to RAMFS, error: {:?}", why);
            busybox_reboot();
            return;
        }
    }

    match unmount_partitions(&s2_config.umount_parts) {
        Ok(_) => (),
        Err(why) => {
            error!("unmount_partitions failed; {:?}", why);
            busybox_reboot();
        }
    }

    if (opts.get_s2_log_level() == Level::Debug) || (opts.get_s2_log_level() == Level::Trace) {
        use crate::common::debug::check_loop_control;
        check_loop_control("Stage2 before flash", "/dev");
    }

    if s2_config.pretend {
        info!("Not flashing due to pretend mode");
        busybox_reboot();
        return;
    }

    sync();

    let image_path = path_append(TRANSFER_DIR, BALENA_IMAGE_PATH);
    match flash_external(&s2_config.flash_dev, &image_path) {
        FlashState::Success => (),
        _ => {
            sleep(Duration::from_secs(10));
            busybox_reboot();
            return;
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

    if (opts.get_s2_log_level() == Level::Debug) || (opts.get_s2_log_level() == Level::Trace) {
        use crate::common::debug::check_loop_control;
        check_loop_control("Stage2 after flash", "/dev");
    }

    if let Err(why) = transfer_files(&s2_config.flash_dev) {
        error!("Failed to transfer files to balena OS, error: {:?}", why);
    } else {
        info!("Migration succeded successfully");
    }

    sync();

    busybox_reboot();
}
