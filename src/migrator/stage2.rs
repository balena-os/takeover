use std::fs::{copy, create_dir, create_dir_all, read_to_string, File, OpenOptions};
use std::io::{Read, Write};
use std::mem::MaybeUninit;
use std::os::raw::c_int;
use std::os::unix::io::AsRawFd;
use std::process::exit;
use std::thread::{self, sleep};
use std::time::{Duration, Instant};

use nix::{
    errno::{errno, Errno},
    fcntl::{fcntl, F_GETFD},
    ioctl_none,
    mount::{mount, umount, umount2, MntFlags, MsFlags},
    unistd::sync,
};

use std::path::{Path, PathBuf};

use failure::ResultExt;
use flate2::read::GzDecoder;
use libc::{
    close, getpid, sigfillset, sigprocmask, sigset_t, wait, MNT_DETACH, MNT_FORCE, SIG_BLOCK,
};
use log::{debug, error, info, trace, warn};
use mod_logger::{LogDestination, Logger};

use crate::common::{
    call,
    defs::{BALENA_IMAGE_PATH, OLD_ROOT_MP, REBOOT_CMD, STAGE2_CONFIG_NAME, UMOUNT_CMD},
    dir_exists, file_exists, format_size_with_unit,
    mig_error::{MigErrCtx, MigError, MigErrorKind},
    options::Options,
    stage2_config::Stage2Config,
};

use crate::common::{
    defs::{
        BALENA_BOOT_FSTYPE, BALENA_BOOT_MP, BALENA_BOOT_PART, BALENA_CONFIG_PATH, BALENA_PART_MP,
        DISK_BY_LABEL_PATH, NIX_NONE,
    },
    disk_util::{Disk, PartitionIterator, DEF_BLOCK_SIZE},
    get_mountpoint, path_append,
};

