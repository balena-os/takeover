use crate::{
    common::{
        call,
        defs::{MOUNT_CMD, NIX_NONE, OLD_ROOT_MP, PIVOT_ROOT_CMD, TAKEOVER_DIR},
        dir_exists, get_mountpoint,
        logging::{
            copy_file_to_destination_dir, open_fallback_log_file,
            persist_fallback_log_to_data_partition,
        },
        path_append, reboot,
        stage2_config::Stage2Config,
        whereis, Error, Result, ToError,
    },
    stage1::api_calls::notify_hup_progress,
    stage2::read_stage2_config,
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
use std::fs::{copy, create_dir_all};
use std::io;
use std::mem::MaybeUninit;
use std::os::raw::c_int;
use std::path::PathBuf;
use std::process::Command;
use std::str::FromStr;
use std::thread::sleep;
use std::time::Duration;

use crate::common::stage2_config::LogDevice;
use libc::{
    close, dup2, getpid, open, pipe, sigfillset, sigprocmask, sigset_t, wait, O_CREAT, O_TRUNC,
    O_WRONLY, SIG_BLOCK, STDERR_FILENO, STDIN_FILENO, STDOUT_FILENO,
};

const INITIAL_LOG_LEVEL: Level = Level::Trace;
const CERTS_DIR: &str = "/etc/ssl/certs";

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
    let takeover_path = PathBuf::from(TAKEOVER_DIR);

    let s2_config = match read_stage2_config(Some(&takeover_path)) {
        Ok(s2_config) => s2_config,
        Err(why) => {
            error!("Failed to read stage2 configuration, error: {:?}", why);
            reboot();
        }
    };

    info!("Stage 2 config was read successfully");

    if unsafe { getpid() } != 1 {
        error!("Process must be pid 1 to run init");
        stage2_init_err_handler(true, &s2_config);
    }

    info!("Init check pid success!");

    if let Err(why) = set_current_dir(&takeover_path) {
        error!(
            "Failed to change to directory '{}', error: {:?}",
            takeover_path.display(),
            why
        );
        stage2_init_err_handler(true, &s2_config);
    }

    match Level::from_str(&s2_config.log_level) {
        Ok(level) => Logger::set_default_level(level),
        Err(why) => {
            warn!(
                "Failed to read log level from '{}', error: {:?}",
                s2_config.log_level, why
            );
        }
    }

    let closed_fds = match close_fds(TAKEOVER_DIR) {
        Ok(fds) => fds,
        Err(_) => {
            error!("Failed close open files");
            stage2_init_err_handler(true, &s2_config);
        }
    };
    info!("Stage 2 closed {} fd's", closed_fds);

    // Logs are buffered prior to closing all open file descriptors
    // After running `close_fds`, we check the logging options passed to takeover
    // --fallback-log and --log-to are mutually exclusive
    // We can setup external logging here
    if !s2_config.fallback_log {
        let ext_log = if let Some(log_dev) = s2_config.log_dev() {
            match setup_log(log_dev, TAKEOVER_DIR) {
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
    }

    /******************************************************************
     * Pivot Root
     ******************************************************************/
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
                stage2_init_err_handler(true, &s2_config);
            }
        }
        Err(why) => {
            error!("Failed to locate '{}' command, error: {}", MOUNT_CMD, why);
            stage2_init_err_handler(true, &s2_config);
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
                stage2_init_err_handler(true, &s2_config);
            }
        }
        Err(why) => {
            error!(
                "Failed to locate '{}' command, error: {}",
                PIVOT_ROOT_CMD, why
            );
            stage2_init_err_handler(true, &s2_config);
        }
    }
    /******************************************************************
     * After this point, paths are relative to OLD_ROOT_MP
     ******************************************************************/

    // If fallback log option was passed
    // We setup logging to file on tmpfs
    // Before this point, stage2-init logs was sent to a buffer
    if s2_config.fallback_log {
        // copy the shared log file from OLD_ROOT_MP
        let source_tmp_log_path =
            format!("{}/tmp/{}", OLD_ROOT_MP, s2_config.fallback_log_filename);
        let dest_dir_path = "/tmp";

        match copy_file_to_destination_dir(&source_tmp_log_path, dest_dir_path) {
            Ok(_) => info!("Copied logfile from {}", source_tmp_log_path),
            Err(_) => error!("Could not copy logfile from {}", source_tmp_log_path),
        }

        setup_stage2_init_fallback_log(&s2_config.fallback_log_filename);
    }
    // Required to send HUP progress messages to balena API.
    match setup_networking() {
        Ok(_) => info!("Networking setup success"),
        Err(why) => {
            error!("Failed to setup networking, error: {:?}", why);
        }
    }

    let _child_pid = match Command::new(format!("/bin/{}", env!("CARGO_PKG_NAME")))
        .args(["--stage2", "--s2-log-level", &s2_config.log_level])
        .spawn()
    {
        Ok(cmd_res) => cmd_res.id(),
        Err(why) => {
            error!("Failed to spawn stage2 worker process, error: {:?}", why);
            stage2_init_err_handler(false, &s2_config);
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

// mod_logger needs to be called separately per module
fn setup_stage2_init_fallback_log(fallback_log_filename: &str) {
    info!(
        "stage2-init:: Setting up temporary log destination to /tmp/{}",
        fallback_log_filename
    );
    Logger::set_color(false);

    let log_file = open_fallback_log_file(fallback_log_filename);

    if log_file.is_some() {
        // Logger::set_log_dest flushes the buffer before setting the log dest
        // We do not use Logger::set_log_file because it truncates the file
        // So we get the buffer before calling Logger::set_log_dest, and then write the
        // buffered logs to the file
        let buffered_logs = Logger::get_buffer();

        match Logger::set_log_dest(&LogDestination::StreamStderr, log_file) {
            Ok(_) => {
                if let Some(buffer) = buffered_logs {
                    // Attempt to convert Vec<u8> to String
                    match String::from_utf8(buffer) {
                        Ok(logs) => info!("\n{}", logs),
                        Err(error) => error!("Error: {}", error),
                    }
                }
                info!(
                    "stage2-init:: Now logging to /tmp/{}",
                    fallback_log_filename
                );
            }
            Err(_) => {
                error!(
                    "Could not set logging to tmpfs at /tmp/{}",
                    fallback_log_filename
                )
            }
        }
    }
}

// Copy files required for reqwest based networking operation, as used to
// send HUP progress messages to balenaCloud. Files must be available relative
// to new root directory. Includes SSL certificates and resolv.dnsmasq.
fn setup_networking() -> Result<()> {
    if !dir_exists(CERTS_DIR)? {
        create_dir_all(CERTS_DIR).upstream_with_context(&format!(
            "Failed to create certs directory: '{}'",
            CERTS_DIR
        ))?;
    }

    let src_path = path_append(OLD_ROOT_MP, "/etc/ssl/certs/ca-certificates.crt");
    let to_path = path_append(CERTS_DIR, "ca-certificates.crt");
    copy(&src_path, &to_path).upstream_with_context(&format!(
        "Failed to copy '{}' to {}",
        src_path.display(),
        &to_path.display()
    ))?;
    info!(
        "Copied certs from {} to '{}'",
        src_path.display(),
        to_path.display()
    );

    let src_path = path_append(OLD_ROOT_MP, "/var/run/resolvconf/interface/NetworkManager");
    let to_path = path_append("/etc", "resolv.conf");
    copy(&src_path, &to_path).upstream_with_context(&format!(
        "Failed to copy '{}' to {}",
        src_path.display(),
        &to_path.display()
    ))?;
    info!(
        "Copied DNS resolver from {} to '{}'",
        src_path.display(),
        to_path.display()
    );

    Ok(())
}

// Helper function to handle errors during stage2-init process
// * `pre_privot_root` - bool to indicate whether handling err
//
// prior to pivot_root command being called. This is relevant
// because if using fallback log mechanism, logs are still in memory
// before the calling pivot_root. We attempt to setup log to tmpfs
// in that case.
fn stage2_init_err_handler(pre_pivot_root: bool, s2_config: &Stage2Config) -> ! {
    // if handling err before pivot_root has been called
    // we attempt to setup stage2-init log to tmpfs
    if pre_pivot_root {
        setup_stage2_init_fallback_log(&s2_config.fallback_log_filename);
    }

    // Notify balena API that takeover failed.
    if s2_config.report_hup_progress {
        match notify_hup_progress(
            &s2_config.api_endpoint,
            &s2_config.api_key,
            &s2_config.uuid,
            "100",
            "OS update failed",
        ) {
            Ok(_) => {
                info!("HUP progress notification OK");
            }
            Err(why) => {
                error!("Failed HUP progress notification, error {}", why);
            }
        }
    }

    if s2_config.fallback_log {
        let _ = persist_fallback_log_to_data_partition(s2_config, false);
    }
    reboot();
}
