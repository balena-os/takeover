use log::{error, info};
use std::fs;
use std::{
    fs::copy,
    path::{Path, PathBuf},
};

use crate::common::{path_append, ToError};
use crate::{
    common::{
        debug,
        defs::{BALENA_DATA_FSTYPE, NIX_NONE},
        disk_util::DEF_BLOCK_SIZE,
        error::Result,
        loop_device::LoopDevice,
    },
    stage2::get_partition_infos,
};

use nix::{
    mount::{mount, umount, MsFlags},
    unistd::sync,
};

use self::stage2_config::Stage2Config;

use super::{
    defs::{BALENA_PART_MP, OLD_ROOT_MP},
    reboot, stage2_config,
};

pub const LOG_TMPFS_DESTINATION: &str = "/tmp";
pub const LOG_STAGE_1: &str = "stage1.log";
pub const LOG_STAGE_2_INIT: &str = "stage2-init.log";
pub const LOG_STAGE_2: &str = "stage2.log";
pub const LOG_PRE_UNMOUNT_DATA_PART_DEST: &str = "/mnt/data/balenahup/takeover";

pub enum Stage {
    S1,
    S2Init,
    Stage2,
}

fn create_dir_if_not_exist(path: &str) -> Result<()> {
    let dir_path = Path::new(path);
    if !dir_path.is_dir() {
        println!("Directory does not exist. Creating: {}", dir_path.display());
        fs::create_dir_all(dir_path).upstream_with_context(
            format!("Failed to create directory {}", dir_path.display()).as_str(),
        )?;
        println!("Directory created successfully.");
    } else {
        println!("Directory already exists: {}", dir_path.display());
    }

    Ok(())
}

// Helper function to get the path for storing logs in different stages
pub fn get_stage_tmpfs_logfile_path(stage: Stage) -> String {
    match stage {
        Stage::S1 => format!("{}/{}", LOG_TMPFS_DESTINATION, LOG_STAGE_1),
        Stage::S2Init => format!("{}/{}", LOG_TMPFS_DESTINATION, LOG_STAGE_2_INIT),
        Stage::Stage2 => format!("{}/{}", LOG_TMPFS_DESTINATION, LOG_STAGE_2),
    }
}

// Helper function to handle dumping log from tmpfs to data partition
// at different stages in the takeover process
pub fn copy_tmpfs_log_to_data_partition(source_tmp_log_path: &str, dest_dir_path: &str) {
    info!(
        "copy_tmpfs_log_to_data_partition entered! Copying {} from tmpfs to {}",
        source_tmp_log_path, dest_dir_path
    );
    // check if target destination exists, if not create
    match create_dir_if_not_exist(dest_dir_path) {
        Ok(_) => (),
        Err(_) => reboot(),
    }

    let source_tmp_log = Path::new(source_tmp_log_path);
    if source_tmp_log.exists() && source_tmp_log.is_file() {
        let file_name = source_tmp_log
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .expect("Failed to extract file name from path");

        match copy(
            PathBuf::from(source_tmp_log),
            path_append(dest_dir_path, format!("/{}", file_name)),
        ) {
            Ok(_) => info!(
                "Copied {} from {} to {} on data partition",
                file_name,
                &source_tmp_log.display(),
                &dest_dir_path
            ),
            Err(why) => error!(
                "Could not copy {} from {} to {}: {:?}",
                file_name,
                &source_tmp_log.display(),
                &dest_dir_path,
                why
            ),
        }
    } else {
        info!(
            "File {} does not exist or is not a regular file.",
            source_tmp_log_path
        );
    }
}

// Helper function to dump logs
// Caters for state prior to calling pivot_root
pub fn stage2_init_pre_pivot_root_tmpfs_log_handler() {
    let stage1_logfile = get_stage_tmpfs_logfile_path(Stage::S1);
    let stage2_init_logfile = get_stage_tmpfs_logfile_path(Stage::S2Init);

    // copy files to data partition
    copy_tmpfs_log_to_data_partition(stage1_logfile.as_str(), LOG_PRE_UNMOUNT_DATA_PART_DEST);
    copy_tmpfs_log_to_data_partition(stage2_init_logfile.as_str(), LOG_PRE_UNMOUNT_DATA_PART_DEST);
}

// Error handling if an error occurs in stage2-init prior to calling pivot_root
pub fn stage2_init_pre_pivot_root_err_handler(fallback_log: bool) -> ! {
    if fallback_log {
        stage2_init_pre_pivot_root_tmpfs_log_handler();
    }

    reboot();
}

// Helper function to dump logs
// Caters for state after calling pivot_root
pub fn stage2_init_post_pivot_root_tmpfs_log_handler() {
    // if pivot_root called, stage1 log will be relative to old root mountpoint
    let stage1_logfile = format!(
        "{}/{}",
        OLD_ROOT_MP,
        get_stage_tmpfs_logfile_path(Stage::S1)
    );

    let stage2_init_logfile = get_stage_tmpfs_logfile_path(Stage::S2Init);

    // data part will be relative to old mount point
    let destination = format!("{}/{}", OLD_ROOT_MP, LOG_PRE_UNMOUNT_DATA_PART_DEST);

    // copy files to data partition
    copy_tmpfs_log_to_data_partition(stage1_logfile.as_str(), destination.as_str());
    copy_tmpfs_log_to_data_partition(stage2_init_logfile.as_str(), destination.as_str());
}

