use std::path::{Path, PathBuf};

use clap::Parser;
use log::Level;

const DEFAULT_CHECK_TIMEOUT: u64 = 10;

#[derive(Parser, Debug, Clone)]
#[clap(name = env!("CARGO_PKG_NAME"), author, about)]
pub struct Options {
    /// what to do
    #[clap(
        short,
        long,
        value_name = "DIRECTORY",
        value_parser,
        help = "Path to working directory"
    )]
    work_dir: Option<PathBuf>,
    #[clap(
        short,
        long,
        value_name = "IMAGE",
        value_parser,
        help = "Path to balena-os image"
    )]
    image: Option<PathBuf>,
    #[clap(
        short,
        long,
        value_name = "VERSION",
        help = "Version of balena-os image to download"
    )]
    version: Option<String>,
    #[clap(
        short,
        long,
        value_name = "CONFIG_JSON",
        value_parser,
        help = "Path to balena config.json"
    )]
    config: Option<PathBuf>,
    #[clap(
        long,
        default_value = "info",
        help = "Set log level, one of [error,warn,info,debug,trace]"
    )]
    log_level: Level,
    #[clap(
        long,
        value_name = "LOG_FILE",
        value_parser,
        help = "Set stage1 log file name"
    )]
    log_file: Option<PathBuf>,
    #[clap(
        long,
        value_name = "BACKUP-CONFIG",
        value_parser,
        help = "Backup configuration file"
    )]
    backup_cfg: Option<PathBuf>,
    #[clap(
        long,
        help = "Set stage2 log level, one of [error,warn,info,debug,trace]"
    )]
    s2_log_level: Option<Level>,
    #[clap(
        long,
        help = "Scripted mode - no interactive acknowledgement of takeover"
    )]
    no_ack: bool,
    #[clap(long, help = "Pretend mode, do not flash device")]
    pretend: bool,
    #[clap(long, help = "Internal - stage2 invocation")]
    stage2: bool,
    #[clap(long, help = "Use internal tar instead of external command")]
    tar_internal: bool,
    #[clap(long, help = "Debug - do not cleanup after stage1 failure")]
    no_cleanup: bool,
    #[clap(long, help = "Do not check if OS is supported")]
    no_os_check: bool,
    #[clap(long, help = "Do not check if the target device type is valid")]
    no_dt_check: bool,
    #[clap(long, help = "Do not check if balena API is available")]
    no_api_check: bool,
    #[clap(long, help = "Do not check if balena VPN is available")]
    no_vpn_check: bool,
    #[clap(long, help = "Do not setup EFI boot")]
    no_efi_setup: bool,
    #[clap(long, help = "Do not check network manager files exist")]
    no_nwmgr_check: bool,
    #[clap(long, help = "Do not migrate host-name")]
    no_keep_name: bool,
    #[clap(
        short,
        long,
        help = "Download image only, do not check device and migrate"
    )]
    download_only: bool,
    #[clap(
        long,
        value_name = "TIMEOUT",
        value_parser,
        help = "API/VPN check timeout in seconds."
    )]
    check_timeout: Option<u64>,
    #[clap(
        long,
        short,
        value_name = "LOG_DEVICE",
        value_parser,
        help = "Write stage2 log to LOG_DEVICE"
    )]
    log_to: Option<PathBuf>,
    #[clap(
        short,
        long,
        value_name = "INSTALL_DEVICE",
        value_parser,
        help = "Use INSTALL_DEVICE to flash balena to"
    )]
    flash_to: Option<PathBuf>,
    #[clap(
        long,
        help = "Do not create network manager configurations for configured wifis"
    )]
    no_wifis: bool,
    #[clap(
        long,
        value_name = "SSID",
        help = "Create a network manager configuration for configured wifi with SSID"
    )]
    wifi: Option<Vec<String>>,
    #[clap(
        long,
        value_name = "NWMGR_FILE",
        value_parser,
        help = "Supply a network manager file to inject into balena-os"
    )]
    nwmgr_cfg: Option<Vec<PathBuf>>,
    #[clap(long, value_name = "DT_SLUG", help = "Device Type slug to change to")]
    change_dt_to: Option<String>,
}

impl Options {
    pub fn backup_config(&self) -> Option<&Path> {
        if let Some(backup_cfg) = &self.backup_cfg {
            Some(backup_cfg.as_path())
        } else {
            None
        }
    }

    pub fn stage2(&self) -> bool {
        self.stage2
    }

    pub fn tar_internal(&self) -> bool {
        self.tar_internal
    }

    pub fn work_dir(&self) -> PathBuf {
        if let Some(work_dir) = &self.work_dir {
            work_dir.clone()
        } else {
            PathBuf::from("./")
        }
    }

    pub fn image(&self) -> &Option<PathBuf> {
        &self.image
    }

    pub fn version(&self) -> &str {
        if let Some(ref version) = self.version {
            version.as_str()
        } else {
            "default"
        }
    }

    pub fn no_ack(&self) -> bool {
        self.no_ack
    }

    pub fn migrate(&self) -> bool {
        !self.download_only
    }

    pub fn config(&self) -> &Option<PathBuf> {
        &self.config
    }

    pub fn pretend(&self) -> bool {
        self.pretend
    }

    pub fn log_file(&self) -> &Option<PathBuf> {
        &self.log_file
    }

    pub fn log_level(&self) -> Level {
        self.log_level
    }

    pub fn s2_log_level(&self) -> Level {
        if let Some(level) = self.s2_log_level {
            level
        } else {
            self.log_level
        }
    }

    pub fn os_check(&self) -> bool {
        !self.no_os_check
    }

    pub fn dt_check(&self) -> bool {
        !self.no_dt_check
    }

    pub fn no_efi_setup(&self) -> bool {
        self.no_efi_setup
    }

    pub fn api_check(&self) -> bool {
        !self.no_api_check
    }

    pub fn vpn_check(&self) -> bool {
        !self.no_vpn_check
    }

    pub fn log_to(&self) -> &Option<PathBuf> {
        &self.log_to
    }

    pub fn flash_to(&self) -> &Option<PathBuf> {
        &self.flash_to
    }

    pub fn check_timeout(&self) -> u64 {
        if let Some(timeout) = self.check_timeout {
            timeout
        } else {
            DEFAULT_CHECK_TIMEOUT
        }
    }

    pub fn no_wifis(&self) -> bool {
        self.no_wifis
    }

    pub fn wifis(&self) -> &[String] {
        const NO_WIFIS: [String; 0] = [];
        if let Some(wifis) = &self.wifi {
            wifis.as_slice()
        } else {
            &NO_WIFIS
        }
    }

    pub fn nwmgr_cfg(&self) -> &[PathBuf] {
        if let Some(nwmgr_cfgs) = &self.nwmgr_cfg {
            nwmgr_cfgs.as_slice()
        } else {
            const NO_NWMGR_CFGS: [PathBuf; 0] = [];
            &NO_NWMGR_CFGS
        }
    }

    pub fn no_nwmgr_check(&self) -> bool {
        self.no_nwmgr_check
    }

    pub fn migrate_name(&self) -> bool {
        !self.no_keep_name
    }

    pub fn cleanup(&self) -> bool {
        !self.no_cleanup
    }

    pub fn change_dt_to(&self) -> &Option<String> {
        &self.change_dt_to
    }
}
