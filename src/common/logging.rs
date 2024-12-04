use log::{error, info};
use std::fs::{self, OpenOptions};
use std::{
    fs::copy,
    path::{Path, PathBuf},
};

use crate::common::defs::BALENA_DATA_MP;
use crate::common::{path_append, Error, ToError};
use crate::{
    common::{
        debug,
        defs::{BALENA_DATA_FSTYPE, NIX_NONE},
        disk_util::DEF_BLOCK_SIZE,
        error::Result,
        loop_device::LoopDevice,
        ErrorKind,
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
    stage2_config,
};

/// Utility function to create a directory and all child directories
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

/// Helper function to copy a source file to a directory
/// it keeps the same file name
pub fn copy_file_to_destination_dir(source_file_path: &str, dest_dir_path: &str) -> Result<()> {
    info!(
        "copy_file_to_destination_dir! Copying {} from tmpfs to {}",
        source_file_path, dest_dir_path
    );

    let source_file = Path::new(source_file_path);
    if source_file.exists() && source_file.is_file() {
        let file_name = source_file
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .expect("Failed to extract file name from path");

        copy(
            PathBuf::from(source_file),
            path_append(dest_dir_path, format!("/{}", file_name)),
        )?;

        Ok(())
    } else {
        Err(Error::with_context(
            ErrorKind::FileNotFound,
            &format!("source file {} does not exist", source_file_path),
        ))
    }
}

/// Helper function to create or open the tmpfs logfile
/// The fallback log mechanism logs to a single file
/// The filename used can be provided as an option
pub fn open_fallback_log_file(fallback_log_filename: &str) -> Option<std::fs::File> {
    let tmpfs_log_file = path_append("/tmp/", fallback_log_filename);
    let log_file = match OpenOptions::new()
        .append(true) // Append to the file, don't overwrite
        .create(true) // Create the file if it does not exist
        .open(tmpfs_log_file)
    {
        Ok(file) => Some(file),
        Err(why) => {
            error!(
                "Could not open /tmp/{}, error {:?}",
                fallback_log_filename, why
            );
            None
        }
    };
    log_file
}

/// Helper function to persist fallback logs from tmpfs to disk
/// This function can be called at different stages during the migration
///
/// # Arguments
/// * `s2_config` - The stage2 config file
/// * `is_new_image_flashed` - indicates if the function is being called after the
/// the target disk has been flashed with the new os image.
///
/// If called before flashing, this is usually as part of error handling
/// and we want to persist the fallback logs
pub fn persist_fallback_log_to_data_partition(
    s2_config: &Stage2Config,
    is_new_image_flashed: bool,
) -> Result<()> {
    let source_tmpfs_log_path = format!("/tmp/{}", s2_config.fallback_log_filename);

    // If true, we need to mount the raw data partition and write to it
    if is_new_image_flashed {
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

        let dest_dir = format!("{}/{}", BALENA_PART_MP, s2_config.fallback_log_dirname);
        create_dir_if_not_exist(&dest_dir)?;

        copy_file_to_destination_dir(&source_tmpfs_log_path, dest_dir.as_str())?;

        sync();
        umount(BALENA_PART_MP).upstream_with_context("Failed to unmount data partition")?;
        info!("Unmounted data partition from {}", BALENA_PART_MP);

        loop_device.unset()?;
    } else if Path::new(BALENA_DATA_MP).exists() {
        let dest_dir = format!("{}/{}", BALENA_DATA_MP, s2_config.fallback_log_dirname);
        create_dir_if_not_exist(&dest_dir)?;

        copy_file_to_destination_dir(&source_tmpfs_log_path, dest_dir.as_str())?;
    } else if Path::new(path_append(OLD_ROOT_MP, BALENA_DATA_MP).as_os_str()).exists() {
        // else if data partition is relative to OLD_ROOT_MP

        let dest_dir = format!(
            "{}/{}/{}",
            OLD_ROOT_MP, BALENA_DATA_MP, s2_config.fallback_log_dirname
        );
        create_dir_if_not_exist(&dest_dir)?;

        copy_file_to_destination_dir(&source_tmpfs_log_path, dest_dir.as_str())?;
    }

    Ok(())
}
