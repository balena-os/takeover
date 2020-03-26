use std::mem::MaybeUninit;
use std::os::raw::c_int;
use std::thread::{self, sleep};
use std::process::exit;
use std::time::{Duration, Instant};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};

use nix::{
    mount::{mount, umount, MsFlags},
    unistd::{sync},
    errno::errno,
};
use std::path::{PathBuf, Path};
use std::collections::hash_set::HashSet;

use libc::{SIG_BLOCK, sigset_t, sigfillset, sigprocmask, close, wait, getpid, kill,  };
use mod_logger::{Logger, LogDestination, NO_STREAM};
use regex::Regex;
use flate2::read::GzDecoder;
use log::{trace, debug, warn , error, info};
use failure::ResultExt;

use crate::{
    common::{
        defs::{STAGE2_CONFIG_NAME, REBOOT_CMD, UMOUNT_CMD, DD_CMD, BALENA_IMAGE_PATH},
        call, file_exists, format_size_with_unit,
        options::Options,
        mig_error::{MigError, MigErrCtx, MigErrorKind},
        stage2_config::Stage2Config,
    }
};

use std::fs::{read_to_string, create_dir_all};
use crate::common::get_mountpoint;
use std::collections::HashMap;

const DD_BLOCK_SIZE: usize = 128 * 1024; // 4_194_304;
pub const NIX_NONE: Option<&'static [u8]> = None;

pub const BUSYBOX_CMD: &str = "/busybox";

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
                warn!("Failed to unmount log device '{}' from '{}', error: {:?}", log_dev.display(), mountpoint.display(), why);
            } else {
                trace!("Unmounted '{}'", mountpoint.display())
            }
        }

        create_dir_all("/mnt/log")
            .context(MigErrCtx::from_remark(MigErrorKind::Upstream, "Failed to create log mount directory /mnt/log"))?;

        trace!("Created log mountpoint: '/mnt/log'");
        mount(Some(log_dev), "/mnt/log", Some("ext4"), MsFlags::empty(), NIX_NONE)
            .context(MigErrCtx::from_remark(MigErrorKind::Upstream, &format!("Failed to mount '{}' on /mnt/log", log_dev.display())))?;

        trace!("Mounted '{}' to log mountpoint: '/mnt/log'", log_dev.display());
        // TODO: remove this later
        Logger::set_log_file(&LogDestination::Stderr, &PathBuf::from("/mnt/log/stage2.log"), false)
            .context(MigErrCtx::from_remark(MigErrorKind::Upstream, "Failed set log file to  '/mnt/log/stage2.log'"))?;
        info!("Now logging to /mnt/log/stage2.log on '{}'", log_dev.display());
        Ok(())
    } else {
        warn!("Log device does not exist: '{}'", log_dev.display());
        Err(MigError::displayed())
    }
}

