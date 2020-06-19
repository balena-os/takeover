use std::env::{current_exe, set_current_dir};
use std::fs::{copy, create_dir, create_dir_all, read_link, remove_dir_all, OpenOptions};
use std::io::Write;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::thread::sleep;
use std::time::Duration;

use nix::{
    mount::{mount, MsFlags},
    unistd::sync,
};

use failure::{Fail, ResultExt};

use libc::MS_BIND;
use log::{debug, error, info, warn, Level};

pub(crate) mod migrate_info;
use migrate_info::MigrateInfo;

pub(crate) mod assets;
use assets::Assets;

mod api_calls;
mod block_device_info;
mod defs;
mod device;
mod device_impl;
mod image_retrieval;
mod utils;
mod wifi_config;

use crate::common::{
    call,
    defs::{
        CP_CMD, OLD_ROOT_MP, STAGE2_CONFIG_NAME, SWAPOFF_CMD, SYSTEM_CONNECTIONS_DIR, TELINIT_CMD,
    },
    file_exists, format_size_with_unit, get_mem_info, is_admin,
    mig_error::{MigErrCtx, MigError, MigErrorKind},
    options::Options,
    path_append,
    stage2_config::Stage2Config,
};

use block_device_info::BlockDeviceInfo;
use mod_logger::{LogDestination, Logger};
use utils::{mktemp, mount_fs};

use crate::common::defs::NIX_NONE;
use crate::common::stage2_config::UmountPart;
use crate::stage1::block_device_info::BlockDevice;

const S1_XTRA_FS_SIZE: u64 = 10 * 1024 * 1024; // const XTRA_MEM_FREE: u64 = 10 * 1024 * 1024; // 10 MB

fn get_required_space(mig_info: &MigrateInfo) -> Result<u64, MigError> {
    let mut req_size: u64 = mig_info.get_assets().busybox_size()?;

    let curr_exe = current_exe().context(upstream_context!(
        "Failed to retrieve path of current executable"
    ))?;
    req_size += curr_exe
        .metadata()
        .context(upstream_context!(&format!(
            "Failed to retrieve file size for '{}'",
            curr_exe.display()
        )))?
        .len();

    Ok(req_size)
}

fn copy_files<P1: AsRef<Path>, P2: AsRef<Path>>(
    work_dir: P1,
    mig_info: &mut MigrateInfo,
    takeover_dir: P2,
) -> Result<(), MigError> {
    let work_dir = work_dir.as_ref();
    let takeover_dir = takeover_dir.as_ref();

    // *********************************************************
    // write busybox executable to tmpfs

    let busybox = mig_info.get_assets().write_to(&takeover_dir)?;

    info!("Copied busybox executable to '{}'", busybox.display());

    // *********************************************************
    // write config.json to tmpfs

    mig_info.update_config()?;

    // *********************************************************
    // write network_manager filess to tmpfs
    let mut nwmgr_cfgs: u64 = 0;
    let nwmgr_path = path_append(&work_dir, SYSTEM_CONNECTIONS_DIR);
    create_dir_all(&nwmgr_path).context(upstream_context!(&format!(
        "Failed to create directory '{}",
        nwmgr_path.display()
    )))?;

    for source_file in mig_info.get_nwmgr_files() {
        nwmgr_cfgs += 1;
        let target_file = path_append(&nwmgr_path, &format!("balena-{:02}", nwmgr_cfgs));
        copy(&source_file, &target_file).context(upstream_context!(&format!(
            "Failed to copy '{}' to '{}'",
            source_file.display(),
            target_file.display()
        )))?;
        info!(
            "Copied '{}' to '{}'",
            source_file.display(),
            target_file.display()
        );
    }

    for wifi_config in mig_info.get_wifis() {
        wifi_config.create_nwmgr_file(&nwmgr_path, nwmgr_cfgs)?;
    }

    let target_path = path_append(takeover_dir, "takeover");
    let curr_exe = current_exe().context(upstream_context!(
        "Failed to retrieve path of current executable"
    ))?;

    copy(&curr_exe, &target_path).context(upstream_context!(&format!(
        "Failed to copy current executable '{}' to '{}",
        curr_exe.display(),
        target_path.display()
    )))?;

    info!("Copied current executable to '{}'", target_path.display());
    Ok(())
}

