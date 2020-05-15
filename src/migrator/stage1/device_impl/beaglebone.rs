use log::{debug, error, info, trace};
use regex::Regex;

use crate::common::path_append;
use crate::common::path_info::PathInfo;
use crate::defs::{DEV_TYPE_BBG, DEV_TYPE_BBXM, MIG_INITRD_NAME, MIG_KERNEL_NAME};
use crate::{
    common::{
        boot_manager::BootManager,
        config::UEnvStrategy,
        migrate_info::MigrateInfo,
        stage2_config::{Stage2Config, Stage2ConfigBuilder},
        Config, MigError, MigErrorKind,
    },
    defs::{BootType, DeviceType, FileType},
    linux::{
        boot_manager_impl::{from_boot_type, u_boot_manager::UBootManager},
        device_impl::Device,
        linux_common::expect_type,
        stage2::mounts::Mounts,
    },
};

const SUPPORTED_OSSES: [&str; 4] = [
    "Ubuntu 18.04.2 LTS",
    "Ubuntu 14.04.1 LTS",
    "Debian GNU/Linux 9 (stretch)",
    "Debian GNU/Linux 7 (wheezy)",
];

// add some of this to balena bb XM command line:
// mtdparts=omap2-nand.0:512k(spl),1920k(u-boot),128k(u-boot-env),128k(dtb),6m(kernel),-(rootfs)
// mpurate=auto
// buddy=none
// camera=none
// vram=12M
// omapfb.mode=dvi:640x480MR-16@60 omapdss.def_disp=dvi
// rootwait

// const BBXM_KOPTS: &str ="mtdparts=omap2-nand.0:512k(spl),1920k(u-boot),128k(u-boot-env),128k(dtb),6m(kernel),-(rootfs) mpurate=auto buddy=none camera=none vram=12M omapfb.mode=dvi:640x480MR-16@60 omapdss.def_disp=dvi";
const BBXM_KOPTS: &str = "";

const BBG_KOPTS: &str = "";

// const BBB_KOPTS: &str = "";

// Supported models
// TI OMAP3 BeagleBoard xM
const BB_MODEL_REGEX: &str = r#"^((\S+\s+)*(\S+))\s+Beagle(Bone|Board)\s+(\S+)$"#;

const BBG_DTB_FILES: [&str; 2] = ["am335x-bonegreen.dtb", "am335x-bonegreen-wireless.dtb"];
const BBXM_DTB_FILES: [&str; 2] = ["omap3-beagle-xm.dtb", "omap3-beagle-xm-ab.dtb"];

const BBG_SLUGS: [&str; 1] = [DEV_TYPE_BBG];
const BBXM_SLUGS: [&str; 1] = [DEV_TYPE_BBXM];

// TODO: check location of uEnv.txt or other files files to improve reliability

fn dump_str(str: &str) -> String {
    let bytes = str.as_bytes();
    let mut res = String::new();
    bytes.iter().all(|byte| {
        res.push_str(&format!("{:02x} ", byte));
        true
    });
    res
}

