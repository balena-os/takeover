use crate::{
    common::{defs::NIX_NONE, get_mountpoint, MigErrCtx, MigError, MigErrorKind},
    stage2::{busybox_reboot, read_stage2_config},
};
use log::{error, info, trace, warn};
use mod_logger::{LogDestination, Logger};
use std::fs::create_dir_all;
use std::mem::MaybeUninit;
use std::os::raw::c_int;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread::sleep;
use std::time::Duration;

use failure::ResultExt;

use nix::{
    errno::{errno, Errno},
    fcntl::{fcntl, F_GETFD},
    mount::{mount, umount, MsFlags},
    unistd::sync,
};

use libc::{close, getpid, sigfillset, sigprocmask, sigset_t, wait, SIG_BLOCK};

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

fn close_fds() -> i32 {
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
    close_count
}

pub fn init() {
    info!("Stage 2 entered");

    if unsafe { getpid() } != 1 {
        error!("Process must be pid 1 to run stage2");
        busybox_reboot();
        return;
    }

    info!("Stage 2 check pid success!");

    info!("Stage 2 closed {} fd's", close_fds());

    let s2_config = match read_stage2_config() {
        Ok(s2_config) => s2_config,
        Err(why) => {
            error!("Failed to read stage2 configuration, error: {:?}", why);
            busybox_reboot();
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
            busybox_reboot();
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
