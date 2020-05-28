use std::fs::{copy, create_dir, create_dir_all, read_dir, read_to_string, File, OpenOptions};
use std::io::{Read, Write};
use std::mem::MaybeUninit;
use std::os::raw::c_int;
use std::os::unix::io::AsRawFd;
use std::process::{exit, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use nix::{
    errno::{errno, Errno},
    fcntl::{fcntl, F_GETFD},
    ioctl_none,
    mount::{mount, umount, MsFlags},
    unistd::sync,
};

use std::path::{Path, PathBuf};

use failure::ResultExt;
use flate2::read::GzDecoder;
use libc::{
    close, getpid, sigfillset, sigprocmask, sigset_t, wait, MS_RDONLY, MS_REMOUNT, SIG_BLOCK,
};
use log::{debug, error, info, trace, warn, Level};
use mod_logger::{LogDestination, Logger};

use crate::common::{
    call,
    defs::{
        BALENA_BOOT_FSTYPE, BALENA_BOOT_MP, BALENA_BOOT_PART, BALENA_CONFIG_PATH,
        BALENA_IMAGE_PATH, BALENA_PART_MP, DD_CMD, DISK_BY_LABEL_PATH, FUSER_CMD, LOSETUP_CMD,
        NIX_NONE, OLD_ROOT_MP, PS_CMD, REBOOT_CMD, STAGE2_CONFIG_NAME, SYSTEM_CONNECTIONS_DIR,
        TRANSFER_DIR,
    },
    dir_exists,
    disk_util::{Disk, PartInfo, PartitionIterator, DEF_BLOCK_SIZE},
    file_exists, format_size_with_unit, get_mountpoint,
    mig_error::{MigErrCtx, MigError, MigErrorKind},
    options::Options,
    path_append,
    stage2_config::{Stage2Config, UmountPart},
};

const DD_BLOCK_SIZE: usize = 128 * 1024; // 4_194_304;

const VALIDATE_MAX_ERR: usize = 20;
const DO_VALIDATE: bool = true;
const VALIDATE_BLOCK_SIZE: usize = 64 * 1024; // 4_194_304;

pub const BUSYBOX_CMD: &str = "/busybox";

const BLK_IOC_MAGIC: u8 = 0x12;
const BLK_RRPART: u8 = 95;

ioctl_none!(blk_reread, BLK_IOC_MAGIC, BLK_RRPART);

fn reboot() {
    trace!("reboot entered");
    Logger::flush();
    sync();
    sleep(Duration::from_secs(3));
    let _cmd_res = call(BUSYBOX_CMD, &[REBOOT_CMD, "-f"], true);
    sleep(Duration::from_secs(1));
    info!("rebooting");
    exit(1);
}

fn read_stage2_config() -> Result<Stage2Config, MigError> {
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
                return Err(MigError::displayed());
            }
        };
        match Stage2Config::deserialze(&s2_cfg_txt) {
            Ok(s2_config) => Ok(s2_config),
            Err(why) => {
                error!("Failed to deserialize stage 2 config: error {}", why);
                return Err(MigError::displayed());
            }
        }
    } else {
        error!(
            "Stage2 config file could not be found in '{}',",
            s2_cfg_path.display()
        );
        return Err(MigError::displayed());
    }
}

fn setup_log<P: AsRef<Path>>(log_dev: P) -> Result<(), MigError> {
    let log_dev = log_dev.as_ref();
    trace!("setup_log entered with '{}'", log_dev.display());
    if log_dev.exists() {
        if let Some(mountpoint) = get_mountpoint(log_dev)? {
            if let Err(why) = umount(&mountpoint) {
                warn!(
                    "Failed to unmount log device '{}' from '{}', error: {:?}",
                    log_dev.display(),
                    mountpoint.display(),
                    why
                );
            } else {
                trace!("Unmounted '{}'", mountpoint.display())
            }
        }

        create_dir_all("/mnt/log").context(upstream_context!(
            "Failed to create log mount directory /mnt/log"
        ))?;

        trace!("Created log mountpoint: '/mnt/log'");
        mount(
            Some(log_dev),
            "/mnt/log",
            Some("vfat"),
            MsFlags::empty(),
            NIX_NONE,
        )
        .context(upstream_context!(&format!(
            "Failed to mount '{}' on /mnt/log",
            log_dev.display()
        )))?;

        trace!(
            "Mounted '{}' to log mountpoint: '/mnt/log'",
            log_dev.display()
        );
        // TODO: remove this later
        Logger::set_log_file(
            &LogDestination::Stderr,
            &PathBuf::from("/mnt/log/stage2-init.log"),
            false,
        )
        .context(upstream_context!(
            "Failed set log file to  '/mnt/log/stage2-init.log'"
        ))?;
        info!(
            "Now logging to /mnt/log/stage2-init.log on '{}'",
            log_dev.display()
        );
        Ok(())
    } else {
        warn!("Log device does not exist: '{}'", log_dev.display());
        Err(MigError::displayed())
    }
}