pub(crate) fn is_bb(
    mig_info: &MigrateInfo,
    config: &Config,
    s2_cfg: &mut Stage2ConfigBuilder,
    model_string: &str,
) -> Result<Option<Box<dyn Device>>, MigError> {
    trace!(
        "Beaglebone::is_bb: entered with model string: '{}'",
        model_string
    );

    debug!(
        "comparing <{}> to <TI AM335x BeagleBone> -> {}",
        model_string,
        model_string.eq("TI AM335x BeagleBone")
    );

    debug!("model_string: {}", dump_str(model_string));
    debug!("comp:         {}", dump_str("TI AM335x BeagleBone"));

    if model_string.eq("TI AM335x BeagleBone") {
        // TODO: found this device model string on a beaglebone-green running debian wheezy
        debug!("match found for BeagleboneGreen");
        Ok(Some(Box::new(BeagleboneGreen::from_config(
            mig_info, config, s2_cfg,
        )?)))
    } else if let Some(captures) = Regex::new(BB_MODEL_REGEX).unwrap().captures(model_string) {
        let model = captures
            .get(5)
            .unwrap()
            .as_str()
            .trim_matches(char::from(0));

        match model {
            "xM" => {
                debug!("match found for BeagleboardXM");
                // TODO: dtb-name is a guess replace with real one
                Ok(Some(Box::new(BeagleboardXM::from_config(
                    mig_info, config, s2_cfg,
                )?)))
            }
            "Green" => {
                debug!("match found for BeagleboneGreen");
                Ok(Some(Box::new(BeagleboneGreen::from_config(
                    mig_info, config, s2_cfg,
                )?)))
            }
            /*
                        "Black" => {
                            debug!("match found for BeagleboneBlack");
                            // TODO: dtb-name is a guess replace with real one
                            Ok(Some(Box::new(BeagleboneBlack::from_config(
                                mig_info,
                                config,
                                s2_cfg,
                                format!("{}-boardblack.dtb", chip_name),
                            )?)))
                        }
            */
            _ => {
                let message = format!("The beaglebone model reported by your device ('{}') is not supported by balena-migrate", model);
                error!("{}", message);
                Err(MigError::from_remark(MigErrorKind::InvParam, &message))
            }
        }
    } else {
        debug!("no match for beaglebone on: <{}>", model_string);
        Ok(None)
    }
}

fn get_uboot_cfg(config: &Config, dev_type: DeviceType) -> (u8, UEnvStrategy) {
    if let Some(uboot_cfg) = config.get_uboot_cfg() {
        let mmc_index = if let Some(mmc_index) = uboot_cfg.mmc_index {
            mmc_index
        } else {
            match dev_type {
                DeviceType::BeagleboneGreen => 1,
                //DeviceType::BeagleboneBlack => 1,
                DeviceType::BeagleboardXM => 0,
                _ => 0,
            }
        };
        let strategy = if let Some(ref strategy) = uboot_cfg.strategy {
            strategy.clone()
        } else {
            UEnvStrategy::UName
        };
        (mmc_index, strategy)
    } else {
        (1, UEnvStrategy::UName)
    }
}

pub(crate) struct BeagleboneGreen {
    boot_manager: Box<dyn BootManager>,
}

impl BeagleboneGreen {
    // this is used in stage1
    fn from_config(
        mig_info: &MigrateInfo,
        config: &Config,
        s2_cfg: &mut Stage2ConfigBuilder,
    ) -> Result<BeagleboneGreen, MigError> {
        let os_name = &mig_info.os_name;

        expect_type(
            &path_append(&mig_info.work_path.path, MIG_KERNEL_NAME),
            &FileType::KernelARMHF,
        )?;

        expect_type(
            &path_append(&mig_info.work_path.path, MIG_INITRD_NAME),
            &FileType::InitRD,
        )?;

        let os_supported = config.is_no_os_check()
            || if let Some(_n) = SUPPORTED_OSSES.iter().position(|&r| r == os_name) {
                true
            } else {
                false
            };

        if os_supported {
            // TODO: make this configurable

            let (mmc_index, strategy) = get_uboot_cfg(config, DeviceType::BeagleboneGreen);

            info!(
                "Using uboot device index: {}, strategy is {:?}",
                mmc_index, strategy
            );

            let mut dtb_files: Vec<String> = Vec::new();
            BBG_DTB_FILES.iter().all(|f| {
                dtb_files.push(String::from(*f));
                true
            });

            let mut boot_manager = UBootManager::new(mmc_index, strategy, dtb_files);

            // TODO: determine boot device
            // use config.migrate.flash_device
            //

            if boot_manager.can_migrate(mig_info, config, s2_cfg)? {
                Ok(BeagleboneGreen {
                    boot_manager: Box::new(boot_manager),
                })
            } else {
                let message = format!(
                    "The boot manager '{:?}' is not able to set up your device",
                    boot_manager.get_boot_type()
                );
                error!("{}", &message);
                Err(MigError::from_remark(MigErrorKind::InvState, &message))
            }
        } else {
            let message = format!(
                "The OS '{}' is not supported for the device type BeagleboneGreen",
                os_name
            );
            error!("{}", &message);
            Err(MigError::from_remark(MigErrorKind::InvState, &message))
        }
    }

