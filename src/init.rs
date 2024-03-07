use crate::{
    common::{
        call, defs::{BALENA_DATA_MP, BALENA_OS_NAME, MOUNT_CMD, NIX_NONE, PIVOT_ROOT_CMD, TAKEOVER_DIR}, get_mountpoint, get_os_name, path_append, whereis, Error, Result, ToError
    },
    stage2::{read_stage2_config, reboot},
    ErrorKind,
};
use log::{error, info, trace, warn, Level};
use mod_logger::{LogDestination, Logger, NO_STREAM};
use nix::{
    errno::{errno, Errno},
    fcntl::{fcntl, F_GETFD},
    mount::{mount, umount, MsFlags},
    unistd::sync,
};
use std::env::set_current_dir;
use std::ffi::CString;
use std::fs::create_dir_all;
use std::io;
use std::mem::MaybeUninit;
use std::os::raw::c_int;
use std::process::Command;
use std::str::FromStr;
use std::thread::sleep;
use std::time::Duration;
use std::path::{PathBuf};

use crate::common::stage2_config::LogDevice;
use libc::{
    close, dup2, getpid, open, pipe, sigfillset, sigprocmask, sigset_t, wait, O_CREAT, O_TRUNC,
    O_WRONLY, SIG_BLOCK, STDERR_FILENO, STDIN_FILENO, STDOUT_FILENO,
};

const INITIAL_LOG_LEVEL: Level = Level::Trace;

fn setup_log(log_dev: &LogDevice, takeover_dir: &str) -> Result<()> {
    trace!(
        "setup_log entered with '{}', fs type: {}",
        log_dev.dev_name.display(),
        log_dev.fs_type
    );
    if log_dev.dev_name.exists() {
        if let Some(mountpoint) = get_mountpoint(log_dev.dev_name.as_path())? {
            if let Err(why) = umount(&mountpoint) {
                warn!(
                    "Failed to unmount log device '{}' from '{}', error: {:?}",
                    log_dev.dev_name.display(),
                    mountpoint.display(),
                    why
                );
            } else {
                trace!("Unmounted '{}'", mountpoint.display())
            }
        }

        let mountpoint = path_append(takeover_dir, "/mnt/log");
        create_dir_all(&mountpoint)
            .upstream_with_context("Failed to create log mount directory /mnt/log")?;

        trace!("Created log mountpoint: '{}'", mountpoint.display());

        // TODO: support other filesystem types
        mount(
            Some(&log_dev.dev_name),
            &mountpoint,
            Some(log_dev.fs_type.as_str()),
            MsFlags::empty(),
            NIX_NONE,
        )
        .upstream_with_context(&format!(
            "Failed to mount '{}' on '{}'",
            log_dev.dev_name.display(),
            mountpoint.display(),
        ))?;

        trace!(
            "Mounted '{}' to log mountpoint: '{}'",
            log_dev.dev_name.display(),
            mountpoint.display()
        );

        let logfile = path_append(&mountpoint, "stage2-init.log");
        Logger::set_log_file(&LogDestination::Stderr, &logfile, false)
            .upstream_with_context(&format!("Failed set log file to  '{}'", logfile.display()))?;
        info!(
            "Now logging to '{}' on '{}'",
            logfile.display(),
            log_dev.dev_name.display()
        );
        Ok(())
    } else {
        Err(Error::with_context(
            ErrorKind::InvParam,
            &format!(
                "The log device does not exist: '{}'",
                log_dev.dev_name.display()
            ),
        ))
    }
}

fn redirect_fd(file_name: &str, old_fd: c_int, mode: c_int) -> Result<()> {
    let filename = CString::new(file_name)
        .upstream_with_context(&format!("Invalid filename: '{}'", file_name))?
        .into_raw();

    let new_fd = unsafe { open(filename, mode) };

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

fn close_fds(takeover_dir: &str) -> Result<i32> {
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

    redirect_fd(
        &format!("{}/stdout.log", takeover_dir),
        STDOUT_FILENO,
        O_WRONLY | O_CREAT | O_TRUNC,
    )?;
    redirect_fd(
        &format!("{}/stderr.log", takeover_dir),
        STDERR_FILENO,
        O_WRONLY | O_CREAT | O_TRUNC,
    )?;

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
                    if why != Errno::EBADF {
                        warn!("Unexpected error from fcntl({},F_GETFD) : {}", fd, why);
                    }
                }
            }
        };
    }
    Ok(close_count)
}