fn kill_procs(signal: &str) -> Result<(), MigError> {
    /*
    let cmd_res = call(BUSYBOX_CMD, &["fuser", "-m", OLD_ROOT_MP], true)?;
    if cmd_res.status.success() {
        info!("fuser: {}", cmd_res.stdout);
    }
    */
    debug!("kill_procs: entered");

    let cmd_res = call(
        BUSYBOX_CMD,
        &[FUSER_CMD, "-k", &format!("-{}", signal), "-m", OLD_ROOT_MP],
        true,
    )?;

    if !cmd_res.status.success() {
        warn!(
            "Failed to kill processes using '{}', stderr: {}",
            OLD_ROOT_MP, cmd_res.stderr
        );
    }
    Ok(())
}

fn unmount_partitions(mountpoints: &[UmountPart]) -> Result<(), MigError> {
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
                        return Err(MigError::displayed());
                    }
                }
            }
        }
    }
    Ok(())
}

fn part_reread(device: &Path) -> Result<i32, MigError> {
    // try ioctrl #define BLKRRPART  _IO(0x12,95)	/* re-read partition table */
    match OpenOptions::new()
        .read(true)
        .write(true)
        .create(false)
        .open(device)
    {
        Ok(file) => match unsafe { blk_reread(file.as_raw_fd()) } {
            Ok(res) => {
                debug!(
                    "Device BLKRRPART IOCTRL to '{}' returned {}",
                    device.display(),
                    res
                );
                Ok(res)
            }
            Err(why) => {
                error!(
                    "Device BLKRRPART IOCTRL to '{}' failed with error: {:?}",
                    device.display(),
                    why
                );
                Err(MigError::displayed())
            }
        },
        Err(why) => {
            error!(
                "Failed to open device '{}', error: {:?}",
                device.display(),
                why
            );
            Err(MigError::displayed())
        }
    }
}

fn transfer_boot_files<P: AsRef<Path>>(dev_root: P) -> Result<(), MigError> {
    let src_path = path_append(TRANSFER_DIR, BALENA_CONFIG_PATH);
    let target_path = path_append(dev_root.as_ref(), BALENA_CONFIG_PATH);
    copy(&src_path, &target_path).context(upstream_context!(&format!(
        "Failed to copy {} to {}",
        src_path.display(),
        target_path.display()
    )))?;

    info!("Successfully copied config.json to boot partition",);

    let src_path = path_append(TRANSFER_DIR, SYSTEM_CONNECTIONS_DIR);
    let dir_list = read_dir(&src_path).context(upstream_context!(&format!(
        "Failed to read directory '{}'",
        src_path.display()
    )))?;

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
                    .context(upstream_context!(&format!(
                        "Failed to read metadata from file '{}'",
                        curr_file.display()
                    )))?
                    .is_file()
                {
                    if let Some(filename) = curr_file.file_name() {
                        let target_path = path_append(&target_dir, filename);
                        copy(&curr_file, &target_path).context(upstream_context!(&format!(
                            "Failed to copy '{}' to '{}'",
                            curr_file.display(),
                            target_path.display()
                        )))?;
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
                return Err(MigError::displayed());
            }
        }
    }

    Ok(())
}