    // this is used in stage2
    pub fn from_boot_type(boot_type: BootType) -> BeagleboneGreen {
        BeagleboneGreen {
            boot_manager: from_boot_type(boot_type),
        }
    }
}

impl Device for BeagleboneGreen {
    fn get_device_type(&self) -> DeviceType {
        DeviceType::BeagleboneGreen
    }

    fn supports_device_type(&self, dev_type: &str) -> bool {
        BBG_SLUGS.contains(&dev_type)
    }

    fn get_boot_type(&self) -> BootType {
        self.boot_manager.get_boot_type()
    }

    fn setup(
        &mut self,
        mig_info: &mut MigrateInfo,
        config: &Config,
        s2_cfg: &mut Stage2ConfigBuilder,
    ) -> Result<(), MigError> {
        let kernel_opts = if let Some(ref kopts) = config.get_kernel_opts() {
            let mut new_opts: String = kopts.clone();
            new_opts.push(' ');
            new_opts.push_str(BBG_KOPTS);
            new_opts
        } else {
            String::from(BBG_KOPTS)
        };

        self.boot_manager
            .setup(mig_info, config, s2_cfg, &kernel_opts)
    }

    fn restore_boot(&self, mounts: &Mounts, config: &Stage2Config) -> bool {
        self.boot_manager.restore(mounts, config)
    }

    fn get_boot_device(&self) -> PathInfo {
        self.boot_manager.get_bootmgr_path()
    }
}

/*
pub(crate) struct BeagleboneBlack {
    boot_manager: Box<dyn BootManager>,
}

impl BeagleboneBlack {
    // this is used in stage1
    fn from_config(
        mig_info: &MigrateInfo,
        config: &Config,
        s2_cfg: &mut Stage2ConfigBuilder,
        dtb_name: String,
    ) -> Result<BeagleboneBlack, MigError> {
        let os_name = &mig_info.os_name;

        expect_type(&mig_info.kernel_file.path, &FileType::KernelARMHF)?;

        if let Some(_idx) = SUPPORTED_OSSES.iter().position(|&r| r == os_name) {
            let (mmc_index, strategy) = get_uboot_cfg(config, DeviceType::BeagleboneBlack);
            let mut dtb_files: Vec<String> = Vec::new();
            BBB_DTB_FILES.iter().all(|f| {
                dtb_files.push(String::from(*f));
                true
            });

            let mut boot_manager = UBootManager::new(mmc_index, strategy, dtb_name);

            if boot_manager.can_migrate(mig_info, config, s2_cfg)? {
                Ok(BeagleboneBlack {
                    boot_manager: Box::new(boot_manager),
                })
            } else {
                let message = format!(
                    "The boot manager '{:?}' is not able to set up your device",
                    boot_manager.get_boot_type()
                );
                error!("{}", &message);
                Err(MigError::from_remark(MigErrorKind::InvState, &message))
            }
        } else {
            let message = format!(
                "The OS '{}' is not supported for the device type BeagleboneBlack",
                os_name
            );
            error!("{}", message);
            Err(MigError::from_remark(MigErrorKind::InvState, &message))
        }
    }

    // this is used in stage2
    pub fn from_boot_type(boot_type: BootType) -> BeagleboneBlack {
        BeagleboneBlack {
            boot_manager: from_boot_type(boot_type),
        }
    }
}

impl Device for BeagleboneBlack {
    fn get_device_type(&self) -> DeviceType {
        DeviceType::BeagleboneBlack
    }

    fn get_device_slug(&self) -> &'static str {
        "beaglebone-black"
    }

    fn get_boot_type(&self) -> BootType {
        self.boot_manager.get_boot_type()
    }

    fn setup(
        &mut self,
        mig_info: &mut MigrateInfo,
        config: &Config,
        s2_cfg: &mut Stage2ConfigBuilder,
    ) -> Result<(), MigError> {
        let kernel_opts = if let Some(ref kopts) = config.migrate.get_kernel_opts() {
            let mut new_opts: String = kopts.clone();
            new_opts.push(' ');
            new_opts.push_str(BBB_KOPTS);
            new_opts
        } else {
            String::from(BBB_KOPTS)
        };

        self.boot_manager
            .setup(mig_info, config, s2_cfg, &kernel_opts)
    }

    fn restore_boot(&self, mounts: &Mounts, config: &Stage2Config) -> bool {
        self.boot_manager.restore(mounts, config)
    }

    fn get_boot_device(&self) -> DeviceInfo {
        self.boot_manager.get_bootmgr_path().device_info
    }
}
*/

