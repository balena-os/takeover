use std::fs::{write, OpenOptions};
use std::io::{copy, Read};
use std::path::{Path, PathBuf};

use log::Level;

use crate::{
    common::{
        error::{Result, ToError},
        system::chmod,
    },
    stage1::defs::OSArch,
};
use flate2::read::GzDecoder;

#[cfg(target_arch = "arm")]
const BUSYBOX_BIN: &[u8] = include_bytes!("../../../assets/armv7/busybox.gz");
#[cfg(target_arch = "x86_64")]
const BUSYBOX_BIN: &[u8] = include_bytes!("../../../assets/x86_64/busybox.gz");

const BUILD_NUM: &[u8] = include_bytes!("../../../build.num");

const STAGE2_SCRIPT: &str = r###"#!__TO__/busybox sh
echo "takeover init started"
if [ -f "__TO____TTY__" ]; then 
  exec <"__TO____TTY__" >"__TO____TTY__" 2>"__TO____TTY__"
fi
cd "__TO__"
echo "Init takeover successful"
echo "Pivoting root..."
mount --make-rprivate /
pivot_root . mnt/old_root
echo "Chrooting and running init..."
exec ./busybox chroot . /takeover --init --s2-log-level __LOG_LEVEL__
"###;

#[derive(Debug)]
pub(crate) struct Assets {
    arch: OSArch,
    busybox: &'static [u8],
}

impl Assets {
    pub fn new() -> Assets {
        let arch = if cfg!(target_arch = "arm") {
            OSArch::ARMHF
        } else if cfg!(target_arch = "x86_64") {
            OSArch::AMD64
        } else {
            panic!("No assets are provided in binary - please compile with device feature")
        };

        Assets {
            arch,
            busybox: BUSYBOX_BIN,
        }
    }

    pub fn get_build_num() -> Result<u32> {
        let build_str = String::from_utf8(BUILD_NUM.to_owned()).upstream_with_context(&format!(
            "Failed to parse string from build num {:?}",
            BUILD_NUM
        ))?;

        Ok(build_str.parse::<u32>().upstream_with_context(&format!(
            "Failed to parse buuild num from string '{}'",
            build_str
        ))?)
    }

    pub fn write_stage2_script<P1: AsRef<Path>, P2: AsRef<Path>, P3: AsRef<Path>>(
        to_dir: P1,
        out_path: P2,
        tty: P3,
        log_level: Level,
    ) -> Result<()> {
        let s2_script = STAGE2_SCRIPT.replace("__TO__", &*to_dir.as_ref().to_string_lossy());
        let s2_script = s2_script.replace("__TTY__", &*tty.as_ref().to_string_lossy());
        let s2_script = s2_script.replace("__LOG_LEVEL__", log_level.to_string().as_str());
        write(out_path.as_ref(), &s2_script).upstream_with_context(&format!(
            "Failed to write stage 2 script to: '{}'",
            out_path.as_ref().display()
        ))?;

        chmod(out_path, 0o755)?;
        Ok(())
    }

    pub fn busybox_size(&self) -> Result<u64> {
        let mut decoder = GzDecoder::new(self.busybox);
        let mut size: u64 = 0;
        const BUFFER_SIZE: usize = 0x0010_0000;
        let mut buffer: [u8; BUFFER_SIZE] = [0; BUFFER_SIZE];
        loop {
            let bytes_read = decoder
                .read(&mut buffer)
                .upstream_with_context("Failed to uncompress busybox executable")?;
            if bytes_read > 0 {
                size += bytes_read as u64
            } else {
                break;
            }
        }

        Ok(size)
    }

    pub fn write_to<P: AsRef<Path>>(&self, target_path: P) -> Result<PathBuf> {
        let target_path = target_path.as_ref().join("busybox");

        {
            let mut decoder = GzDecoder::new(self.busybox);
            let mut target_file = OpenOptions::new()
                .create(true)
                .write(true)
                .read(false)
                .open(&target_path)
                .upstream_with_context(&format!(
                    "Failed to open file for writing: '{}'",
                    target_path.display()
                ))?;

            copy(&mut decoder, &mut target_file).upstream_with_context(&format!(
                "Failed to decompress busybox executable to '{}'",
                target_path.display()
            ))?;
        }

        chmod(&target_path, 0o755)?;

        Ok(target_path)
    }
}
