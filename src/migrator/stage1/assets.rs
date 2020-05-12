use std::path::{Path, PathBuf};
use std::os::unix::fs::PermissionsExt;
use std::io::Write;
use std::fs::{OpenOptions, write};

use failure::ResultExt;
use log::error;

use crate::{
    common::{
        defs::{OSArch, CHMOD_CMD},
        mig_error::{MigError, MigErrCtx, MigErrorKind},
        call,
    },
};
use crate::common::defs::OLD_ROOT_MP;

const RPI3_BUSYBOX: &[u8] = include_bytes!("../../../assets/armv7/busybox");
const X86_64_BUSYBOX: &[u8] = include_bytes!("../../../assets/x86_64/busybox");

const STAGE2_SCRIPT: &str = r###"#!__TO__/busybox sh
exec <"__TO____TTY__" >"__TO____TTY__" 2>"__TO____TTY__"
cd "__TO__"
./busybox echo "Init takeover successful"
./busybox echo "Pivoting root..."
./busybox mount --make-rprivate /
./busybox pivot_root . mnt/old_root
./busybox echo "Chrooting and running init..."
exec ./busybox chroot . /takeover stage2
"###;

#[derive(Debug)]
pub(crate) struct Assets {
    arch:  OSArch,
    busybox: &'static [u8],
}

impl Assets {
    pub  fn new() -> Assets {
        if cfg!(target_arch="arm") {
            Assets {
                arch: OSArch::ARMHF,
                busybox: RPI3_BUSYBOX,
            }
        } else if cfg!(target_arch="x86_64") {
            Assets {
                arch: OSArch::AMD64,
                busybox: X86_64_BUSYBOX,
            }
        } else {
            panic!("No assets are provided in binary - please compile with device feature")
        }
    }

    pub fn write_stage2_script<P1: AsRef<Path>, P2: AsRef<Path>, P3: AsRef<Path>>(to_dir: P1, out_path: P2, tty: P3) -> Result<(), MigError> {
        let s2_script = STAGE2_SCRIPT.replace("__TO__", &*to_dir.as_ref().to_string_lossy());
        let s2_script = s2_script.replace("__TTY__", &*tty.as_ref().to_string_lossy());
        write(out_path.as_ref(), &s2_script)
            .context(MigErrCtx::from_remark(MigErrorKind::Upstream, &format!("Failed to write stage 2 script to: '{}'", out_path.as_ref().display())))?;
        let cmd_res = call(CHMOD_CMD, &["+x", &*out_path.as_ref().to_string_lossy()], true)?;
        if cmd_res.status.success() {
            Ok(())
        } else {
            error!("Failed to set executable flags on stage 2 script: '{}', stderr: '{}'", out_path.as_ref().display(), cmd_res.stderr);
            Err(MigError::displayed())
        }
    }

    pub fn get_os_arch(&self) -> &OSArch {
        &self.arch
    }

    pub fn busybox_size(&self) -> usize {
        self.busybox.len()
    }

    pub fn write_to<P: AsRef<Path>>(&self,target_path: P) -> Result<PathBuf,MigError> {
        let target_path = target_path.as_ref().join("busybox");

        {
            let mut target_file = OpenOptions::new().create(true).write(true).read(false).open(&target_path)
                .context(MigErrCtx::from_remark(MigErrorKind::Upstream,
                                                &format!("Failed to open file for writing: '{}'", target_path.display())))?;
            target_file.write(self.busybox)
                .context(MigErrCtx::from_remark(MigErrorKind::Upstream,
                                                &format!("Failed to write to file: '{}'", target_path.display())))?;
        }

        /*
        let mut busybox_file = OpenOptions::new().create(false).write(true).open(&target_path)
            .context(MigErrCtx::from_remark(MigErrorKind::Upstream,
                                            &format!("Failed to set open '{}'", target_path.display())))?;

        let metadata = busybox_file.metadata()
            .context(MigErrCtx::from_remark(MigErrorKind::Upstream,
                                            &format!("Failed to get metadata for '{}'", target_path.display())))?;
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o755);
        */

        let cmd_res = call(CHMOD_CMD, &["+x", &*target_path.to_string_lossy()], true)?;

        if !cmd_res.status.success() {
            return Err(MigError::from_remark(MigErrorKind::CmdIO,
                                             &format!("Failed to set executable flags for '{}', stderr: '{}'", target_path.display(), cmd_res.stderr)));
        }

        Ok(target_path)
    }
}