fn get_umount_parts(
    flash_dev: &Rc<Box<dyn BlockDevice>>,
    block_dev_info: &BlockDeviceInfo,
) -> Result<Vec<UmountPart>, MigError> {
    let mut umount_parts: Vec<UmountPart> = Vec::new();

    for device in block_dev_info.get_devices().values() {
        if let Some(parent) = device.get_parent() {
            // this is a partition rather than a device
            if parent.get_name() == flash_dev.get_name() {
                // it is a partition of the flash device
                if let Some(mount) = device.get_mountpoint() {
                    let mut inserted = false;
                    for (idx, mpoint) in umount_parts.iter().enumerate() {
                        if mpoint.mountpoint.starts_with(mount.get_mountpoint()) {
                            umount_parts.insert(
                                idx,
                                UmountPart {
                                    dev_name: device.get_dev_path().to_path_buf(),
                                    mountpoint: PathBuf::from(mount.get_mountpoint()),
                                    fs_type: mount.get_fs_type().to_string(),
                                },
                            );
                            inserted = true;
                            break;
                        }
                    }
                    if !inserted {
                        umount_parts.push(UmountPart {
                            dev_name: device.get_dev_path().to_path_buf(),
                            mountpoint: PathBuf::from(mount.get_mountpoint()),
                            fs_type: mount.get_fs_type().to_string(),
                        });
                    }
                }
            }
        }
    }
    umount_parts.reverse();
    Ok(umount_parts)
}