pub(crate) struct BeagleboardXM {
    boot_manager: Box<dyn BootManager>,
}

impl BeagleboardXM {
    // this is used in stage1

    fn from_config(
        mig_info: &MigrateInfo,
        config: &Config,
        s2_cfg: &mut Stage2ConfigBuilder,
    ) -> Result<BeagleboardXM, MigError> {
        let os_name = &mig_info.os_name;

        expect_type(
            &path_append(&mig_info.work_path.path, MIG_KERNEL_NAME),
            &FileType::KernelARMHF,
        )?;
        expect_type(
            &path_append(&mig_info.work_path.path, MIG_INITRD_NAME),
            &FileType::InitRD,
        )?;

        let os_supported = config.is_no_os_check()
            || if let Some(_n) = SUPPORTED_OSSES.iter().position(|&r| r == os_name) {
                true
            } else {
                false
            };

        if os_supported {
            let (mmc_index, strategy) = get_uboot_cfg(config, DeviceType::BeagleboardXM);

            let mut dtb_files: Vec<String> = Vec::new();
            BBXM_DTB_FILES.iter().all(|f| {
                dtb_files.push(String::from(*f));
                true
            });

            let mut boot_manager = UBootManager::new(mmc_index, strategy, dtb_files);

            if boot_manager.can_migrate(mig_info, config, s2_cfg)? {
                Ok(BeagleboardXM {
                    boot_manager: Box::new(boot_manager),
                })
            } else {
                let message = format!(
                    "The boot manager '{:?}' is not able to set up your device",
                    boot_manager.get_boot_type()
                );
                error!("{}", &message);
                Err(MigError::from_remark(MigErrorKind::InvState, &message))
            }
        } else {
            let message = format!(
                "The OS '{}' is not supported for the device type BeagleboardXM",
                os_name
            );
            error!("{}", message);
            Err(MigError::from_remark(MigErrorKind::InvState, &message))
        }
    }

    // this is used in stage2
    pub fn from_boot_type(boot_type: BootType) -> BeagleboardXM {
        BeagleboardXM {
            boot_manager: from_boot_type(boot_type),
        }
    }
}

impl<'a> Device for BeagleboardXM {
    fn get_device_type(&self) -> DeviceType {
        DeviceType::BeagleboardXM
    }

    fn supports_device_type(&self, dev_type: &str) -> bool {
        BBXM_SLUGS.contains(&dev_type)
    }

    fn get_boot_type(&self) -> BootType {
        self.boot_manager.get_boot_type()
    }

    fn restore_boot(&self, mounts: &Mounts, config: &Stage2Config) -> bool {
        self.boot_manager.restore(mounts, config)
    }

    fn setup(
        &mut self,
        mig_info: &mut MigrateInfo,
        config: &Config,
        s2_cfg: &mut Stage2ConfigBuilder,
    ) -> Result<(), MigError> {
        let kernel_opts = if let Some(ref kopts) = config.get_kernel_opts() {
            let mut new_opts: String = kopts.clone();
            new_opts.push(' ');
            new_opts.push_str(BBXM_KOPTS);
            new_opts
        } else {
            String::from(BBXM_KOPTS)
        };

        self.boot_manager
            .setup(mig_info, config, s2_cfg, &kernel_opts)
    }

    fn get_boot_device(&self) -> PathInfo {
        self.boot_manager.get_bootmgr_path()
    }
}
