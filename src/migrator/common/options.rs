use std::path::PathBuf;

use log::Level;
use structopt::StructOpt;

const DEFAULT_CHECK_TIMEOUT: u64 = 10;

#[derive(StructOpt, Debug, Clone)]
#[structopt(name = env!("CARGO_PKG_NAME"), author, about)]
pub struct Options {
    /// what to do
    #[structopt(
        short,
        long,
        value_name = "DIRECTORY",
        parse(from_os_str),
        help = "Path to working directory"
    )]
    work_dir: Option<PathBuf>,
    #[structopt(
        short,
        long,
        value_name = "IMAGE",
        parse(from_os_str),
        help = "Path to balena-os image"
    )]
    image: Option<PathBuf>,
    #[structopt(
        short,
        long,
        value_name = "VERSION",
        help = "Version of balena-os image to download"
    )]
    version: Option<String>,
    #[structopt(
        short,
        long,
        value_name = "CONFIG_JSON",
        parse(from_os_str),
        help = "Path to balena config.json"
    )]
    config: Option<PathBuf>,
    #[structopt(
        long,
        default_value = "info",
        help = "Set log level, one of [error,warn,info,debug,trace]"
    )]
    log_level: Level,
    #[structopt(
        long,
        value_name = "LOG_FILE",
        parse(from_os_str),
        help = "Set stage1 log file name"
    )]
    log_file: Option<PathBuf>,
    #[structopt(
        long,
        help = "Set stage2 log level, one of [error,warn,info,debug,trace]"
    )]
    s2_log_level: Option<Level>,
    #[structopt(
        long,
        help = "Scripted mode - no interactive acknoledgement of takeover"
    )]
    no_ack: bool,
    #[structopt(long, help = "Pretend mode, do not flash device")]
    pretend: bool,
    #[structopt(long, help = "Internal - stage2 invocation")]
    stage2: bool,
    #[structopt(long, help = "Internal - init process invocation")]
    init: bool,
    #[structopt(long, help = "Debug - do not cleanup after stage1 failure")]
    no_cleanup: bool,
    #[structopt(long, help = "Do not check if OS is supported")]
    no_os_check: bool,
    #[structopt(long, help = "Do not check if balena API is available")]
    no_api_check: bool,
    #[structopt(long, help = "Do not check if balena VPN is available")]
    no_vpn_check: bool,
    #[structopt(long, help = "Do not fail if EFI setup fails")]
    no_fail_on_efi: bool,
    #[structopt(long, help = "Do not check network manager files exist")]
    no_nwmgr_check: bool,
    #[structopt(long, help = "Do not migrate host-name")]
    no_keep_name: bool,
    #[structopt(long, help = "Debug - print build num and exit")]
    build_num: bool,
    #[structopt(
        short,
        long,
        help = "Download image only, do not check device and migrate"
    )]
    download_only: bool,
    #[structopt(
        long,
        value_name = "TIMEOUT",
        parse(try_from_str),
        help = "API/VPN check timeout in seconds."
    )]
    check_timeout: Option<u64>,
    #[structopt(
        long,
        short,
        value_name = "LOG_DEVICE",
        parse(from_os_str),
        help = "Write stage2 log to LOG_DEVICE"
    )]
    log_to: Option<PathBuf>,
    #[structopt(
        short,
        long,
        value_name = "INSTALL_DEVICE",
        parse(from_os_str),
        help = "Use INSTALL_DEVICE to flash balena to"
    )]
    flash_to: Option<PathBuf>,
    #[structopt(
        long,
        help = "Do not create network manager configurations for configured wifis"
    )]
    no_wifis: bool,
    #[structopt(
        long,
        value_name = "SSID",
        help = "Create a network manager configuation for configured wifi with SSID"
    )]
    wifi: Option<Vec<String>>,
    #[structopt(
        long,
        value_name = "NWMGR_FILE",
        parse(from_os_str),
        help = "Supply a network manager file to inject into balena-os"
    )]
    nwmgr_cfg: Option<Vec<PathBuf>>,
}

impl Options {
    pub fn is_stage2(&self) -> bool {
        self.stage2
    }

    pub fn is_init(&self) -> bool {
        self.init
    }

    pub fn get_work_dir(&self) -> PathBuf {
        if let Some(work_dir) = &self.work_dir {
            work_dir.clone()
        } else {
            PathBuf::from("./")
        }
    }

    pub fn get_image(&self) -> &Option<PathBuf> {
        &self.image
    }

    pub fn get_version(&self) -> &str {
        if let Some(ref version) = self.version {
            version.as_str()
        } else {
            "default"
        }
    }

    pub fn is_no_ack(&self) -> bool {
        self.no_ack
    }

    pub fn is_migrate(&self) -> bool {
        !self.download_only
    }

    pub fn get_config(&self) -> &Option<PathBuf> {
        &self.config
    }

    pub fn is_pretend(&self) -> bool {
        self.pretend
    }

    pub fn is_build_num(&self) -> bool {
        self.build_num
    }

    pub fn get_log_file(&self) -> &Option<PathBuf> {
        &self.log_file
    }

    pub fn get_log_level(&self) -> Level {
        self.log_level
    }

    pub fn get_s2_log_level(&self) -> Level {
        if let Some(level) = self.s2_log_level {
            level
        } else {
            self.log_level
        }
    }

    pub fn is_os_check(&self) -> bool {
        !self.no_os_check
    }

    pub fn is_no_fail_on_efi(&self) -> bool {
        self.no_fail_on_efi
    }

    pub fn is_api_check(&self) -> bool {
        !self.no_api_check
    }

    pub fn is_vpn_check(&self) -> bool {
        !self.no_vpn_check
    }

    pub fn get_log_to(&self) -> &Option<PathBuf> {
        &self.log_to
    }

    pub fn get_flash_to(&self) -> &Option<PathBuf> {
        &self.flash_to
    }

    pub fn get_check_timeout(&self) -> u64 {
        if let Some(timeout) = self.check_timeout {
            timeout
        } else {
            DEFAULT_CHECK_TIMEOUT
        }
    }

    pub fn is_no_wifis(&self) -> bool {
        self.no_wifis
    }

    pub fn get_wifis(&self) -> &[String] {
        const NO_WIFIS: [String; 0] = [];
        if let Some(wifis) = &self.wifi {
            wifis.as_slice()
        } else {
            &NO_WIFIS
        }
    }

    pub fn get_nwmgr_cfg(&self) -> &[PathBuf] {
        if let Some(nwmgr_cfgs) = &self.nwmgr_cfg {
            nwmgr_cfgs.as_slice()
        } else {
            const NO_NWMGR_CFGS: [PathBuf; 0] = [];
            &NO_NWMGR_CFGS
        }
    }

    pub fn is_no_nwmgr_check(&self) -> bool {
        self.no_nwmgr_check
    }

    pub fn is_migrate_name(&self) -> bool {
        !self.no_keep_name
    }

    pub fn is_cleanup(&self) -> bool {
        !self.no_cleanup
    }
}
