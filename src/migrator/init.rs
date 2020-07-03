use crate::{
    common::{defs::NIX_NONE, get_mountpoint, Error, Options, Result, ToError},
    stage2::{busybox_reboot, read_stage2_config},
    ErrorKind,
};
use log::{error, info, trace, warn};
use mod_logger::{LogDestination, Logger, NO_STREAM};
use std::ffi::CString;
use std::fs::create_dir_all;
use std::io;
use std::mem::MaybeUninit;
use std::os::raw::c_int;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread::sleep;
use std::time::Duration;

use nix::{
    errno::{errno, Errno},
    fcntl::{fcntl, F_GETFD},
    mount::{mount, umount, MsFlags},
    unistd::sync,
};

use libc::{
    close, dup2, getpid, open, pipe, sigfillset, sigprocmask, sigset_t, wait, O_CREAT, O_TRUNC,
    O_WRONLY, SIG_BLOCK, STDERR_FILENO, STDIN_FILENO, STDOUT_FILENO,
};

fn setup_log<P: AsRef<Path>>(log_dev: P) -> Result<()> {
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

        create_dir_all("/mnt/log")
            .upstream_with_context("Failed to create log mount directory /mnt/log")?;

        trace!("Created log mountpoint: '/mnt/log'");
        mount(
            Some(log_dev),
            "/mnt/log",
            Some("vfat"),
            MsFlags::empty(),
            NIX_NONE,
        )
        .upstream_with_context(&format!(
            "Failed to mount '{}' on /mnt/log",
            log_dev.display()
        ))?;

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
        .upstream_with_context("Failed set log file to  '/mnt/log/stage2-init.log'")?;
        info!(
            "Now logging to /mnt/log/stage2-init.log on '{}'",
            log_dev.display()
        );
        Ok(())
    } else {
        Err(Error::with_context(
            ErrorKind::InvParam,
            &format!("The log device does not exist: '{}'", log_dev.display()),
        ))
    }
}

fn redirect_fd(file_name: &str, old_fd: c_int, mode: c_int) -> Result<()> {
    let filename = CString::new(file_name)
        .upstream_with_context(&format!("Invalid filename: '{}'", file_name))?
        .into_raw();

    let new_fd = unsafe { open(filename, mode) };
    unsafe { CString::from_raw(filename) };
    if new_fd >= 0 {
        let res = unsafe { dup2(new_fd, old_fd) };
        if res >= 0 {
            unsafe { close(new_fd) };
            Ok(())
        } else {
            Err(Error::with_context(
                ErrorKind::ExecProcess,
                &format!(
                    "Failed to redirect STDOUT to '{}', error: {}",
                    file_name,
                    io::Error::last_os_error()
                ),
            ))
        }
    } else {
        Err(Error::with_context(
            ErrorKind::Upstream,
            &format!(
                "Failed to open '{}', error: {}",
                file_name,
                io::Error::last_os_error()
            ),
        ))
    }
}

fn close_fds() -> Result<i32> {
    let mut pipe_fds: [c_int; 2] = [0; 2];
    let sys_rc = unsafe { pipe(pipe_fds.as_mut_ptr()) };

    if sys_rc >= 0 {
        let sys_rc = unsafe { dup2(pipe_fds[0], STDIN_FILENO) };
        if sys_rc >= 0 {
            let _sys_rc = unsafe { close(pipe_fds[0]) };
        } else {
            return Err(Error::with_context(
                ErrorKind::Upstream,
                &format!(
                    "Failed to dup2 pipe read handle to stdin, error: {}",
                    io::Error::last_os_error()
                ),
            ));
        }
    } else {
        return Err(Error::with_context(
            ErrorKind::Upstream,
            &format!(
                "Failed to create pipe for stdin, error: {}",
                io::Error::last_os_error()
            ),
        ));
    }

    redirect_fd("/stdout.log", STDOUT_FILENO, O_WRONLY | O_CREAT | O_TRUNC)?;
    redirect_fd("/stderr.log", STDERR_FILENO, O_WRONLY | O_CREAT | O_TRUNC)?;

    const START_FD: i32 = 3;
    let mut close_count = 1;
    for fd in START_FD..1024 {
        if fd == pipe_fds[1] {
            // dont't close the stdin pipe
            continue;
        }
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
    Ok(close_count)
}

pub fn init(opts: &Options) -> ! {
    Logger::set_default_level(opts.get_s2_log_level());
    Logger::set_brief_info(false);
    Logger::set_color(true);

    if let Err(why) = Logger::set_log_dest(&LogDestination::BufferStderr, NO_STREAM) {
        error!("Failed to initialize logging, error: {:?}", why);
        busybox_reboot();
    }

    info!("Stage 2 entered");

    if unsafe { getpid() } != 1 {
        error!("Process must be pid 1 to run init");
        busybox_reboot();
    }

    info!("Stage 2 check pid success!");

    let closed_fds = match close_fds() {
        Ok(fds) => fds,
        Err(_) => {
            error!("Failed close open files");
            busybox_reboot();
        }
    };
    info!("Stage 2 closed {} fd's", closed_fds);

    let s2_config = match read_stage2_config() {
        Ok(s2_config) => s2_config,
        Err(why) => {
            error!("Failed to read stage2 configuration, error: {:?}", why);
            busybox_reboot();
        }
    };

    info!("Stage 2 config was read successfully");

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

    let _child_pid = match Command::new("./takeover")
        .args(&[
            "--stage2",
            "--s2-log-level",
            opts.get_s2_log_level().to_string().as_str(),
        ])
        .spawn()
    {
        Ok(cmd_res) => cmd_res.id(),
        Err(why) => {
            error!("Failed to spawn stage2 worker process, error: {:?}", why);
            busybox_reboot();
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
