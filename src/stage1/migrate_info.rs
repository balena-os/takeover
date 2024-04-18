use file_diff::diff_files;
use log::{debug, error, info, warn};
use nix::mount::umount;
use std::fs::{read_dir, read_to_string, remove_dir_all, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::ptr::read_volatile;

use crate::common::defs::{
    BACKUP_ARCH_NAME, BALENA_NETWORK_MANAGER_BIND_MOUNT, BALENA_OS_BOOT_MP, BALENA_OS_NAME,
    BALENA_SYSTEM_CONNECTIONS_BOOT_PATH, BALENA_SYSTEM_PROXY_BOOT_PATH, SYSTEM_CONNECTIONS_DIR,
};
use crate::common::path_append;
use crate::{
    common::{file_exists, get_os_name, options::Options, Error, ErrorKind, Result, ToError},
    stage1::{
        backup::config::backup_cfg_from_file,
        backup::{create, create_ext},
        defs::{
            DEV_TYPE_GEN_X86_64, DEV_TYPE_JETSON_XAVIER, DEV_TYPE_JETSON_XAVIER_NX,
            GZIP_MAGIC_COOKIE, MAX_CONFIG_JSON,
        },
        device::Device,
        device_impl::get_device,
        image_retrieval::download_image,
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

#[allow(dead_code)]
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
    system_proxy_files: Vec<PathBuf>,
    backup: Option<PathBuf>,
}

#[allow(dead_code)]
impl MigrateInfo {
    pub fn new(opts: &Options) -> Result<MigrateInfo> {
        let device = get_device(opts)?;
        let os_name = get_os_name()?;
        info!(
            "Detected device type: {} running {}",
            device.get_device_type(),
            os_name
        );

        // If no config.json is passed in command line and we're running on balenaOS,
        // we can preserve the existing config.json
        let mut config = if let Some(balena_cfg) = opts.config() {
            BalenaCfgJson::new(balena_cfg)?
        } else if get_os_name()?.starts_with(BALENA_OS_NAME) {
            BalenaCfgJson::new("/mnt/boot/config.json")?
        } else {
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
            if file_exists(image_path) {
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
                opts,
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

        let mut nwmgr_files = Vec::from(opts.nwmgr_cfg());

        // Migration of system connections has some special handling when
        // migrating from balenaOS.
        if os_name.starts_with(BALENA_OS_NAME) {
            // Check if the files in /etc/NetworkManager/system-connections also exist in /mnt/boot/system-connections
            // and if they have the same contents
            let mut compare_result = compare_files(
                PathBuf::from(BALENA_NETWORK_MANAGER_BIND_MOUNT).join(SYSTEM_CONNECTIONS_DIR),
                PathBuf::from(BALENA_OS_BOOT_MP).join(SYSTEM_CONNECTIONS_DIR),
            );

            match compare_result {
                Ok(()) => {
                    info!(
                    "OK: Bind-mounted path connection files match the ones in the boot partition."
                );
                }
                Err(why) => {
                    return Err(Error::from_upstream_error(
                    Box::new(why),
                    "Bind-mounted path connection files don't match the ones in the boot partition.",
                ));
                }
            }

            // Check if the files in /mnt/boot/system-connections also exist in /etc/NetworkManager/system-connections
            // and if they have the same contents
            compare_result = compare_files(
                PathBuf::from(BALENA_OS_BOOT_MP).join(SYSTEM_CONNECTIONS_DIR),
                PathBuf::from(BALENA_NETWORK_MANAGER_BIND_MOUNT).join(SYSTEM_CONNECTIONS_DIR),
            );

            match compare_result {
                Ok(()) => {
                    info!(
                    "OK: Boot partition connection files match the ones in the bind-mounted path."
                );
                }
                Err(why) => {
                    return Err(Error::from_upstream_error(
                    Box::new(why),
                    "Boot partition connection files don't match the ones in the bind-mounted path.",
                ));
                }
            }

            // If migrating from balenaOS, copy all files from /mnt/boot/system-connections
            if os_name.starts_with(BALENA_OS_NAME) {
                debug!("migrating from balenaOS - marking system-connections files for copying");
                let nwmgr_dir_entries = read_dir(BALENA_SYSTEM_CONNECTIONS_BOOT_PATH)
                    .upstream_with_context(&format!(
                        "Getting NetworkManager connections from '{}'",
                        BALENA_SYSTEM_CONNECTIONS_BOOT_PATH
                    ))?;
                for path in nwmgr_dir_entries {
                    nwmgr_files.push(path?.path());
                }
            }
        }

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

        let mut system_proxy_files = Vec::new();

        // If migrating from balenaOS, copy all files from /mnt/boot/system-proxy
        if os_name.starts_with(BALENA_OS_NAME) {
            debug!("migrating from balenaOS - marking system-proxy files for copying");
            let system_proxy_dir_entries = read_dir(BALENA_SYSTEM_PROXY_BOOT_PATH)
                .upstream_with_context(&format!(
                    "Getting system proxy connections from '{}'",
                    BALENA_SYSTEM_PROXY_BOOT_PATH
                ))?;

            for sys_proxy_path in system_proxy_dir_entries {
                system_proxy_files.push(sys_proxy_path?.path());
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
            device,
            work_dir,
            wifis,
            nwmgr_files,
            system_proxy_files,
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

    pub fn set_to_dir(&mut self, to_dir: &Path) {
        self.to_dir = Some(to_dir.to_path_buf())
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

    pub fn is_jetson_xavier_nx(&self) -> bool {
        self.device.supports_device_type(DEV_TYPE_JETSON_XAVIER_NX)
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

    pub fn system_proxy_files(&self) -> &Vec<PathBuf> {
        &self.system_proxy_files
    }

    // Adds NetworkManager files to the list of connection files to be transferred to the new OS
    pub fn add_nwmgr_file<P: AsRef<Path>>(&mut self, nwmgr_file_path: P) {
        self.nwmgr_files
            .push(nwmgr_file_path.as_ref().to_path_buf());
        debug!(
            "Adding network connection file to copy list: {}",
            nwmgr_file_path.as_ref().to_path_buf().display()
        );
    }

    // Adds redsocks proxy configuration files to the list of system-proxy files to be transferred to the new OS
    // This is useful in the context of balenaOS to balenaOS migration
    pub fn add_system_proxy_file<P: AsRef<Path>>(&mut self, system_proxy_file_path: P) {
        self.system_proxy_files
            .push(system_proxy_file_path.as_ref().to_path_buf());
        debug!(
            "Adding system proxy configuration file to copy list: {}",
            system_proxy_file_path.as_ref().to_path_buf().display()
        );
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

// Compares the contents of the files in dir1
// to the contents of the files that have the same name in dir2.
fn compare_files(dir1: PathBuf, dir2: PathBuf) -> Result<()> {
    let files_dir1 = read_dir(dir1).unwrap();

    for dir1_entry in files_dir1 {
        let file = dir1_entry?;
        let metadata = file.metadata()?;

        // skip any directories or symlinks
        if !metadata.is_file() {
            continue;
        }

        let dir2_file_path = Path::new(&dir2).join(Path::new(
            file.path().file_name().unwrap().to_str().as_ref().unwrap(),
        ));

        let mut dir2_file = match File::open(dir2_file_path.clone()) {
            Ok(f) => f,
            Err(e) => {
                return Err(Error::with_context(
                    ErrorKind::InvState,
                    &format!(
                        "Failed to open {} for comparison - {}",
                        dir2_file_path.as_path().display(),
                        e
                    ),
                ));
            }
        };

        let mut dir1_file = match File::open(file.path()) {
            Ok(f) => f,
            Err(e) => {
                return Err(Error::with_context(
                    ErrorKind::InvState,
                    &format!(
                        "Failed to open {} for comparison - {}",
                        file.path().display(),
                        e
                    ),
                ));
            }
        };

        if diff_files(&mut dir2_file, &mut dir1_file) {
            debug!(
                "Files {} and {} have matching contents",
                dir2_file_path.as_path().display(),
                file.path().display()
            );
        } else {
            error!(
                "Files {} and {} don't have matching contents!",
                dir2_file_path.as_path().display(),
                file.path().display()
            );
            return Err(Error::with_context(
                ErrorKind::InvState,
                "Connection files contents mistmatch detected. Aborting.",
            ));
        }
    }

    // No mismatch found, or directory is empty
    Ok(())
}
