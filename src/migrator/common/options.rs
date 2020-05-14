use std::path::PathBuf;

use structopt::StructOpt;

#[derive(StructOpt, Debug, Copy, Clone, PartialEq)]
pub enum Action {
    /// migrate the device
    Migrate,
    /// check configuration but do not migrate
    Pretend,
    /// run as stage 2 instead of as stage 1 executable.
    Stage2,
}

#[derive(StructOpt, Debug, Clone)]
#[structopt(name = env!("CARGO_PKG_NAME"), author, about)]
pub struct Options {
    /// what to do
    #[structopt(subcommand)]
    command: Action,
    /// The working directory
    #[structopt(short, long, value_name = "DIRECTORY", parse(from_os_str))]
    work_dir: Option<PathBuf>,
    #[structopt(short, long, value_name = "IMAGE", parse(from_os_str))]
    image: Option<PathBuf>,
    #[structopt(short)]
    debug: bool,
    #[structopt(short)]
    trace: bool,
    #[structopt(short, long, value_name = "LOG_DEVICE", parse(from_os_str))]
    log_to: Option<PathBuf>,
    #[structopt(short, long, value_name = "INSTALL_DEVICE", parse(from_os_str))]
    flash_to: Option<PathBuf>,
    #[structopt(short, long, value_name = "CONFIG_JSON", parse(from_os_str))]
    config: Option<PathBuf>,
}

impl Options {
    pub fn get_cmd(&self) -> &Action {
        &self.command
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

    pub fn get_config(&self) -> &Option<PathBuf> {
        &self.config
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
