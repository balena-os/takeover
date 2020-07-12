mod backup;

use std::env::set_current_dir;
use std::fs::{copy, create_dir, create_dir_all, read_dir, read_link, remove_dir_all, OpenOptions};
use std::io::Write;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::thread::sleep;
use std::time::Duration;

use structopt::StructOpt;

use nix::{
    mount::{mount, MsFlags},
    unistd::sync,
};

use libc::MS_BIND;

use log::{debug, error, info, warn, Level};

pub(crate) mod migrate_info;

mod api_calls;
mod block_device_info;
mod defs;
mod device;
mod device_impl;

mod exe_copy;

mod image_retrieval;
mod utils;
mod wifi_config;

use crate::{
    common::{
        call,
        defs::{
            NIX_NONE, OLD_ROOT_MP, STAGE2_CONFIG_NAME, SWAPOFF_CMD, SYSTEM_CONNECTIONS_DIR,
            SYS_EFIVARS_DIR, SYS_EFI_DIR, TELINIT_CMD,
        },
        error::{Error, ErrorKind, Result, ToError},
        file_exists, format_size_with_unit, get_mem_info, is_admin,
        options::Options,
        path_append,
        stage2_config::{Stage2Config, UmountPart},
        system::copy_dir,
    },
    stage1::{
        block_device_info::BlockDevice, block_device_info::BlockDeviceInfo, exe_copy::ExeCopy,
        migrate_info::MigrateInfo, utils::mount_fs,
    },
};

use crate::common::defs::{DD_CMD, EFIBOOTMGR_CMD, TAKEOVER_DIR};
use crate::common::dir_exists;
use crate::common::stage2_config::LogDevice;
use crate::common::system::{is_dir, mkdir, stat};
use mod_logger::{LogDestination, Logger, NO_STREAM};

const S1_XTRA_FS_SIZE: u64 = 10 * 1024 * 1024; // const XTRA_MEM_FREE: u64 = 10 * 1024 * 1024; // 10 MB

fn prepare_configs<P1: AsRef<Path>>(
    work_dir: P1,
    mig_info: &mut MigrateInfo,
    // takeover_dir: P2,
) -> Result<()> {
    let work_dir = work_dir.as_ref();
    // let takeover_dir = takeover_dir.as_ref();

    // *********************************************************
    // write busybox executable to tmpfs

    //let busybox = mig_info.get_assets().write_to(&takeover_dir)?;

    // info!("Copied busybox executable to '{}'", busybox.display());

    // *********************************************************
    // write config.json to tmpfs

    mig_info.update_config()?;

    // *********************************************************
    // write network_manager filess to tmpfs
    let mut nwmgr_cfgs: u64 = 0;
    let nwmgr_path = path_append(&work_dir, SYSTEM_CONNECTIONS_DIR);
    create_dir_all(&nwmgr_path).upstream_with_context(&format!(
        "Failed to create directory '{}",
        nwmgr_path.display()
    ))?;

    for source_file in mig_info.nwmgr_files() {
        nwmgr_cfgs += 1;
        let target_file = path_append(&nwmgr_path, &format!("balena-{:02}", nwmgr_cfgs));
        copy(&source_file, &target_file).upstream_with_context(&format!(
            "Failed to copy '{}' to '{}'",
            source_file.display(),
            target_file.display()
        ))?;
        info!(
            "Copied '{}' to '{}'",
            source_file.display(),
            target_file.display()
        );
    }

    for wifi_config in mig_info.wifis() {
        wifi_config.create_nwmgr_file(&nwmgr_path, nwmgr_cfgs)?;
    }

    Ok(())
}