fn prepare(opts: &Options, mig_info: &mut MigrateInfo) -> Result<(), MigError> {
    info!("Preparing for takeover..");

    // *********************************************************
    // turn off swap
    if let Ok(cmd_res) = call(SWAPOFF_CMD, &["-a"], true) {
        if cmd_res.status.success() {
            info!("SWAP was disabled successfully");
        } else {
            error!("Failed to disable SWAP, stderr: '{}'", cmd_res.stderr);
            return Err(MigError::displayed());
        }
    }

    // *********************************************************
    // calculate required memory

    let (mem_tot, mem_free) = get_mem_info()?;
    info!(
        "Found {} total, {} free memory",
        format_size_with_unit(mem_tot),
        format_size_with_unit(mem_free)
    );

    let req_space = get_required_space(mig_info)?;

    // TODO: maybe kill some procs first
    if mem_free < req_space + S1_XTRA_FS_SIZE {
        error!(
            "Not enough memory space found to copy files to RAMFS, required size is {} free memory is {}",
            format_size_with_unit(req_space + S1_XTRA_FS_SIZE),
            format_size_with_unit(mem_free)
        );
        return Err(MigError::displayed());
    }

    // *********************************************************
    // make mountpoint for tmpfs

    let takeover_dir = mktemp(true, Some("TO.XXXXXXXX"), Some("/"))?;

    mig_info.set_to_dir(&takeover_dir);

    info!("Created takeover directory in '{}'", takeover_dir.display());

    // *********************************************************
    // mount tmpfs

    mount_fs(&takeover_dir, "tmpfs", "tmpfs", mig_info)?;

    let curr_path = takeover_dir.join("etc");
    create_dir(&curr_path).context(upstream_context!(&format!(
        "Failed to create directory '{}'",
        curr_path.display()
    )))?;

    // *********************************************************
    // initialize essential paths

    let curr_path = curr_path.join("mtab");
    symlink("/proc/mounts", &curr_path).context(upstream_context!(&format!(
        "Failed to create symlink /proc/mounts to '{}'",
        curr_path.display()
    )))?;

    info!("Created mtab in  '{}'", curr_path.display());

    let curr_path = takeover_dir.join("proc");
    mount_fs(curr_path, "proc", "proc", mig_info)?;

    let curr_path = takeover_dir.join("tmp");
    mount_fs(&curr_path, "tmpfs", "tmpfs", mig_info)?;

    let curr_path = takeover_dir.join("sys");
    mount_fs(&curr_path, "sys", "sysfs", mig_info)?;

    let curr_path = takeover_dir.join("dev");
    if mount_fs(&curr_path, "dev", "devtmpfs", mig_info).is_err() {
        warn!("Failed to mount devtmpfs on /dev, trying to copy device nodes");
        mount_fs(&curr_path, "tmpfs", "tmpfs", mig_info)?;

        let cmd_res = call(
            CP_CMD,
            &["-a", "/dev/*", &*curr_path.to_string_lossy()],
            true,
        )?;
        if !cmd_res.status.success() {
            error!(
                "Failed to copy /dev file system to '{}', error : '{}",
                curr_path.display(),
                cmd_res.stderr
            );
            return Err(MigError::displayed());
        }

        let curr_path = takeover_dir.join("dev/pts");
        if curr_path.exists() {
            remove_dir_all(&curr_path).context(upstream_context!(&format!(
                "Failed to delete directory '{}'",
                curr_path.display()
            )))?;
        }
    }

    if (opts.get_log_level() == Level::Debug) || (opts.get_log_level() == Level::Trace) {
        use crate::common::debug::check_loop_control;
        check_loop_control("After dev mount", &curr_path);
    } else {
        debug!("(??)Log Level: {:?}", opts.get_log_level());
    }

    let curr_path = takeover_dir.join("dev/pts");
    mount_fs(&curr_path, "devpts", "devpts", mig_info)?;

    // *********************************************************
    // create mountpoint for old root

    let curr_path = path_append(&takeover_dir, OLD_ROOT_MP);

    create_dir_all(&curr_path).context(upstream_context!(&format!(
        "Failed to create directory '{}'",
        curr_path.display()
    )))?;

    info!("Created directory '{}'", curr_path.display());

    copy_files(opts.get_work_dir(), mig_info, &takeover_dir)?;

    // *********************************************************
    // setup new init

    let tty = read_link("/proc/self/fd/1")
        .context(upstream_context!("Failed to read link for /proc/self/fd/1"))?;

    let old_init_path = read_link("/proc/1/exe")
        .context(upstream_context!("Failed to read link for /proc/1/exe"))?;
    let new_init_path = takeover_dir
        .join("tmp")
        .join(old_init_path.file_name().unwrap());
    Assets::write_stage2_script(&takeover_dir, &new_init_path, &tty, opts.get_s2_log_level())?;

    let block_dev_info = BlockDeviceInfo::new()?;

    let flash_dev = if let Some(flash_dev) = opts.get_flash_to() {
        if let Some(flash_dev) = block_dev_info.get_devices().get(flash_dev) {
            flash_dev
        } else {
            error!(
                "Could not find configured flash device '{}'",
                flash_dev.display()
            );
            return Err(MigError::displayed());
        }
    } else {
        block_dev_info.get_root_device()
    };

    if !file_exists(&flash_dev.as_ref().get_dev_path()) {
        error!(
            "The device could not be found: '{}'",
            flash_dev.get_dev_path().display()
        );
        return Err(MigError::displayed());
    }

    // collect partitions that need to be unmounted

    let s2_cfg = Stage2Config {
        log_dev: opts.get_log_to().clone(),
        flash_dev: flash_dev.get_dev_path().to_path_buf(),
        pretend: opts.is_pretend(),
        umount_parts: get_umount_parts(flash_dev, &block_dev_info)?,
        work_dir: opts
            .get_work_dir()
            .canonicalize()
            .context(upstream_context!(&format!(
                "Failed to canonicalize work dir '{}'",
                opts.get_work_dir().display()
            )))?,
        image_path: mig_info.get_image_path().to_path_buf(),
        config_path: mig_info.get_balena_cfg().get_path().to_path_buf(),
        backup_path: None,
    };

    let s2_cfg_path = takeover_dir.join(STAGE2_CONFIG_NAME);
    let mut s2_cfg_file = OpenOptions::new()
        .create(true)
        .write(true)
        .open(&s2_cfg_path)
        .context(upstream_context!(&format!(
            "Failed to open stage2 config file for writing: '{}'",
            s2_cfg_path.display()
        )))?;

    let s2_cfg_txt = s2_cfg.serialize()?;
    debug!("Stage 2 config: \n{}", s2_cfg_txt);

    s2_cfg_file
        .write(s2_cfg_txt.as_bytes())
        .context(upstream_context!(&format!(
            "Failed to write stage2 config file to '{}'",
            s2_cfg_path.display()
        )))?;

    info!("Wrote stage2 config to '{}'", s2_cfg_path.display());

    set_current_dir(&takeover_dir).context(upstream_context!(&format!(
        "Failed to change current dir to '{}'",
        takeover_dir.display()
    )))?;

    mount(
        Some(&new_init_path),
        &old_init_path,
        NIX_NONE,
        MsFlags::from_bits(MS_BIND).unwrap(),
        NIX_NONE,
    )
    .context(upstream_context!(&format!(
        "Failed to bind-mount '{}' to '{}'",
        new_init_path.display(),
        old_init_path.display()
    )))?;

    info!("Bind-mounted new init as '{}'", new_init_path.display());

    debug!("calling '{} u'", TELINIT_CMD);
    let cmd_res = call(TELINIT_CMD, &["u"], true)?;
    if !cmd_res.status.success() {
        error!("Call to telinit failed, stderr: '{}'", cmd_res.stderr);
        return Err(MigError::displayed());
    }

    info!("Restarted init");

    Ok(())
}