fn raw_mount_partition<P1: AsRef<Path>, P2: AsRef<Path>>(
    partition: &PartInfo,
    device: P2,
    mountpoint: P1,
    fs_type: &str,
) -> Result<String, MigError> {
    let cmd_res = call(BUSYBOX_CMD, &[LOSETUP_CMD, "-f"], true)?;
    let loop_dev = if cmd_res.status.success() {
        cmd_res.stdout
    } else {
        error!(
            "Failed determine next free loop device, stderr: {}",
            cmd_res.stderr
        );
        return Err(MigError::displayed());
    };

    let byte_offset = partition.start_lba * DEF_BLOCK_SIZE as u64;
    let args = &[
        LOSETUP_CMD,
        "-o",
        &byte_offset.to_string(),
        "-f",
        &*device.as_ref().to_string_lossy(),
    ];

    let cmd_res = call(BUSYBOX_CMD, args, true)?;
    if cmd_res.status.success() {
        info!(
            "Loop-mounted {} on '{}'",
            loop_dev,
            mountpoint.as_ref().display()
        );
        mount(
            Some(loop_dev.as_str()),
            mountpoint.as_ref(),
            Some(fs_type.as_bytes()),
            MsFlags::empty(),
            NIX_NONE,
        )
        .context(upstream_context!(&format!(
            "Failed to mount {} on {}",
            loop_dev,
            mountpoint.as_ref().display()
        )))?;
        Ok(loop_dev)
    } else {
        error!(
            "Failed to loop-mount partition, error: '{}'",
            cmd_res.stderr
        );
        Err(MigError::displayed())
    }
}

fn raw_umount_partition<P: AsRef<Path>>(device: &str, mountpoint: P) -> Result<(), MigError> {
    umount(mountpoint.as_ref()).context(upstream_context!(&format!(
        "Failed to unmount {}",
        mountpoint.as_ref().display()
    )))?;

    let cmd_res = call(BUSYBOX_CMD, &[LOSETUP_CMD, "-d", device], true)?;
    if !cmd_res.status.success() {
        error!(
            "Failed to remove loop device {}, stderr: {}",
            device, cmd_res.stderr
        );
        return Err(MigError::displayed());
    }

    Ok(())
}

fn raw_mount_balena(device: &Path) -> Result<(), MigError> {
    debug!("raw_mount_balena called");
    let mut disk = Disk::from_drive_file(device, None)?;
    let part_iterator = PartitionIterator::new(&mut disk)?;

    if !dir_exists(BALENA_PART_MP)? {
        create_dir(BALENA_PART_MP).context(upstream_context!(&format!(
            "Failed to create balena partition mountpoint: '{}'",
            BALENA_PART_MP
        )))?;
    }

    for partition in part_iterator {
        debug!(
            "partition: {}, start: {}, sectors: {}",
            partition.index, partition.start_lba, partition.num_sectors
        );

        match partition.index {
            1 => {
                // boot partition
                // losetup -o offset --sizelimit size --show -f log.img
                let loop_dev = raw_mount_partition(&partition, device, BALENA_PART_MP, "vfat")?;

                info!(
                    "Mounted boot partition as {} on {}",
                    loop_dev, BALENA_PART_MP
                );
                // TODO: copy files

                transfer_boot_files(BALENA_PART_MP)?;

                sync();

                raw_umount_partition(&loop_dev, BALENA_PART_MP)?;

                // TODO: check if backup.tgz is available - mount stick around if yes
                return Ok(());
            }
            _ => {
                error!("Invalid partition index encountered: {}", partition.index);
                return Err(MigError::displayed());
            }
        }
    }
    Ok(())
}

fn sys_mount_balena() -> Result<(), MigError> {
    debug!("sys_mount_balena called");
    sleep(Duration::from_secs(1));

    let part_label = path_append(DISK_BY_LABEL_PATH, BALENA_BOOT_PART);
    if !file_exists(&part_label) {
        error!(
            "Failed to locate path to boot partition in '{}'",
            part_label.display()
        );
        return Err(MigError::displayed());
    }
    create_dir(BALENA_BOOT_MP).context(upstream_context!(&format!(
        "Failed to create balena-boot mountpoint: '{}'",
        BALENA_BOOT_MP
    )))?;

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
        return Err(MigError::displayed());
    }

    Ok(())
}

fn transfer_files(device: &Path) -> Result<(), MigError> {
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
}

enum FlashState {
    Success,
    FailRecoverable,
    FailNonRecoverable,
}

