use std::fs::remove_dir_all;
use std::path::{Path, PathBuf};

use log::{error, info, warn};
use mod_logger::Level;
use nix::mount::umount;

use crate::{
    common::{file_exists, get_os_name, mig_error::MigError, options::Options},
    stage1::{
        assets::Assets, device::Device, device_impl::get_device, image_retrieval::download_image,
        migrate_info::balena_cfg_json::BalenaCfgJson,
    },
};

pub(crate) mod balena_cfg_json;

#[derive(Debug)]
pub(crate) struct MigrateInfo {
    os_name: String,
    assets: Assets,
    mounts: Vec<PathBuf>,
    to_dir: Option<PathBuf>,
    log_level: Level,
    image_path: PathBuf,
    device: Box<dyn Device>,
    config: BalenaCfgJson,
    work_dir: PathBuf,
}

#[allow(dead_code)]
impl MigrateInfo {
    pub fn new(opts: &Options) -> Result<MigrateInfo, MigError> {
        let device = get_device(opts)?;
        info!("Detected device type: {}", device.get_device_type());

        let config = if let Some(balena_cfg) = opts.get_config() {
            BalenaCfgJson::new(balena_cfg)?
        } else {
            error!("The required parameter --config/-c was not provided");
            return Err(MigError::displayed());
        };
        config.check(opts, &*device)?;

        info!(
            "config.json is for device type {}",
            config.get_device_type()?
        );

        let work_dir = opts.get_work_dir();

        let image_path = if let Some(image_path) = opts.get_image() {
            if file_exists(&image_path) {
                image_path.clone()
            } else {
                error!(
                    "The balena-os image configured as '{}' could not be found",
                    image_path.display()
                );
                return Err(MigError::displayed());
            }
        } else {
            download_image(
                &config,
                &work_dir,
                config.get_device_type()?.as_str(),
                opts.get_version(),
            )?
        };

        Ok(MigrateInfo {
            assets: Assets::new(),
            os_name: get_os_name()?,
            to_dir: None,
            mounts: Vec::new(),
            log_level: if opts.is_trace() {
                Level::Trace
            } else if opts.is_debug() {
                Level::Debug
            } else {
                Level::Info
            },
            config,
            image_path,
            device,
            work_dir,
        })
    }

    pub fn get_assets(&self) -> &Assets {
        &self.assets
    }

    pub fn set_to_dir(&mut self, to_dir: &PathBuf) {
        self.to_dir = Some(to_dir.clone())
    }

    pub fn get_log_level(&self) -> Level {
        self.log_level
    }

    pub fn get_to_dir(&self) -> &Option<PathBuf> {
        &self.to_dir
    }

    pub fn get_image_path(&self) -> &Path {
        self.image_path.as_path()
    }

    pub fn get_balena_cfg(&self) -> &BalenaCfgJson {
        &self.config
    }

    pub fn add_mount<P: AsRef<Path>>(&mut self, mount: P) {
        self.mounts.push(mount.as_ref().to_path_buf())
    }

    pub fn get_mounts(&self) -> &Vec<PathBuf> {
        &self.mounts
    }

    pub fn umount_all(&mut self) {
        loop {
            if let Some(mountpoint) = self.mounts.pop() {
                if let Err(why) = umount(&mountpoint) {
                    warn!(
                        "Failed to unmount mountpoint: '{}', error : {:?}",
                        mountpoint.display(),
                        why
                    );
                }
            } else {
                break;
            }
        }

        if let Some(takeover_dir) = &self.to_dir {
            if let Err(why) = remove_dir_all(takeover_dir) {
                warn!(
                    "Failed to remove takeover directory: '{}', error : {:?}",
                    takeover_dir.display(),
                    why
                );
            }
        }
    }
}