pub fn stage1(opts: &Options) -> Result<(), MigError> {
    Logger::set_default_level(opts.get_log_level());
    Logger::set_brief_info(true);
    Logger::set_color(true);

    if opts.is_build_num() {
        println!("build: {}", Assets::get_build_num()?);
        return Ok(());
    }

    let log_file = PathBuf::from("./stage1.log");
    if let Err(why) = Logger::set_log_file(&LogDestination::StreamStderr, &log_file, true) {
        error!(
            "Failed to set logging to '{}', error: {:?}",
            log_file.display(),
            why
        );
        return Err(MigError::displayed());
    }

    let mut mig_info = match MigrateInfo::new(&opts) {
        Ok(mig_info) => mig_info,
        Err(why) => {
            if why.kind() == MigErrorKind::ImageDownloaded {
                return Ok(());
            } else {
                return Err(from_upstream!(why, "Failed to create migrate info"));
            }
        }
    };

    if !is_admin()? {
        error!("please run this program as root");
        return Err(MigError::from(MigErrorKind::Displayed));
    }

    if !opts.is_no_ack() {
        println!("{} will prepare your device for migration. Are you sure you want to migrate this device: [Y/n]", env!("CARGO_PKG_NAME"));
        loop {
            let mut buffer = String::new();
            match std::io::stdin().read_line(&mut buffer) {
                Ok(_) => match buffer.trim() {
                    "Y" => {
                        break;
                    }
                    "n" => {
                        info!("Terminating on user request");
                        return Err(MigError::displayed());
                    }
                    _ => {
                        println!("please type Y for yes or n for no");
                        continue;
                    }
                },
                Err(why) => return Err(from_upstream!(why, "Failed to read line from stdin")),
            }
        }
    }

    if opts.is_migrate() {
        match prepare(&opts, &mut mig_info) {
            Ok(_) => {
                info!("Takeover initiated successfully, please wait for the device to be reflashed and reboot");
                Logger::flush();
                sync();
                sleep(Duration::from_secs(10));
                Ok(())
            }
            Err(why) => {
                if opts.is_cleanup() {
                    mig_info.umount_all();
                }
                Err(why)
            }
        }
    } else {
        Ok(())
    }
}
