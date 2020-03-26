use std::path::{PathBuf, Path};
use std::fs::remove_dir_all;

use failure::{ResultExt, Fail};
use nix::{
    mount::{umount},
};
use log::warn;
use mod_logger::Level;


use crate::{
    common::{
        options::Options,
        get_os_name, file_exists,
        mig_error::{MigError, MigErrCtx, MigErrorKind},
        },
    stage1::assets::Assets,
};


#[derive(Debug)]
pub(crate) struct MigrateInfo {
    os_name: String,
    assets: Assets,
    work_path: PathBuf,
    image_path: PathBuf,
    mounts: Vec<PathBuf>,
    to_dir: Option<PathBuf>,
    log_level: Level
}

impl MigrateInfo {
    pub fn new(opts: &Options) -> Result<MigrateInfo, MigError> {

        let work_dir = opts.get_work_dir().canonicalize()
            .context(MigErrCtx::from_remark(MigErrorKind::Upstream,
                                            &format!("Failed to canonicalize work dir '{}'", opts.get_work_dir().display())))?;

        let image_path = opts.get_image()?;
        let image_path = if file_exists(&image_path) {
          image_path.canonicalize()
              .context(MigErrCtx::from_remark(MigErrorKind::Upstream,
                                              &format!("Failed to canonicalze path: '{}'", image_path.display())))?
        } else {
            let image_path = work_dir.join(&image_path);
            if file_exists(&image_path) {
                image_path.canonicalize()
                    .context(MigErrCtx::from_remark(MigErrorKind::Upstream,
                                                    &format!("Failed to canonicalze path: '{}'", image_path.display())))?
            } else {
                return Err(MigError::from_remark(MigErrorKind::NotFound, &format!("Could not find image: '{}'", opts.get_image()?.display())))
            }
        };

        Ok(MigrateInfo {
            work_path: work_dir,
            assets: Assets::new(),
            os_name: get_os_name()?,
            image_path,
            to_dir: None,
            mounts: Vec::new(),
            log_level: if opts.is_trace() {
                Level::Trace
            } else if opts.is_debug() {
                Level::Debug
            } else {
                Level::Info
            }
        })
    }

    pub fn get_assets(&self) -> &Assets {
        &self.assets
    }

    pub fn get_work_dir(&self) -> &Path {
        &self.work_path
    }

    pub fn get_image_path(&self) -> &Path {
        &self.image_path
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

    pub fn add_mount<P: AsRef<Path>>(&mut self, mount: P) {
        self.mounts.push(mount.as_ref().to_path_buf())
    }

    pub fn get_mounts(&self) -> &Vec<PathBuf> {
        &self.mounts
    }

    pub fn umount_all(&mut self)  {
        loop {
            if let Some(mountpoint) = self.mounts.pop() {
                if let Err(why) = umount(&mountpoint) {
                    warn!("Failed to unmount mountpoint: '{}', error : {:?}", mountpoint.display(), why);
                }
            } else {
                break;
            }
        }

        if let Some(takeover_dir) = &self.to_dir {
            if let Err(why) = remove_dir_all(takeover_dir) {
                warn!("Failed to remove takeover directory: '{}', error : {:?}", takeover_dir.display(), why);
            }
        }
    }
}
