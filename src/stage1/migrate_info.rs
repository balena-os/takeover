use log::{debug, error, info, warn};
use nix::mount::umount;
use std::fs::{read_to_string, remove_dir_all, OpenOptions};
use std::path::{Path, PathBuf};
use std::ptr::read_volatile;

use crate::common::defs::BACKUP_ARCH_NAME;
use crate::common::path_append;
use crate::{
    common::{file_exists, get_os_name, options::Options, Error, ErrorKind, Result, ToError},
    stage1::{
        backup::config::backup_cfg_from_file,
        backup::{create, create_ext},
        defs::{DEV_TYPE_GEN_X86_64, GZIP_MAGIC_COOKIE, MAX_CONFIG_JSON, BOOT_BLOB_PARTITION_JETSON_XAVIER, DEV_TYPE_JETSON_XAVIER},
        device::Device,
        device_impl::get_device,
        image_retrieval::{download_image, FLASHER_DEVICES},
        migrate_info::balena_cfg_json::BalenaCfgJson,
        utils::mktemp,
        wifi_config::WifiConfig,
    },
};

use crate::stage1::utils::ReadBuffer;
use flate2::read::GzDecoder;
use std::io::copy;

#[link_section = ".config_json_section"]
static CONFIG_JSON: [u8; MAX_CONFIG_JSON] = [0; MAX_CONFIG_JSON];

pub(crate) mod balena_cfg_json;

#[derive(Debug)]
pub(crate) struct MigrateInfo {
    os_name: String,
    // assets: Assets,
    mounts: Vec<PathBuf>,
    to_dir: Option<PathBuf>,
    image_path: PathBuf,
    boot0_image_path: PathBuf, /* boot blob for Jetson AGX Xavier */
    boot0_image_dev: PathBuf, /* HW defined boot partition on AGX Xavier */
    device: Box<dyn Device>,
    config: BalenaCfgJson,
    work_dir: PathBuf,
    wifis: Vec<WifiConfig>,
    nwmgr_files: Vec<PathBuf>,
    backup: Option<PathBuf>,
}

