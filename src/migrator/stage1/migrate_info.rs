use std::fs::{read_to_string, remove_dir_all};
use std::path::{Path, PathBuf};

use log::{debug, error, info, warn};
use nix::mount::umount;

use crate::stage1::defs::DEV_TYPE_GEN_X86_64;
use crate::stage1::utils::mktemp;
use crate::{
    common::{file_exists, get_os_name, options::Options, Error, ErrorKind, Result, ToError},
    stage1::{
        device::Device, device_impl::get_device, image_retrieval::download_image,
        migrate_info::balena_cfg_json::BalenaCfgJson, wifi_config::WifiConfig,
    },
};

pub(crate) mod balena_cfg_json;

#[derive(Debug)]
pub(crate) struct MigrateInfo {
    os_name: String,
    // assets: Assets,
    mounts: Vec<PathBuf>,
    to_dir: Option<PathBuf>,
    image_path: PathBuf,
    device: Box<dyn Device>,
    config: BalenaCfgJson,
    work_dir: PathBuf,
    wifis: Vec<WifiConfig>,
    nwmgr_files: Vec<PathBuf>,
}

#[allow(dead_code)]
impl MigrateInfo {
    pub fn new(opts: &Options) -> Result<MigrateInfo> {
        let device = get_device(opts)?;
        info!("Detected device type: {}", device.get_device_type());

        let mut config = if let Some(balena_cfg) = opts.config() {
            BalenaCfgJson::new(balena_cfg)?
        } else {
            error!("The required parameter --config/-c was not provided");
            return Err(Error::displayed());
        };

        if opts.migrate() {
            config.check(opts, &*device)?;
        }

        info!(
            "config.json is for device type {}",
            config.get_device_type()?
        );

        let work_dir = opts.work_dir();

        let image_path = if let Some(image_path) = opts.image() {
            if file_exists(&image_path) {
                image_path.canonicalize().upstream_with_context(&format!(
                    "Failed to canonicalize path '{}'",
                    image_path.display()
                ))?
            } else {
                error!(
                    "The balena-os image configured as '{}' could not be found",
                    image_path.display()
                );
                return Err(Error::displayed());
            }
        } else {
            let image_path = download_image(
                &config,
                &work_dir,
                config.get_device_type()?.as_str(),
                opts.version(),
            )?;
            image_path.canonicalize().upstream_with_context(&format!(
                "Failed to canonicalize path '{}'",
                image_path.display()
            ))?
        };

        if !opts.migrate() {
            return Err(Error::with_context(
                ErrorKind::ImageDownloaded,
                "Image downloaded successfully",
            ));
        }

        debug!("image path: '{}'", image_path.display());

        let wifi_ssids = opts.wifis();

        let wifis: Vec<WifiConfig> = if !wifi_ssids.is_empty() || !opts.no_wifis() {
            WifiConfig::scan(wifi_ssids)?
        } else {
            Vec::new()
        };

        let nwmgr_files = Vec::from(opts.nwmgr_cfg());

        if nwmgr_files.is_empty() && wifis.is_empty() {
            if opts.no_nwmgr_check() {
                warn!(
                    "No Network manager files were found, the device might not be able to come online"
                );
            } else {
                error!(
                    "No Network manager files were found, the device might not be able to come online"
                );
                return Err(Error::displayed());
            }
        }

        if opts.migrate_name() {
            let hostname = read_to_string("/proc/sys/kernel/hostname")
                .upstream_with_context("Failed to read file '/proc/sys/kernel/hostname'")?
                .trim()
                .to_string();

            info!("Writing hostname to config.json: '{}'", hostname);
            config.set_host_name(&hostname);
        }

        Ok(MigrateInfo {
            // assets: Assets::new(),
            os_name: get_os_name()?,
            to_dir: None,
            mounts: Vec::new(),
            config,
            image_path,
            device,
            work_dir,
            wifis,
            nwmgr_files,
        })
    }

    pub fn update_config(&mut self) -> Result<()> {
        if self.config.is_modified() {
            let target_path = mktemp(false, Some("config."), Some(".json"), Some(&self.work_dir))?;
            self.config.write(&target_path)?;
            info!("Copied config.json to '{}'", target_path.display());
        }
        Ok(())
    }

    pub fn is_x86(&self) -> bool {
        self.device.supports_device_type(DEV_TYPE_GEN_X86_64)
    }

    /*    pub fn get_assets(&self) -> &Assets {
            &self.assets
        }
    */
    pub fn set_to_dir(&mut self, to_dir: &PathBuf) {
        self.to_dir = Some(to_dir.clone())
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

    pub fn get_nwmgr_files(&self) -> &Vec<PathBuf> {
        &self.nwmgr_files
    }

    pub fn get_wifis(&self) -> &Vec<WifiConfig> {
        &self.wifis
    }

    pub fn umount_all(&mut self) {
        while let Some(mountpoint) = self.mounts.pop() {
            if let Err(why) = umount(&mountpoint) {
                warn!(
                    "Failed to unmount mountpoint: '{}', error : {:?}",
                    mountpoint.display(),
                    why
                );
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
