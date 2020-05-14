use std::env::{current_exe, set_current_dir};
use std::fs::{copy, create_dir, create_dir_all, read_link, remove_dir_all, OpenOptions};
use std::os::unix::fs::symlink;
use std::path::PathBuf;
use std::thread::sleep;
use std::time::Duration;

use nix::{
    mount::{mount, MsFlags},
    unistd::sync,
};

use failure::ResultExt;
use log::{debug, error, info};

pub(crate) mod lsblk_info;
use lsblk_info::LsblkInfo;

pub(crate) mod migrate_info;
use migrate_info::MigrateInfo;

pub(crate) mod assets;
use assets::Assets;

use crate::common::defs::NIX_NONE;
use crate::common::defs::{BALENA_CONFIG_PATH, BALENA_IMAGE_NAME, OLD_ROOT_MP, STAGE2_CONFIG_NAME};

use crate::common::{
    call,
    defs::{CP_CMD, MKTEMP_CMD, MOUNT_CMD, SWAPOFF_CMD, TELINIT_CMD},
    format_size_with_unit, get_mem_info, is_admin,
    mig_error::{MigErrCtx, MigError, MigErrorKind},
    options::Options,
    stage2_config::Stage2Config,
};
use crate::common::{file_exists, path_append};
use mod_logger::Logger;
use std::io::Write;

const XTRA_FS_SIZE: u64 = 10 * 1024 * 1024; // 10 MB
                                            // const XTRA_MEM_FREE: u64 = 10 * 1024 * 1024; // 10 MB