#[allow(dead_code)]
impl MigrateInfo {
    pub fn new(opts: &Options) -> Result<MigrateInfo> {
        let device = get_device(opts)?;
        let os_name = get_os_name()?;
        info!("Detected device type: {} running {}", device.get_device_type(), os_name);

        /* If no config.json is passed in command line and we're running on balenaOS,
         * we can preserve the existing config.json
         */
        let mut config = if let Some(balena_cfg) = opts.config() {
            BalenaCfgJson::new(balena_cfg)?
        } else if Path::new("/mnt/boot/config.json").exists() {
            BalenaCfgJson::new("/mnt/boot/config.json")?
        }
        else {
            match MigrateInfo::get_internal_cfg_json(&opts.work_dir()) {
                Ok(balena_cfg_json) => balena_cfg_json,
                Err(why) => {
                    if why.kind() == ErrorKind::NotFound {
                        error!("The required parameter --config/-c was not provided and no internal config.json was found");
                        return Err(Error::displayed());
                    } else {
                        return Err(why);
                    }
                }
            }
        };

        if opts.migrate() {
            config.check(opts, &*device)?;
        }

        info!(
            "config.json is for device type {}",
            config.get_device_type()?
        );

        let work_dir = opts
            .work_dir()
            .canonicalize()
            .upstream_with_context(&format!(
                "Failed to canonicalize path to work_dir: '{}'",
                opts.work_dir().display()
            ))?;

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

        /* We could not to extract the boot blob from the non-flasher
         * image, so, for the purpose of testing migration on the AGX Xavier
         * we added a config flag to pass the path to the boot blob
         * TODO: Extract the boot blob from /opt/<boot.img>
         */
        let mut boot0_image_path = PathBuf::new();
        let mut boot0_image_dev = PathBuf::new();
        if device.supports_device_type(DEV_TYPE_JETSON_XAVIER) {
             boot0_image_path = opts
            .boot0_image()
            .canonicalize()
            .upstream_with_context(&format!(
                "Failed to canonicalize path to boot0 image: '{}'",
                opts.boot0_image().display()
            ))?;

            if file_exists(&boot0_image_path) {
                info!("boot0 image found!");
            }

            boot0_image_dev = PathBuf::from(BOOT_BLOB_PARTITION_JETSON_XAVIER);
        }

        /* TODO: Check if we will convert the Jetson AGX, Jetson Xavier NX eMMC and NX SD to flasher types */
        if FLASHER_DEVICES.contains(&config.get_device_type()?.as_str()) {
            info!("device-type '{}' is a flasher type, should unpack image", &config.get_device_type()?.as_str());
            // below function needs to be implemented if we decide to release flasher images for the new L4T, to extract the flasher image from the non-flasher image
            //extract_image_from_local(&image_path ...)?;
        }

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

        let backup = if let Some(backup_cfg) = opts.backup_config() {
            let backup_path = path_append(&work_dir, BACKUP_ARCH_NAME);
            let created = if opts.tar_internal() {
                create(backup_path.as_path(), backup_cfg_from_file(backup_cfg)?)?
            } else {
                create_ext(backup_path.as_path(), backup_cfg_from_file(backup_cfg)?)?
            };
            if created {
                Some(backup_path)
            } else {
                None
            }
        } else {
            None
        };

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
            boot0_image_path,
            boot0_image_dev,
            device,
            work_dir,
            wifis,
            nwmgr_files,
            backup,
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

    pub fn set_to_dir(&mut self, to_dir: &PathBuf) {
        self.to_dir = Some(to_dir.clone())
    }

    pub fn to_dir(&self) -> &Option<PathBuf> {
        &self.to_dir
    }

    pub fn get_device_type_name(&self) -> String {
         self.device.to_string()
    }

    pub fn is_x86(&self) -> bool {
        self.device.supports_device_type(DEV_TYPE_GEN_X86_64)
    }

    pub fn is_jetson_xavier(&self) -> bool {
        self.device.supports_device_type(DEV_TYPE_JETSON_XAVIER)
    }

    pub fn backup(&self) -> Option<&Path> {
        if let Some(backup) = &self.backup {
            Some(backup.as_path())
        } else {
            None
        }
    }

    pub(crate) fn os_name(&self) -> &str {
        self.os_name.as_ref()
    }
    pub fn image_path(&self) -> &Path {
        self.image_path.as_path()
    }

    pub fn boot0_image_path(&self) -> &Path {
        self.boot0_image_path.as_path()
    }

    pub fn boot0_image_dev(&self) -> &Path {
        self.boot0_image_dev.as_path()
    }

    pub fn balena_cfg(&self) -> &BalenaCfgJson {
        &self.config
    }

    pub fn add_mount<P: AsRef<Path>>(&mut self, mount: P) {
        self.mounts.push(mount.as_ref().to_path_buf())
    }

    pub fn mounts(&self) -> &Vec<PathBuf> {
        &self.mounts
    }

    pub fn nwmgr_files(&self) -> &Vec<PathBuf> {
        &self.nwmgr_files
    }

    pub fn add_nwmgr_file<P: AsRef<Path>>(&mut self, nwmgr_file_path: P) {
        self.nwmgr_files.push(nwmgr_file_path.as_ref().to_path_buf());
        debug!("Adding network connection file to copy list: {}", nwmgr_file_path.as_ref().to_path_buf().display());
    }

    pub fn wifis(&self) -> &Vec<WifiConfig> {
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

    fn get_internal_cfg_json(work_dir: &Path) -> Result<BalenaCfgJson> {
        const SIZE_LEN: usize = std::mem::size_of::<u32>();
        const COOKIE_LEN: usize = std::mem::size_of::<u16>();

        let byte_ptr = &CONFIG_JSON as *const u8;
        // use of read_volatile makes sure CONFIG_JSON is not removed from ELF image
        let mut size_buf: [u8; SIZE_LEN] = [0; SIZE_LEN];
        for (idx, dest) in size_buf.iter_mut().enumerate() {
            *dest = unsafe { read_volatile(byte_ptr.add(idx)) };
        }
        let size = u32::from_ne_bytes(size_buf) as usize;

        let mut cookie_buf: [u8; COOKIE_LEN] = [0; COOKIE_LEN];
        for (idx, dest) in cookie_buf.iter_mut().enumerate() {
            *dest = unsafe { read_volatile(byte_ptr.add(idx + SIZE_LEN)) };
        }
        let cookie = u16::from_be_bytes(cookie_buf);

        debug!(
            "Internal config_json size: {}, cookie: 0x{:04x}",
            size, cookie
        );

        if size == 0 {
            Err(Error::new(ErrorKind::NotFound))
        } else if size < CONFIG_JSON.len() - SIZE_LEN {
            let target_path = mktemp(false, Some("config."), Some(".json"), Some(work_dir))?;

            {
                let mut file = OpenOptions::new()
                    .append(true)
                    .write(true)
                    .open(&target_path)
                    .upstream_with_context(&format!(
                        "Failed to open config.json for writing: '{}",
                        target_path.display()
                    ))?;

                let mut read_buffer = ReadBuffer::new(&CONFIG_JSON[SIZE_LEN..size + SIZE_LEN]);

                if cookie == GZIP_MAGIC_COOKIE {
                    debug!(
                        "get_internal_cfg_json: decompressing internal config.json to '{}'",
                        target_path.display()
                    );
                    let mut decoder = GzDecoder::new(read_buffer);
                    copy(&mut decoder, &mut file).upstream_with_context(&format!(
                        "Failed to uncompress/write config.json to: '{}",
                        target_path.display()
                    ))?;
                } else {
                    debug!(
                        "get_internal_cfg_json: writing internal config.json to '{}'",
                        target_path.display()
                    );
                    copy(&mut read_buffer, &mut file).upstream_with_context(&format!(
                        "Failed to write config.json to: '{}",
                        target_path.display()
                    ))?;
                }
            }

            Ok(BalenaCfgJson::new(&target_path)?)
        } else {
            Err(Error::with_context(
                ErrorKind::InvParam,
                &format!(
                    "Invalid size found for internal config.json: {} > {}",
                    size,
                    CONFIG_JSON.len() - 4
                ),
            ))
        }
    }
}