fn fill_buffer<I: Read>(buffer: &mut [u8], input: &mut I) -> Result<usize, MigError> {
    // fill buffer
    let mut buff_fill: usize = 0;
    loop {
        let bytes_read = input
            .read(&mut buffer[buff_fill..])
            .context(upstream_context!("Failed to read from  input stream"))?;

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

fn validate(target_path: &Path, image_path: &Path) -> Result<bool, MigError> {
    debug!("Validate: opening: '{}'", image_path.display());

    let mut decoder = GzDecoder::new(match File::open(&image_path) {
        Ok(file) => file,
        Err(why) => {
            error!(
                "Validate: Failed to open image file '{}', error: {:?}",
                image_path.display(),
                why
            );
            return Err(MigError::displayed());
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
            return Err(MigError::displayed());
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

/*
fn flash_internal(target_path: &Path, image_path: &Path) -> FlashState {
    debug!("Flash: opening: '{}'", image_path.display());

    let mut decoder = GzDecoder::new(match File::open(&image_path) {
        Ok(file) => file,
        Err(why) => {
            error!(
                "Flash: Failed to open image file '{}', error: {:?}",
                image_path.display(),
                why
            );
            return FlashState::FailRecoverable;
        }
    });

    debug!("Flash: opening output file '{}", target_path.display());
    let mut out_file = match OpenOptions::new()
        .write(true)
        .read(false)
        .create(false)
        .open(&target_path)
    {
        Ok(file) => file,
        Err(why) => {
            error!(
                "Flash: Failed to open output file '{}', error: {:?}",
                target_path.display(),
                why
            );
            return FlashState::FailRecoverable;
        }
    };

    let start_time = Instant::now();
    let mut last_elapsed = Duration::new(0, 0);
    let mut write_count: usize = 0;

    let mut fail_res = FlashState::FailRecoverable;
    // TODO: might pay to put buffer on page boundary
    let mut buffer: [u8; DD_BLOCK_SIZE] = [0; DD_BLOCK_SIZE];
    loop {
        // fill buffer
        let buff_fill = match fill_buffer(&mut buffer, &mut decoder) {
            Ok(buff_fill) => buff_fill,
            Err(why) => {
                error!(
                    "Failed to read compressed data from '{}', error: {}:?",
                    image_path.display(),
                    why
                );
                return fail_res;
            }
        };

        if buff_fill > 0 {
            fail_res = FlashState::FailNonRecoverable;

            let bytes_written = match out_file.write(&buffer[0..buff_fill]) {
                Ok(bytes_written) => bytes_written,
                Err(why) => {
                    error!("Failed to write uncompressed data to dd, error {:?}", why);
                    return fail_res;
                }
            };

            write_count += bytes_written;

            if buff_fill != bytes_written {
                error!(
                    "Read/write count mismatch, read {}, wrote {}",
                    buff_fill, bytes_written
                );
                return fail_res;
            }

            let curr_elapsed = start_time.elapsed();
            let since_last = match curr_elapsed.checked_sub(last_elapsed) {
                Some(dur) => dur,
                None => Duration::from_secs(0),
            };

            if (since_last.as_secs() >= 10) || buff_fill < buffer.len() {
                last_elapsed = curr_elapsed;
                let secs_elapsed = curr_elapsed.as_secs();
                info!(
                    "{} written @ {}/sec in {} seconds",
                    format_size_with_unit(write_count as u64),
                    format_size_with_unit(write_count as u64 / secs_elapsed),
                    secs_elapsed
                );
                Logger::flush();
            }

            if buff_fill < buffer.len() {
                break;
            }
        } else {
            let secs_elapsed = start_time.elapsed().as_secs();
            info!(
                "{} written @ {}/sec in {} seconds",
                format_size_with_unit(write_count as u64),
                format_size_with_unit(write_count as u64 / secs_elapsed),
                secs_elapsed
            );
            Logger::flush();
            break;
        }
    }
    FlashState::Success
}
*/

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

pub fn stage2(_opts: Options) {
    info!("Stage 2 migrate_worker entered");

    let s2_config = match read_stage2_config() {
        Ok(s2_config) => s2_config,
        Err(why) => {
            error!("Failed to read stage2 configuration, error: {:?}", why);
            reboot();
            return;
        }
    };

    info!("Stage 2 config was read successfully");

    Logger::set_default_level(s2_config.get_log_level());
    if s2_config.log_dev.is_some() {
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

    info!("Stage 2 log level set to {:?}", s2_config.get_log_level());

    Logger::flush();
    sync();

    //match kill_procs1(&["takeover"], 15) {
    match kill_procs("TERM") {
        Ok(_) => (),
        Err(why) => {
            if let MigErrorKind::Displayed = why.kind() {
            } else {
                error!("kill_procs first attempt failed with error: {:?} ", why);
            }
            reboot();
        }
    }

    sleep(Duration::from_secs(5));

    match kill_procs("KILL") {
        Ok(_) => (),
        Err(why) => {
            if let MigErrorKind::Displayed = why.kind() {
            } else {
                error!("kill_procs second attempt failed with error: {:?} ", why);
            }
            reboot();
        }
    }

    if let Level::Trace = s2_config.get_log_level() {
        if let Ok(cmd_res) = call(BUSYBOX_CMD, &[PS_CMD, "-A"], true) {
            if cmd_res.status.success() {
                trace!("ps: {}", cmd_res.stdout);
            }
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
        return;
    }

    sync();

    let image_path = path_append(TRANSFER_DIR, BALENA_IMAGE_PATH);
    match flash_external(&s2_config.flash_dev, &image_path) {
        FlashState::Success => (),
        _ => {
            sleep(Duration::from_secs(10));
            reboot();
            return;
        }
    }

    sync();
    sleep(Duration::from_secs(1));

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

    if let Err(why) = transfer_files(&s2_config.flash_dev) {
        error!("Failed to transfer files to balena OS, error: {:?}", why);
    }

    sync();

    reboot();
}

pub fn init() {
    info!("Stage 2 entered");

    if unsafe { getpid() } != 1 {
        error!("Process must be pid 1 to run stage2");
        reboot();
        return;
    }

    info!("Stage 2 check pid success!");

    const START_FD: i32 = 0;
    let mut close_count = 0;
    for fd in START_FD..1024 {
        unsafe {
            match fcntl(fd, F_GETFD) {
                Ok(_) => {
                    close(fd);
                    close_count += 1;
                }
                Err(why) => {
                    if let Some(err_no) = why.as_errno() {
                        if let Errno::EBADF = err_no {
                            ()
                        } else {
                            warn!("Unexpected error from fcntl({},F_GETFD) : {}", fd, err_no);
                        }
                    } else {
                        warn!("Unexpected error from fcntl({},F_GETFD) : {}", fd, why);
                    }
                }
            }
        };
    }

    info!("Stage 2 closed {} fd's", close_count);

    let s2_config = match read_stage2_config() {
        Ok(s2_config) => s2_config,
        Err(why) => {
            error!("Failed to read stage2 configuration, error: {:?}", why);
            reboot();
            return;
        }
    };

    info!("Stage 2 config was read successfully");

    Logger::set_default_level(s2_config.get_log_level());

    info!("Stage 2 log level set to {:?}", s2_config.get_log_level());

    let ext_log = if let Some(log_dev) = s2_config.get_log_dev() {
        match setup_log(log_dev) {
            Ok(_) => true,
            Err(why) => {
                error!("Setup log failed, error: {:?}", why);
                false
            }
        }
    } else {
        false
    };

    info!("Stage 2 setup_log success!, ext_log: {}", ext_log);

    Logger::flush();
    sync();

    let _child_pid = match Command::new("./takeover").args(&["--stage2"]).spawn() {
        Ok(cmd_res) => cmd_res.id(),
        Err(why) => {
            error!("Failed to spawn stage2 worker process, error: {:?}", why);
            reboot();
            return;
        }
    };

    info!("Stage 2 migrate worker spawned");

    unsafe {
        let mut signals: sigset_t = MaybeUninit::<sigset_t>::zeroed().assume_init();
        let mut old_signals: sigset_t = MaybeUninit::<sigset_t>::zeroed().assume_init();

        sigfillset(&mut signals);
        sigprocmask(SIG_BLOCK, &signals, &mut old_signals);

        let mut status: c_int = MaybeUninit::<c_int>::zeroed().assume_init();

        let mut loop_count = 0;
        loop {
            let pid = wait(&mut status);
            loop_count += 1;
            if pid == -1 {
                let sys_error = errno();
                if sys_error == 10 {
                    sleep(Duration::from_secs(1));
                } else {
                    warn!("wait returned error, errno: {}", sys_error);
                }
            } else {
                trace!(
                    "Stage 2 wait loop {}, status: {}, pid: {}",
                    loop_count,
                    status,
                    pid
                );
            }
        }
    }
}