fn prepare(opts: &Options, mig_info: &mut MigrateInfo) -> Result<(), MigError> {
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

    let bla = vecco![1, 2, 3];

    // *********************************************************
    // calculate required memory
    let mut req_size: u64 = mig_info.get_assets().busybox_size() as u64 + XTRA_FS_SIZE;

    let image_path = if let Some(image_path) = opts.get_image() {
        if image_path.exists() {
            req_size += image_path
                .metadata()
                .context(upstream_context!(&format!(
                    "Failed to retrieve imagesize for '{}'",
                    image_path.display()
                )))?
                .len() as u64;
            image_path
        } else {
            error!("Image could not be found: '{}'", image_path.display());
            return Err(MigError::displayed());
        }
    } else {
        error!("Required parameter image is missing.");
        return Err(MigError::displayed());
    };

    let config_path = if let Some(config_path) = opts.get_config().clone() {
        if file_exists(&config_path) {
            req_size += config_path
                .metadata()
                .context(upstream_context!(&format!(
                    "Failed to retrieve imagesize for '{}'",
                    config_path.display()
                )))?
                .len() as u64;
            config_path
        } else {
            error!("Config could not be found: '{}'", config_path.display());
            return Err(MigError::displayed());
        }
    } else {
        error!("The required parameter --config/-c was not provided");
        return Err(MigError::displayed());
    };

    let (mem_tot, mem_free) = get_mem_info()?;
    info!(
        "Found {} total, {} free memory",
        format_size_with_unit(mem_tot),
        format_size_with_unit(mem_free)
    );

    // TODO: check memory, abort if not enough

    // *********************************************************
    // make mountpoint for tmpfs

    let cmd_res = call(MKTEMP_CMD, &["-d", "-p", "/", "TO.XXXXXXXX"], true)?;
    let takeover_dir = if cmd_res.status.success() {
        PathBuf::from(cmd_res.stdout)
    } else {
        return Err(MigError::from_remark(
            MigErrorKind::CmdIO,
            &format!(
                "Failed to create temporary directory, stderr: '{}'",
                cmd_res.stderr
            ),
        ));
    };

    mig_info.set_to_dir(&takeover_dir);

    info!("Created takeover directory in '{}'", takeover_dir.display());

    // *********************************************************
    // mount tmpfs

    mount(
        Some("tmpfs".as_bytes()),
        &takeover_dir,
        Some("tmpfs".as_bytes()),
        MsFlags::empty(),
        NIX_NONE,
    )
    .context(upstream_context!(&format!(
        "Failed to mount tmpfs on {} with fstype tmpfs",
        &takeover_dir.display()
    )))?;

    mig_info.add_mount(&takeover_dir);

    let curr_path = takeover_dir.join("etc");
    create_dir(&curr_path).context(upstream_context!(&format!(
        "Failed to create directory '{}'",
        curr_path.display()
    )))?;
    info!("Mounted tmpfs on '{}'", takeover_dir.display());

    // *********************************************************
    // initialize essential paths

    let curr_path = curr_path.join("mtab");
    symlink("/proc/mounts", &curr_path).context(upstream_context!(&format!(
        "Failed to create symlink /proc/mounts to '{}'",
        curr_path.display()
    )))?;

    info!("Created mtab in  '{}'", curr_path.display());

    let curr_path = takeover_dir.join("proc");
    create_dir(&curr_path).context(upstream_context!(&format!(
        "Failed to create directory '{}'",
        curr_path.display()
    )))?;

    mount(
        Some("proc".as_bytes()),
        &curr_path,
        Some("proc".as_bytes()),
        MsFlags::empty(),
        NIX_NONE,
    )
    .context(upstream_context!(&format!(
        "Failed to mount proc on {} with fstype proc",
        &curr_path.display()
    )))?;

    mig_info.add_mount(&curr_path);

    info!("Mounted proc file system on '{}'", curr_path.display());

    let curr_path = takeover_dir.join("tmp");
    create_dir(&curr_path).context(upstream_context!(&format!(
        "Failed to create directory '{}'",
        curr_path.display()
    )))?;

    mount(
        Some("tmpfs".as_bytes()),
        &curr_path,
        Some("tmpfs".as_bytes()),
        MsFlags::empty(),
        NIX_NONE,
    )
    .context(upstream_context!(&format!(
        "Failed to mount tmpfs on {} with fstype tmpfs",
        &curr_path.display()
    )))?;

    mig_info.add_mount(&curr_path);

    info!("Mounted tmpfs  on '{}'", curr_path.display());

    let curr_path = takeover_dir.join("sys");
    create_dir(&curr_path).context(upstream_context!(&format!(
        "Failed to create directory '{}'",
        curr_path.display()
    )))?;

    mount(
        Some("sys".as_bytes()),
        &curr_path,
        Some("sysfs".as_bytes()),
        MsFlags::empty(),
        NIX_NONE,
    )
    .context(upstream_context!(&format!(
        "Failed to mount sys on {} with fstype sysfs",
        &curr_path.display()
    )))?;

    mig_info.add_mount(&curr_path);

    info!("Mounted sysfs  on '{}'", curr_path.display());

    let curr_path = takeover_dir.join("dev");
    create_dir(&curr_path).context(upstream_context!(&format!(
        "Failed to create directory '{}'",
        curr_path.display()
    )))?;

    match mount(
        Some("dev".as_bytes()),
        &curr_path,
        Some("devtmpfs".as_bytes()),
        MsFlags::empty(),
        NIX_NONE,
    ) {
        Ok(_) => {
            mig_info.add_mount(&curr_path);
        }
        Err(_why) => {
            mount(
                Some("tmpfs".as_bytes()),
                &curr_path,
                Some("tmpfs".as_bytes()),
                MsFlags::empty(),
                NIX_NONE,
            )
            .context(upstream_context!(&format!(
                "Failed to mount tmpfs on {} with fstype tmpfs",
                &curr_path.display()
            )))?;
            mig_info.add_mount(&curr_path);

            let cmd_res = call(
                CP_CMD,
                &["-a", "/dev/*", &*curr_path.to_string_lossy()],
                true,
            )?;
            if !cmd_res.status.success() {
                error!(
                    "Failed to copy /dev file systemto '{}', error : '{}",
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
    }
    let curr_path = takeover_dir.join("dev/pts");
    if !curr_path.exists() {
        create_dir(&curr_path).context(upstream_context!(&format!(
            "Failed to create directory '{}'",
            curr_path.display()
        )))?;
    }

    mount(
        Some("devpts".as_bytes()),
        &curr_path,
        Some("devpts".as_bytes()),
        MsFlags::empty(),
        NIX_NONE,
    )
    .context(upstream_context!(&format!(
        "Failed to mount devpts on {} with fstype devpts",
        &curr_path.display()
    )))?;
    mig_info.add_mount(&curr_path);

    info!("Mounted dev file system on '{}'", curr_path.display());

    // *********************************************************
    // create mountpoint for old root

    let curr_path = path_append(&takeover_dir, OLD_ROOT_MP);

    create_dir_all(&curr_path).context(upstream_context!(&format!(
        "Failed to create directory '{}'",
        curr_path.display()
    )))?;

    info!("Created directory '{}'", curr_path.display());

    // *********************************************************
    // write busybox executable to tmpfs

    let busybox = mig_info.get_assets().write_to(&takeover_dir)?;

    info!("Copied busybox executable to '{}'", busybox.display());

    // *********************************************************
    // write balena image to tmpfs

    let to_image_path = takeover_dir.join(BALENA_IMAGE_NAME);
    copy(image_path, &to_image_path).context(upstream_context!(&format!(
        "Failed to copy '{}' to {}",
        image_path.display(),
        &to_image_path.display()
    )))?;
    info!("Copied image to '{}'", to_image_path.display());

    // *********************************************************
    // write config.json to tmpfs

    let to_cfg_path = path_append(&takeover_dir, BALENA_CONFIG_PATH);
    copy(&config_path, &to_cfg_path).context(upstream_context!(&format!(
        "Failed to copy '{}' to {}",
        config_path.display(),
        &to_cfg_path.display()
    )))?;

    // *********************************************************
    // write this executable to tmpfs

    let target_path = takeover_dir.join("takeover");
    let curr_exe = current_exe().context(upstream_context!(
        "Failed to retrieve path of current executable"
    ))?;

    copy(&curr_exe, &target_path).context(upstream_context!(&format!(
        "Failed to copy current executable '{}' to '{}",
        curr_exe.display(),
        target_path.display()
    )))?;

    info!("Copied current executable to '{}'", target_path.display());

    // *********************************************************
    // setup new init

    let tty = read_link("/proc/self/fd/1")
        .context(upstream_context!("Failed to read link for /proc/self/fd/1"))?;

    let old_init_path = read_link("/proc/1/exe")
        .context(upstream_context!("Failed to read link for /proc/1/exe"))?;
    let new_init_path = takeover_dir
        .join("tmp")
        .join(old_init_path.file_name().unwrap());
    Assets::write_stage2_script(&takeover_dir, &new_init_path, &tty)?;

    let lsblk_info = LsblkInfo::all()?;

    let flash_dev = if let Some(flash_dev) = opts.get_flash_to() {
        if let Some(device) = lsblk_info
            .get_blk_devices()
            .iter()
            .find(|device| -> bool { device.get_path() == *flash_dev })
        {
            device
        } else {
            error!("Flash device could not be found: '{}'", flash_dev.display());
            return Err(MigError::displayed());
        }
    } else {
        let (flash_dev, _root_part) = lsblk_info.get_path_devs("/")?;
        flash_dev
    };

    // collect partitions that need to be unmounted
    let mut umount_parts: Vec<PathBuf> = Vec::new();
    if let Some(partitions) = &flash_dev.children {
        for partition in partitions {
            if let Some(mountpoint) = &partition.mountpoint {
                let mut inserted = false;
                for (idx, mpoint) in umount_parts.iter().enumerate() {
                    if mpoint.starts_with(mountpoint) {
                        umount_parts.insert(idx, mountpoint.clone());
                        inserted = true;
                        break;
                    }
                }
                if !inserted {
                    umount_parts.push(mountpoint.clone());
                }
            }
        }
        umount_parts.reverse();
    }

    let s2_cfg = Stage2Config {
        log_dev: opts.get_log_to().clone(),
        log_level: mig_info.get_log_level().to_string(),
        flash_dev: flash_dev.get_path(),
        pretend: opts.is_pretend(),
        umount_parts,
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

    let cmd_res = call(
        MOUNT_CMD,
        &[
            "--bind",
            &*new_init_path.to_string_lossy(),
            &*old_init_path.to_string_lossy(),
        ],
        true,
    )?;
    if !cmd_res.status.success() {
        error!(
            "Failed to bindmount new init over old init, stder: '{}'",
            cmd_res.stderr
        );
        return Err(MigError::displayed());
    }

    info!("Bind-mounted new init as '{}'", new_init_path.display());

    let cmd_res = call(TELINIT_CMD, &["u"], true)?;
    if !cmd_res.status.success() {
        error!("Call to telinit failed, stderr: '{}'", cmd_res.stderr);
        return Err(MigError::displayed());
    }

    info!("Restarted init");

    Ok(())
}

pub fn stage1(opts: Options) -> Result<(), MigError> {
    if !is_admin()? {
        error!("please run this program as root");
        return Err(MigError::from(MigErrorKind::Displayed));
    }
    let mut mig_info = MigrateInfo::new(&opts)?;

    match prepare(&opts, &mut mig_info) {
        Ok(_) => {
            info!("Takeover initiated successfully");
            Logger::flush();
            sync();
            sleep(Duration::from_secs(10));
            Ok(())
        }
        Err(why) => {
            mig_info.umount_all();
            return Err(why);
        }
    }
}
