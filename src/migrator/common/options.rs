use std::path::PathBuf;

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
    #[structopt(short, long, value_name = "CONFIG_JSON", parse(from_os_str))]
    config: Option<PathBuf>,
    #[structopt(long)]
    pretend: bool,
    #[structopt(long)]
    debug: bool,
    #[structopt(long)]
    trace: bool,
    #[structopt(long)]
    stage2: bool,
    #[structopt(long)]
    no_os_check: bool,
    #[structopt(long)]
    no_api_check: bool,
    #[structopt(long)]
    no_vpn_check: bool,
    #[structopt(
        long,
        value_name = "TIMEOUT",
        parse(try_from_str),
        help = "API/VPN check timeout in seconds."
    )]
    check_timeout: Option<u64>,
    #[structopt(long,short, value_name = "LOG_DEVICE", parse(from_os_str))]
    log_to: Option<PathBuf>,
    #[structopt(short, long, value_name = "INSTALL_DEVICE", parse(from_os_str))]
    flash_to: Option<PathBuf>,
}

impl Options {
    pub fn is_stage2(&self) -> bool {
        self.stage2
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

    pub fn get_config(&self) -> &Option<PathBuf> {
        &self.config
    }

    pub fn is_pretend(&self) -> bool {
        self.pretend
    }

    pub fn is_debug(&self) -> bool {
        self.debug
    }

    pub fn is_trace(&self) -> bool {
        self.trace
    }

    pub fn is_os_check(&self) -> bool {
        !self.no_os_check
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
}