// Error handling if an error occurs in stage2-init after calling pivot_root
pub fn stage2_init_post_pivot_root_err_handler() -> ! {
    stage2_init_post_pivot_root_tmpfs_log_handler();
    reboot();
}

// Helper function to dump logs
// Caters for state in stage2 worker prior to unmounting partitions
pub fn stage2_pre_unmount_tmpfs_log_handler() {
    let stage1_logfile = format!(
        "{}/{}",
        OLD_ROOT_MP,
        get_stage_tmpfs_logfile_path(Stage::S1)
    );
    let stage2_init_logfile = get_stage_tmpfs_logfile_path(Stage::S2Init);
    let stage2_logfile = get_stage_tmpfs_logfile_path(Stage::Stage2);

    // copy files to data partition
    let destination = format!("{}/{}", OLD_ROOT_MP, LOG_PRE_UNMOUNT_DATA_PART_DEST);

    copy_tmpfs_log_to_data_partition(stage1_logfile.as_str(), destination.as_str());
    copy_tmpfs_log_to_data_partition(stage2_init_logfile.as_str(), destination.as_str());
    copy_tmpfs_log_to_data_partition(stage2_logfile.as_str(), destination.as_str());
}
// Error handling if an error occurs in stage2 worker process before unmounting partitions
pub fn stage2_pre_unmount_err_handler(fallback_log: bool) -> ! {
    if fallback_log {
        stage2_pre_unmount_tmpfs_log_handler();
    }

    reboot();
}

// Helper function to dump logs
// Caters for state in stage2 worker after unmounting partitions
pub fn stage2_post_unmount_tmpfs_log_handler(s2_config: &Stage2Config) -> Result<()> {
    let stage1_logfile = format!(
        "{}/{}",
        OLD_ROOT_MP,
        get_stage_tmpfs_logfile_path(Stage::S1)
    );
    let stage2_init_logfile = get_stage_tmpfs_logfile_path(Stage::S2Init);
    let stage2_logfile = get_stage_tmpfs_logfile_path(Stage::Stage2);

    // destination will be relative to partition mountpoint
    let destination = format!("{}/{}", BALENA_PART_MP, "/balenahup/takeover");

    // Mount raw data partition
    let device = &s2_config.flash_dev;

    let (_boot_part, _root_a_part, data_part) = get_partition_infos(device)?;

    let mut loop_device = LoopDevice::get_free(true)?;
    info!("Create loop device: '{}'", loop_device.get_path().display());
    let byte_offset = data_part.start_lba * DEF_BLOCK_SIZE as u64;
    let size_limit = data_part.num_sectors * DEF_BLOCK_SIZE as u64;

    debug!(
        "Setting up device '{}' with offset {}, sizelimit {} on '{}'",
        device.display(),
        byte_offset,
        size_limit,
        loop_device.get_path().display()
    );

    loop_device
        .setup(device, Some(byte_offset), Some(size_limit))
        .unwrap();
    info!(
        "Setup device '{}' with offset {}, sizelimit {} on '{}'",
        device.display(),
        byte_offset,
        size_limit,
        loop_device.get_path().display()
    );

    mount(
        Some(loop_device.get_path()),
        BALENA_PART_MP,
        Some(BALENA_DATA_FSTYPE.as_bytes()),
        MsFlags::empty(),
        NIX_NONE,
    )
    .upstream_with_context(&format!(
        "Failed to mount '{}' to '{}'",
        loop_device.get_path().display(),
        BALENA_PART_MP,
    ))?;

    info!(
        "Mounted data partition as {} on {}",
        loop_device.get_path().display(),
        BALENA_PART_MP
    );

    // copy files to data partition
    copy_tmpfs_log_to_data_partition(stage1_logfile.as_str(), destination.as_str());
    copy_tmpfs_log_to_data_partition(stage2_init_logfile.as_str(), destination.as_str());
    copy_tmpfs_log_to_data_partition(stage2_logfile.as_str(), destination.as_str());

    sync();
    umount(BALENA_PART_MP).unwrap();
    info!("Unmounted data partition from {}", BALENA_PART_MP);

    loop_device.unset()?;
    Ok(())
}

// Error handling if an error occurs in stage2 worker process after unmounting partitions
pub fn stage2_post_unmount_err_handler(s2_config: &Stage2Config) -> ! {
    // if --log-to-balenaos was not passed, we simply reboot
    if s2_config.fallback_log {
        match stage2_post_unmount_tmpfs_log_handler(s2_config) {
            Ok(_) => (),
            Err(_) => reboot(),
        }
    }
    reboot();
}