fn get_umount_parts(
    flash_dev: &Rc<dyn BlockDevice>,
    block_dev_info: &BlockDeviceInfo,
) -> Result<Vec<UmountPart>> {
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

fn mount_sys_filesystems(
    takeover_dir: &Path,
    mig_info: &mut MigrateInfo,
    opts: &Options,
) -> Result<()> {
    // *********************************************************
    // mount tmpfs

    mount_fs(&takeover_dir, "tmpfs", "tmpfs", None)?;

    let curr_path = takeover_dir.join("etc");
    create_dir(&curr_path).upstream_with_context(&format!(
        "Failed to create directory '{}'",
        curr_path.display()
    ))?;

    // *********************************************************
    // initialize essential paths

    let curr_path = curr_path.join("mtab");
    symlink("/proc/mounts", &curr_path).upstream_with_context(&format!(
        "Failed to create symlink /proc/mounts to '{}'",
        curr_path.display()
    ))?;

    info!("Created mtab in  '{}'", curr_path.display());

    let curr_path = takeover_dir.join("proc");
    mount_fs(curr_path, "proc", "proc", Some(mig_info))?;

    let curr_path = takeover_dir.join("tmp");
    mount_fs(&curr_path, "tmpfs", "tmpfs", Some(mig_info))?;

    let curr_path = takeover_dir.join("sys");
    mount_fs(&curr_path, "sys", "sysfs", Some(mig_info))?;

    if dir_exists(SYS_EFIVARS_DIR)? {
        let curr_path = path_append(&takeover_dir, SYS_EFIVARS_DIR);
        create_dir_all(&curr_path)?;
        mount_fs(&curr_path, "efivarfs", "efivarfs", Some(mig_info))?;
        // TODO: copy stuff ?
    }

    let curr_path = takeover_dir.join("dev");
    if mount_fs(&curr_path, "dev", "devtmpfs", Some(mig_info)).is_err() {
        warn!("Failed to mount devtmpfs on /dev, trying to copy device nodes");
        mount_fs(&curr_path, "tmpfs", "tmpfs", Some(mig_info))?;

        copy_dir("/dev", &curr_path)?;

        let curr_path = takeover_dir.join("dev/pts");
        if curr_path.exists() {
            remove_dir_all(&curr_path).upstream_with_context(&format!(
                "Failed to delete directory '{}'",
                curr_path.display()
            ))?;
        }
    }

    if (opts.log_level() == Level::Debug) || (opts.log_level() == Level::Trace) {
        use crate::common::debug::check_loop_control;
        check_loop_control("After dev mount", &curr_path);
    } else {
        debug!("(??)Log Level: {:?}", opts.log_level());
    }

    let curr_path = takeover_dir.join("dev/pts");
    mount_fs(&curr_path, "devpts", "devpts", Some(mig_info))?;

    Ok(())
}

fn prepare(opts: &Options, mig_info: &mut MigrateInfo) -> Result<()> {
    info!("Preparing for takeover..");

    // *********************************************************
    // turn off swap
    call_command!(SWAPOFF_CMD, &["-a"], "Failed to disable SWAP")?;

    // *********************************************************
    // calculate required memory

    let mut req_space: u64 = 0;
    let mut copy_commands = vec![DD_CMD];
    if mig_info.is_x86() && !opts.no_efi_setup() && dir_exists(SYS_EFI_DIR)? {
        copy_commands.push(EFIBOOTMGR_CMD)
    }

    let commands = match ExeCopy::new(copy_commands) {
        Ok(commands) => {
            let cmd_space = commands.get_req_space();
            debug!(
                "Space required for commands: {}",
                format_size_with_unit(cmd_space)
            );
            req_space += cmd_space;
            commands
        }
        Err(why) => {
            return Err(Error::from_upstream_error(
                Box::new(why),
                "Failed to gather dependencies for copied commands",
            ));
        }
    };

    let (mem_tot, mem_free) = get_mem_info()?;
    info!(
        "Found {} total, {} free memory",
        format_size_with_unit(mem_tot),
        format_size_with_unit(mem_free)
    );

    // TODO: maybe kill some procs first
    if mem_free < req_space + S1_XTRA_FS_SIZE {
        return Err(Error::with_context(ErrorKind::InvState, &format!(
            "Not enough memory space found to copy files to RAMFS, required size is {} free memory is {}",
            format_size_with_unit(req_space + S1_XTRA_FS_SIZE),
            format_size_with_unit(mem_free)
        )));
    }

    // *********************************************************
    // make mountpoint for tmpfs
    let takeover_dir = PathBuf::from(TAKEOVER_DIR);
    match stat(&takeover_dir) {
        Ok(stat) => {
            if is_dir(&stat) {
                let read_dir = read_dir(&takeover_dir).upstream_with_context(&format!(
                    "Failed to read directory '{}'",
                    takeover_dir.display()
                ))?;
                if read_dir.count() > 0 {
                    error!(
                        "Found a non-empty directory '{}' - please remove or rename this directory",
                        takeover_dir.display()
                    );
                    return Err(Error::displayed());
                } else {
                    warn!(
                        "Directory '{}' exists. Reusing directory",
                        takeover_dir.display()
                    );
                }
            } else {
                error!(
                    "Found a file '{}' - please remove or rename this file",
                    takeover_dir.display()
                );
                return Err(Error::displayed());
            }
        }
        Err(why) => {
            if why.kind() == ErrorKind::FileNotFound {
                mkdir(&takeover_dir, 0o755)?;
            } else {
                return Err(Error::from_upstream(
                    Box::new(why),
                    &format!("Failed to stat '{}'", takeover_dir.display()),
                ));
            }
        }
    }

    mig_info.set_to_dir(&takeover_dir);

    info!("Using '{}' as takeover directory", takeover_dir.display());

    mount_sys_filesystems(&takeover_dir, mig_info, opts)?;

    // *********************************************************
    // create mountpoint for old root

    let curr_path = path_append(&takeover_dir, OLD_ROOT_MP);

    create_dir_all(&curr_path).upstream_with_context(&format!(
        "Failed to create directory '{}'",
        curr_path.display()
    ))?;

    info!("Created directory '{}'", curr_path.display());

    commands.copy_files(&takeover_dir)?;

    prepare_configs(opts.work_dir(), mig_info)?;

    // *********************************************************
    // setup new init

    let old_init_path =
        read_link("/proc/1/exe").upstream_with_context("Failed to read link for /proc/1/exe")?;

    // TODO: make new_init_path point to /$takeover_dir/bin/takeover directly
    let new_init_path = path_append(&takeover_dir, &format!("/bin/{}", env!("CARGO_PKG_NAME")));
    // Assets::write_stage2_script(&takeover_dir, &new_init_path, &tty, opts.get_s2_log_level())?;

    let block_dev_info = BlockDeviceInfo::new()?;

    let flash_dev = if let Some(flash_dev) = opts.flash_to() {
        if let Some(flash_dev) = block_dev_info.get_devices().get(flash_dev) {
            flash_dev
        } else {
            return Err(Error::with_context(
                ErrorKind::InvState,
                &format!(
                    "Could not find configured flash device '{}'",
                    flash_dev.display()
                ),
            ));
        }
    } else {
        block_dev_info.get_root_device()
    };

    if !file_exists(&flash_dev.as_ref().get_dev_path()) {
        return Err(Error::with_context(
            ErrorKind::DeviceNotFound,
            &format!(
                "The device could not be found: '{}'",
                flash_dev.get_dev_path().display()
            ),
        ));
    }

    let log_device = if let Some(log_dev_path) = opts.log_to() {
        if let Some(log_dev) = block_dev_info.get_devices().get(log_dev_path) {
            if let Some(partition_info) = log_dev.get_partition_info() {
                if let Some(fs_type) = partition_info.fs_type() {
                    const SUPPORTED_LOG_FS_TYPES: [&str; 3] = ["vfat", "ext3", "ext4"];
                    if SUPPORTED_LOG_FS_TYPES.iter().any(|val| *val == fs_type) {
                        Some(LogDevice {
                            dev_name: log_dev_path.clone(),
                            fs_type: fs_type.to_owned(),
                        })
                    } else {
                        warn!("The log device's ('{}') files system type '{}' is not in the list of supported file systems: {:?}. Your device will not be able to write stage2 logs",
                              log_dev_path.display(),
                                fs_type,
                            SUPPORTED_LOG_FS_TYPES);
                        None
                    }
                } else {
                    warn!("We could not detect the filesystemm type for the log device '{}'. Your device will not be able to write stage2 logs",
                          log_dev_path.display());
                    None
                }
            } else {
                warn!("The log device '{}' is not a partition. Your device will not be able to write stage2 logs",
                      log_dev_path.display());
                None
            }
        } else {
            warn!("The log device '{}' could not be found. Your device will not be able to write stage2 logs",
                  log_dev_path.display());
            None
        }
    } else {
        None
    };

    // collect partitions that need to be unmounted

    let s2_cfg = Stage2Config {
        log_dev: log_device,
        log_level: opts.s2_log_level().to_string(),
        flash_dev: flash_dev.get_dev_path(),
        pretend: opts.pretend(),
        umount_parts: get_umount_parts(flash_dev, &block_dev_info)?,
        work_dir: opts
            .work_dir()
            .canonicalize()
            .upstream_with_context(&format!(
                "Failed to canonicalize work dir '{}'",
                opts.work_dir().display()
            ))?,
        image_path: mig_info.image_path().to_path_buf(),
        config_path: mig_info.balena_cfg().get_path().to_path_buf(),
        backup_path: if let Some(backup_path) = mig_info.backup() {
            Some(backup_path.to_owned())
        } else {
            None
        },
        tty: read_link("/proc/self/fd/1")
            .upstream_with_context("Failed to read tty from '/proc/self/fd/1'")?,
    };

    let s2_cfg_path = takeover_dir.join(STAGE2_CONFIG_NAME);
    let mut s2_cfg_file = OpenOptions::new()
        .create(true)
        .write(true)
        .open(&s2_cfg_path)
        .upstream_with_context(&format!(
            "Failed to open stage2 config file for writing: '{}'",
            s2_cfg_path.display()
        ))?;

    let s2_cfg_txt = s2_cfg.serialize()?;
    debug!("Stage 2 config: \n{}", s2_cfg_txt);

    s2_cfg_file
        .write(s2_cfg_txt.as_bytes())
        .upstream_with_context(&format!(
            "Failed to write stage2 config file to '{}'",
            s2_cfg_path.display()
        ))?;

    info!("Wrote stage2 config to '{}'", s2_cfg_path.display());

    set_current_dir(&takeover_dir).upstream_with_context(&format!(
        "Failed to change current dir to '{}'",
        takeover_dir.display()
    ))?;

    mount(
        Some(&new_init_path),
        &old_init_path,
        NIX_NONE,
        MsFlags::from_bits(MS_BIND).unwrap(),
        NIX_NONE,
    )
    .upstream_with_context(&format!(
        "Failed to bind-mount '{}' to '{}'",
        new_init_path.display(),
        old_init_path.display()
    ))?;

    info!("Bind-mounted new init as '{}'", new_init_path.display());

    //return Ok(());

    debug!("calling '{} u'", TELINIT_CMD,);
    call_command!(
        TELINIT_CMD,
        &["u"],
        &format!("Call to {} failed", TELINIT_CMD)
    )?;

    info!("Restarted init");

    Ok(())
}

pub fn stage1(opts: &Options) -> Result<()> {
    Logger::set_default_level(opts.log_level());
    Logger::set_brief_info(true);
    Logger::set_color(true);

    if opts.config().is_none() {
        let mut clap = Options::clap();
        let _res = clap.print_help();
        return Err(Error::displayed());
    }

    if let Some(s1_log_path) = opts.log_file() {
        Logger::set_log_file(&LogDestination::StreamStderr, &s1_log_path, true)
            .upstream_with_context(&format!(
                "Failed to set logging to '{}'",
                s1_log_path.display(),
            ))?;
    } else {
        Logger::set_log_dest(&LogDestination::Stderr, NO_STREAM)
            .upstream_with_context("Failed to set up logging")?;
    }

    let mut mig_info = match MigrateInfo::new(&opts) {
        Ok(mig_info) => mig_info,
        Err(why) => {
            if why.kind() == ErrorKind::ImageDownloaded {
                return Ok(());
            } else {
                return Err(Error::from_upstream(
                    Box::new(why),
                    "Failed to create migrate info",
                ));
            }
        }
    };

    if !is_admin()? {
        error!("please run this program as root");
        return Err(Error::displayed());
    }

    if !opts.no_ack() {
        println!("{} will prepare your device for migration. Are you sure you want to migrate this device: [Y/n]", env!("CARGO_PKG_NAME"));
        loop {
            let mut buffer = String::new();
            match std::io::stdin().read_line(&mut buffer) {
                Ok(_) => match buffer.trim() {
                    "Y" | "y" => {
                        break;
                    }
                    "n" => {
                        info!("Terminating on user request");
                        return Err(Error::displayed());
                    }
                    _ => {
                        println!("please type Y for yes or n for no");
                        continue;
                    }
                },
                Err(why) => {
                    return Err(Error::from_upstream(
                        Box::new(why),
                        "Failed to read line from stdin",
                    ))
                }
            }
        }
    }

    if opts.migrate() {
        match prepare(&opts, &mut mig_info) {
            Ok(_) => {
                info!("Takeover initiated successfully, please wait for the device to be reflashed and reboot");
                Logger::flush();
                sync();
                sleep(Duration::from_secs(10));
                Ok(())
            }
            Err(why) => {
                if opts.cleanup() {
                    mig_info.umount_all();
                }
                Err(why)
            }
        }
    } else {
        Ok(())
    }
}