const DD_BLOCK_SIZE: usize = 128 * 1024; // 4_194_304;

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
            &PathBuf::from("/mnt/log/stage2.log"),
            false,
        )
        .context(upstream_context!(
            "Failed set log file to  '/mnt/log/stage2.log'"
        ))?;
        info!(
            "Now logging to /mnt/log/stage2.log on '{}'",
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
    let cmd_res = call(
        BUSYBOX_CMD,
        &["fuser", "-k", &format!("-{}", signal), "-m", OLD_ROOT_MP],
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

fn unmount_partitions(mountpoints: &Vec<PathBuf>) -> Result<(), MigError> {
    for mpoint in mountpoints {
        let mountpoint = path_append(OLD_ROOT_MP, mpoint);

        match umount2(
            &mountpoint,
            MntFlags::from_bits(MNT_FORCE | MNT_DETACH).unwrap(),
        ) {
            Ok(_) => {
                info!("Successfully unmounted '{}", mountpoint.display());
            }
            Err(why) => {
                error!(
                    "Failed to unmount partition '{}', error : {:?} ",
                    mountpoint.display(),
                    why
                );
                let cmd_res = call(
                    BUSYBOX_CMD,
                    &[UMOUNT_CMD, "-l", &*mountpoint.to_string_lossy()],
                    true,
                )?;
                if !cmd_res.status.success() {
                    error!(
                        "Failed to unmount '{}', stderr: '{}'",
                        mountpoint.display(),
                        cmd_res.stderr
                    );
                    return Err(MigError::displayed());
                } else {
                    info!("Successfully unmounted '{}'", mountpoint.display());
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

                let cmd_res = call(BUSYBOX_CMD, &["losetup", "-f"], true)?;
                let loop_dev = if cmd_res.status.success() {
                    cmd_res.stdout
                } else {
                    error!(
                        "Failed determine next free loop device, stderr: {}",
                        cmd_res.stderr
                    );
                    return Err(MigError::displayed());
                };

                let byte_offset = (partition.start_lba * DEF_BLOCK_SIZE as u64).to_string();
                let args = &[
                    "losetup",
                    "-o",
                    byte_offset.as_str(),
                    "-f",
                    &*device.to_string_lossy(),
                ];

                let cmd_res = call(BUSYBOX_CMD, args, true)?;
                if cmd_res.status.success() {
                    info!("Loop-mounted boot partition on '{}'", loop_dev);
                    mount(
                        Some(loop_dev.as_str()),
                        BALENA_PART_MP,
                        Some(BALENA_BOOT_FSTYPE.as_bytes()),
                        MsFlags::empty(),
                        NIX_NONE,
                    )
                    .context(upstream_context!(&format!(
                        "Failed to mount {} on {}",
                        loop_dev, BALENA_PART_MP
                    )))?;

                    info!(
                        "Mounted boot partition as {} on {}",
                        loop_dev, BALENA_PART_MP
                    );
                    // TODO: copy files

                    let target_path = path_append(BALENA_PART_MP, BALENA_CONFIG_PATH);
                    copy(BALENA_CONFIG_PATH, &target_path).context(upstream_context!(&format!(
                        "Failed to copy {} to {}",
                        BALENA_CONFIG_PATH,
                        target_path.display()
                    )))?;

                    info!("Successfully copied config.json to boot partition",);

                    umount(BALENA_PART_MP).context(upstream_context!(&format!(
                        "Failed to unmount {}",
                        BALENA_PART_MP
                    )))?;

                    let cmd_res = call(BUSYBOX_CMD, &["losetup", "-d", loop_dev.as_str()], true)?;
                    if !cmd_res.status.success() {
                        error!(
                            "Failed to remove loop device {}, stderr: {}",
                            loop_dev, cmd_res.stderr
                        );
                        return Err(MigError::displayed());
                    }
                } else {
                    error!("Failed to mount boot partition, stderr: {}", cmd_res.stderr);
                    return Err(MigError::displayed());
                }
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

fn mount_balena(device: &Path) -> Result<(), MigError> {
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

fn flash_gzip_internal(target_path: &Path, image_path: &Path) -> FlashState {
    debug!("opening: '{}'", image_path.display());

    let mut decoder = GzDecoder::new(match File::open(&image_path) {
        Ok(file) => file,
        Err(why) => {
            error!(
                "Failed to open image file '{}', error: {:?}",
                image_path.display(),
                why
            );
            return FlashState::FailRecoverable;
        }
    });

    /* debug!("invoking dd");

    let mut dd_child = match Command::new(dd_cmd)
        .args(&[
            // "conv=fsync", sadly not supported on busybox dd
            // "oflag=direct",
            &format!("of={}", &target_path.to_string_lossy()),
            &format!("bs={}", DD_BLOCK_SIZE),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit()) // test
        .stderr(Stdio::inherit())
        .spawn()
    {
        Ok(dd_child) => dd_child,
        Err(why) => {
            error!("failed to execute command {}, error: {:?}", dd_cmd, why);
            Logger::flush();
            return FlashResult::FailRecoverable;
        }
    };
    */

    debug!("opening output file '{}", target_path.display());
    let mut out_file = match OpenOptions::new()
        .write(true)
        .read(false)
        .create(false)
        .open(&target_path)
    {
        Ok(file) => file,
        Err(why) => {
            error!(
                "Failed to open output file '{}', error: {:?}",
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
        let mut buff_fill: usize = 0;
        loop {
            let bytes_read = match decoder.read(&mut buffer[buff_fill..]) {
                Ok(bytes_read) => bytes_read,
                Err(why) => {
                    error!(
                        "Failed to read uncompressed data from '{}', error: {:?}",
                        image_path.display(),
                        why
                    );
                    return fail_res;
                }
            };

            if bytes_read > 0 {
                buff_fill += bytes_read;
                if buff_fill < buffer.len() {
                    continue;
                }
            }
            break;
        }

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

            if since_last.as_secs() >= 10 {
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
            break;
        }
    }
    FlashState::Success
}

fn migrate_worker(_opts: Options, s2_config: &Stage2Config) {
    info!("Stage 2 migrate_worker entered");

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

    // match kill_procs(&[], 9) {
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

    if let Ok(cmd_res) = call(BUSYBOX_CMD, &["ps", "-A"], true) {
        if cmd_res.status.success() {
            info!("ps: {}", cmd_res.stdout);
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

    match flash_gzip_internal(&s2_config.flash_dev, &PathBuf::from(BALENA_IMAGE_PATH)) {
        FlashState::Success => (),
        _ => {
            sleep(Duration::from_secs(10));
            reboot();
            return;
        }
    }

    sync();
    sleep(Duration::from_secs(1));

    if let Err(_why) = mount_balena(&s2_config.flash_dev) {
        error!("Failed to mount balena drives");
        sleep(Duration::from_secs(10));
        reboot();
        return;
    }

    let target_path = path_append(BALENA_BOOT_MP, BALENA_CONFIG_PATH);
    if let Err(why) = copy(BALENA_CONFIG_PATH, &target_path) {
        error!(
            "Failed to copy '{}' to '{}, error: {:?}",
            BALENA_CONFIG_PATH,
            target_path.display(),
            why
        );
        sleep(Duration::from_secs(10));
        reboot();
        return;
    }

    sync();

    reboot();
}

pub fn stage2(opts: Options) -> Result<(), MigError> {
    info!("Stage 2 entered");

    if unsafe { getpid() } != 1 {
        error!("Process must be pid 1 to run stage2");
        reboot()
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

    let s2_cfg_path = PathBuf::from(&format!("/{}", STAGE2_CONFIG_NAME));
    let s2_config = if file_exists(&s2_cfg_path) {
        let s2_cfg_txt = match read_to_string(&s2_cfg_path) {
            Ok(s2_config_txt) => s2_config_txt,
            Err(why) => {
                error!(
                    "Failed to read stage 2 config from '{}', error: {}",
                    s2_cfg_path.display(),
                    why
                );
                reboot();
                return Err(MigError::displayed());
            }
        };
        match Stage2Config::deserialze(&s2_cfg_txt) {
            Ok(s2_config) => s2_config,
            Err(why) => {
                error!("Failed to deserialize stage 2 config: error {}", why);
                reboot();
                return Err(MigError::displayed());
            }
        }
    } else {
        error!(
            "Stage2 config file could not be found in '{}',",
            s2_cfg_path.display()
        );
        reboot();
        return Err(MigError::displayed());
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

    info!("Stage 2 setup_log success!, ext_log = {}", ext_log);

    Logger::flush();
    sync();

    let worker_opts = opts.clone();
    thread::spawn(move || {
        migrate_worker(worker_opts, &s2_config);
    });

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
