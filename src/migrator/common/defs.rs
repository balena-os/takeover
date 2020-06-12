pub(crate) const MKTEMP_CMD: &str = "mktemp";
pub(crate) const UNAME_CMD: &str = "uname";
pub(crate) const CHMOD_CMD: &str = "chmod";
pub(crate) const SWAPOFF_CMD: &str = "swapoff";
pub(crate) const CP_CMD: &str = "cp";
pub(crate) const TELINIT_CMD: &str = "telinit";
pub(crate) const REBOOT_CMD: &str = "reboot";

pub(crate) const MOKUTIL_CMD: &str = "mokutil";
pub(crate) const WHEREIS_CMD: &str = "whereis";
pub(crate) const PIDOF_CMD: &str = "pidof";

pub(crate) const DD_CMD: &str = "dd";
pub(crate) const FUSER_CMD: &str = "fuser";
pub(crate) const PS_CMD: &str = "ps";

pub(crate) const STAGE2_CONFIG_NAME: &str = "stage2-config.yml";

pub(crate) const BALENA_IMAGE_NAME: &str = "balena.img.gz";
pub(crate) const BALENA_IMAGE_PATH: &str = "/balena.img.gz";

pub(crate) const BALENA_CONFIG_PATH: &str = "/config.json";

pub const DISK_BY_LABEL_PATH: &str = "/dev/disk/by-label";

pub const BALENA_BOOT_PART: &str = "resin-boot";
pub const BALENA_BOOT_FSTYPE: &str = "vfat";

pub const BALENA_DATA_PART: &str = "resin-data";
pub const BALENA_DATA_FSTYPE: &str = "ext4";

pub const OLD_ROOT_MP: &str = "/mnt/old_root";
pub const BALENA_BOOT_MP: &str = "/mnt/balena-boot";
pub const BALENA_PART_MP: &str = "/mnt/balena-part";

pub const SYSTEM_CONNECTIONS_DIR: &str = "system-connections";

pub const BACKUP_ARCH_NAME: &str = "backup.tgz";

pub const NIX_NONE: Option<&'static [u8]> = None;