fn kill_procs(filter: &[&str], signal: c_int) -> Result<(),MigError> {
    let cmd_res = call(BUSYBOX_CMD, &["lsof", "-t", "/old_root"], true)?;
    if cmd_res.status.success() {
        // trace!("unmount_root: parsing lsof output:\n{}", cmd_res.stdout);
        let mut nokill_pid_list : HashMap<i32, &str> = HashMap::new();
        let mut kill_pid_list : HashMap<i32, &str> = HashMap::new();
        let pid_re = Regex::new(r##"^(\d+)\s+(\S+)\s+(\S+)"##).unwrap();
        for line in cmd_res.stdout.lines().skip(1) {
            trace!("processing line: '{}'", line);
            if let Some(captures) = pid_re.captures(line) {
                let pid_str = captures.get(1).unwrap().as_str();
                let proc_ref = captures.get(2).unwrap().as_str();
                // let file_ref = captures.get(3).unwrap().as_str();

                if proc_ref.starts_with("/old_root") {
                    match pid_str.parse::<i32>() {
                        Ok(pid) => {
                            let no_kill = if let Some(proc) = PathBuf::from(proc_ref).file_name() {
                                if filter.contains(&&*proc.to_string_lossy()) {
                                    true
                                } else {
                                    false
                                }
                            } else {
                                false
                            };

                            if no_kill {
                                if let None = nokill_pid_list.insert(pid, proc_ref) {
                                    debug!("unmount_root: added pid to nokill list {}, {}", pid, proc_ref);
                                } else {
                                    debug!("unmount_root: pid {} was already in list", pid);
                                }
                            } else {
                                if let None = kill_pid_list.insert(pid, proc_ref) {
                                    debug!("unmount_root: added pid to kill list {}, {}", pid, proc_ref);
                                } else {
                                    debug!("unmount_root: pid {} was already in list", pid);
                                }
                            }
                        },
                        Err(why) => {
                            warn!("Failed to parse pid from '{}' in lsof line: '{}', error: {:?}", pid_str, line, why);
                        }
                    }
                } else {
                    warn!("unmount_root: Skipping lsof line '{}' not refering to /old_root", line);
                }
            } else {
                warn!("Failed to parse lsof line: '{}'", line);
            }
        }

        for (pid, proc_ref) in kill_pid_list.iter() {
            info!("unmount_root: attempting to kill process {}, {}", pid, proc_ref);
            let res = unsafe { kill(*pid,signal) };
            if res == 0 {
                info!("Killed process {}", pid);
            } else {
                warn!("Failed to kill process {}, error: {}", pid, errno());
            }
        }

        // TODO: another roud of kill with -9
    } else {
        error!("call to lsof failed with error: '{}'", cmd_res.stderr);
        return Err(MigError::displayed());
    }
    Ok(())
}

fn unmount_flash_dev(flash_dev: &Path) -> Result<(), MigError> {
    match umount(flash_dev) {
        Ok(_) => {
            info!("Successfully unmounted old root");
            Ok(())
        },
        Err(why) => {
            warn!("Failed to unmount old root, error : {:?} ", why);
            let cmd_res = call(BUSYBOX_CMD, &[UMOUNT_CMD, "-l", &*flash_dev.to_string_lossy()],true )?;
            if !cmd_res.status.success() {
                error!("Failed to unmount old root, stderr: '{}'", cmd_res.stderr);
                Err(MigError::displayed())
            } else {
                Ok(())
            }
        }
    }
}

fn flash_gzip_internal(target_path: &Path, image_path: &Path) -> bool {
    debug!("opening: '{}'", image_path.display());

    let mut decoder = GzDecoder::new(match File::open(&image_path) {
        Ok(file) => file,
        Err(why) => {
            error!(
                "Failed to open image file '{}', error: {:?}",
                image_path.display(),
                why
            );
            return true;
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
            return true;
        }
    };

    let start_time = Instant::now();
    let mut last_elapsed = Duration::new(0, 0);
    let mut write_count: usize = 0;

    let mut fail_res = true;
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
            fail_res = false;

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
    true
}


fn migrate_worker(opts: Options, s2_config: &Stage2Config) {
    info!("Stage 2 migrate_worker entered");

    match kill_procs(&["takeover"], 15) {
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

    match kill_procs(&[], 9) {
        Ok(_) => (),
        Err(why) => {
            if let MigErrorKind::Displayed = why.kind() {

            } else {
                error!("kill_procs second attempt failed with error: {:?} ", why);
            }
            reboot();
        }
    }


    match unmount_flash_dev(&s2_config.flash_dev) {
        Ok(_) => (),
        Err(why) => {
            error!("unmount_root failed; {:?}", why);
            reboot();
        }
    }

    let _recoverable = flash_gzip_internal(&s2_config.flash_dev, &PathBuf::from(BALENA_IMAGE_PATH));

    sleep(Duration::from_secs(10));
    reboot();
}

pub fn stage2(opts: Options) -> Result<(), MigError> {
    info!("Stage 2 entered");

    if unsafe { getpid() } != 1 {
        error!("Process must be pid 1 to run action stage2");
        reboot()
    }

    info!("Stage 2 check pid success!");

    const START_FD: i32 = 0;
    for i in START_FD..1024 {
        unsafe { close(i) };
    }

    info!("Stage 2 closed fd's {} to 1024", START_FD);

    let s2_cfg_path = PathBuf::from(&format!("/{}",STAGE2_CONFIG_NAME));
    let s2_config = if file_exists(&s2_cfg_path) {
        let s2_cfg_txt = match read_to_string(&s2_cfg_path) {
            Ok(s2_config_txt) => s2_config_txt,
            Err(why) => {
                error!("Failed to read stage 2 config from '{}'", s2_cfg_path.display());
                reboot();
                return Err(MigError::displayed());
            }
        };
        match Stage2Config::deserialze(&s2_cfg_txt) {
            Ok(s2_config) => s2_config,
            Err(why) => {
                error!("Failed to deserialize stage 2 config");
                reboot();
                return Err(MigError::displayed());
            }
        }
    } else {
        error!("Stage2 config file could not be found in '{}',", s2_cfg_path.display());
        reboot();
        return Err(MigError::displayed());
    };

    info!("Stage 2 config was read successfully");

    Logger::set_default_level(&s2_config.get_log_level());

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
    thread::spawn( move || {
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
                warn!("wait returned error, errno: {}", sys_error);
                if sys_error == 10 {
                    sleep(Duration::from_secs(1));
                }
            } else {
                trace!("Stage 2 wait loop {}, status: {}, pid: {}", loop_count, status, pid);
            }

        }
    }

    // should be unreachable
    reboot();
    Ok(())
}
