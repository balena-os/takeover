
pub(crate) const MKTEMP_CMD: &str = "mktemp";
pub(crate) const UNAME_CMD: &str = "uname";
pub(crate) const CHMOD_CMD: &str = "chmod";
pub(crate) const MOUNT_CMD: &str = "mount";
pub(crate) const SWAPON_CMD: &str = "swapon";
pub(crate) const CP_CMD: &str = "cp";
pub(crate) const TTY_CMD: &str = "tty";
pub(crate) const TELINIT_CMD: &str = "telinit";
pub(crate) const REBOOT_CMD: &str = "reboot";
pub(crate) const UMOUNT_CMD: &str = "umount";
pub(crate) const DD_CMD: &str = "dd";

pub(crate) const STAGE2_CONFIG_NAME: &str = "stage2-config.yml";

pub(crate) const BALENA_IMAGE_NAME: &str = "balena.img.gz";
pub(crate) const BALENA_IMAGE_PATH: &str = "/balena.img.gz";


#[derive(Debug, Clone)]
pub(crate) enum OSArch {
    AMD64,
    #[cfg(target_os = "linux")]
    ARMHF,
    I386,
    /*
        ARM64,
        ARMEL,
        MIPS,
        MIPSEL,
        Powerpc,
        PPC64EL,
        S390EX,
    */
}