#[allow(clippy::cognitive_complexity)]
pub fn init() -> ! {
    Logger::set_default_level(INITIAL_LOG_LEVEL);
    Logger::set_brief_info(false);
    Logger::set_color(true);

    if let Err(why) = Logger::set_log_dest(&LogDestination::BufferStderr, NO_STREAM) {
        error!("Failed to initialize logging, error: {:?}", why);
        reboot();
    }

    info!("Init entered");

    if unsafe { getpid() } != 1 {
        error!("Process must be pid 1 to run init");
        reboot();
    }

    info!("Init check pid success!");

    let mut takeover_path = PathBuf::from("");
    match get_os_name() {
        Ok(name) => {
            if name.starts_with(BALENA_OS_NAME) {
                takeover_path.push(BALENA_DATA_MP);
            }
        }
        Err(_) => {
            error!("Can't determine operating system name");
            reboot();
        }
    }
    if TAKEOVER_DIR.starts_with("/") {
        takeover_path.push(TAKEOVER_DIR[1..].to_string());
    } else {
        takeover_path.push(TAKEOVER_DIR);
    }
    let takeover_dir = takeover_path.to_str().unwrap_or(TAKEOVER_DIR);

    if let Err(why) = set_current_dir(&takeover_path) {
        error!(
            "Failed to change to directory '{}', error: {:?}",
            takeover_path.display(), why
        );
        reboot();
    }

    let s2_config = match read_stage2_config(Some(&takeover_path)) {
        Ok(s2_config) => s2_config,
        Err(why) => {
            error!("Failed to read stage2 configuration, error: {:?}", why);
            reboot();
        }
    };
    info!("Stage 2 config was read successfully");

    match Level::from_str(&s2_config.log_level) {
        Ok(level) => Logger::set_default_level(level),
        Err(why) => {
            warn!(
                "Failed to read log level from '{}', error: {:?}",
                s2_config.log_level, why
            );
        }
    }

    let closed_fds = match close_fds(takeover_dir) {
        Ok(fds) => fds,
        Err(_) => {
            error!("Failed close open files");
            reboot();
        }
    };
    info!("Stage 2 closed {} fd's", closed_fds);

    let ext_log = if let Some(log_dev) = s2_config.log_dev() {
        match setup_log(log_dev, takeover_dir) {
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

    match whereis(MOUNT_CMD) {
        Ok(mount_cmd) => {
            if let Err(why) = call_command!(
                mount_cmd.as_str(),
                &["--make-rprivate", "/"],
                "failed to mount / private"
            ) {
                error!(
                    "Failed to call '{} --make-rprivate /' error: {}",
                    mount_cmd, why
                );
                reboot();
            }
        }
        Err(why) => {
            error!("Failed to locate '{}' command, error: {}", MOUNT_CMD, why);
            reboot();
        }
    }

    match whereis(PIVOT_ROOT_CMD) {
        Ok(pivot_root_cmd) => {
            if let Err(why) = call_command!(
                pivot_root_cmd.as_str(),
                &[".", "mnt/old_root"],
                "Failed to pivot root"
            ) {
                error!(
                    "Failed to call '{} . mnt/old_root' error: {}",
                    pivot_root_cmd, why
                );
                reboot();
            }
        }
        Err(why) => {
            error!(
                "Failed to locate '{}' command, error: {}",
                PIVOT_ROOT_CMD, why
            );
            reboot();
        }
    }

    let _child_pid = match Command::new(&format!("/bin/{}", env!("CARGO_PKG_NAME")))
        .args(["--stage2", "--s2-log-level", &s2_config.log_level])
        .spawn()
    {
        Ok(cmd_res) => cmd_res.id(),
        Err(why) => {
            error!("Failed to spawn stage2 worker process, error: {:?}", why);
            reboot();
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
