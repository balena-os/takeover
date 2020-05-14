use std::path::PathBuf;

use structopt::StructOpt;

#[derive(StructOpt, Debug, Clone)]
#[structopt(name = env!("CARGO_PKG_NAME"), author, about)]
pub struct Options {
    /// what to do
    #[structopt(short, long, value_name = "DIRECTORY", parse(from_os_str))]
    work_dir: Option<PathBuf>,
    #[structopt(short, long, value_name = "IMAGE", parse(from_os_str))]
    image: Option<PathBuf>,
    #[structopt(short, long, value_name = "VERSION")]
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
    #[structopt(short, long, value_name = "LOG_DEVICE", parse(from_os_str))]
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

    pub fn get_version(&self) -> &Option<String> {
        &self.version
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

    pub fn get_log_to(&self) -> &Option<PathBuf> {
        &self.log_to
    }

    pub fn get_flash_to(&self) -> &Option<PathBuf> {
        &self.flash_to
    }
}
