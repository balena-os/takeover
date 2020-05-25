pub(crate) const MKTEMP_CMD: &str = "mktemp";
pub(crate) const UNAME_CMD: &str = "uname";
pub(crate) const CHMOD_CMD: &str = "chmod";
pub(crate) const MOUNT_CMD: &str = "mount";
pub(crate) const SWAPOFF_CMD: &str = "swapoff";
pub(crate) const CP_CMD: &str = "cp";
pub(crate) const TELINIT_CMD: &str = "telinit";
pub(crate) const REBOOT_CMD: &str = "reboot";
// pub(crate) const UMOUNT_CMD: &str = "umount";
pub(crate) const LOSETUP_CMD: &str = "losetup";
pub(crate) const MOKUTIL_CMD: &str = "mokutil";
pub(crate) const WHEREIS_CMD: &str = "whereis";
pub(crate) const GZIP_CMD: &str = "gzip";
pub(crate) const DD_CMD: &str = "dd";

pub(crate) const STAGE2_CONFIG_NAME: &str = "stage2-config.yml";

pub(crate) const BALENA_IMAGE_NAME: &str = "balena.img.gz";
pub(crate) const BALENA_IMAGE_PATH: &str = "/balena.img.gz";

// pub(crate) const BALENA_CONFIG_NAME: &str = "config.json";
pub(crate) const BALENA_CONFIG_PATH: &str = "/config.json";

pub const DISK_BY_LABEL_PATH: &str = "/dev/disk/by-label";
// pub const DISK_BY_PARTUUID_PATH: &str = "/dev/disk/by-partuuid";
// pub const DISK_BY_UUID_PATH: &str = "/dev/disk/by-uuid";

pub const BALENA_BOOT_PART: &str = "resin-boot";
pub const BALENA_BOOT_FSTYPE: &str = "vfat";

pub const OLD_ROOT_MP: &str = "/mnt/old_root";
pub const BALENA_BOOT_MP: &str = "/mnt/balena-boot";
pub const BALENA_PART_MP: &str = "/mnt/balena-part";

pub const TRANSFER_DIR: &str = "/transfer";
pub const SYSTEM_CONNECTIONS_DIR: &str = "system-connections";

pub const NIX_NONE: Option<&'static [u8]> = None;

/*pub const BALENA_ROOTA_PART: &str = "resin-rootA";
pub const BALENA_ROOTA_FSTYPE: &str = "ext4";
pub const BALENA_ROOTB_PART: &str = "resin-rootB";
pub const BALENA_ROOTB_FSTYPE: &str = "ext4";
pub const BALENA_STATE_PART: &str = "resin-state";
pub const BALENA_STATE_FSTYPE: &str = "ext4";

pub const BALENA_DATA_PART: &str = "resin-data";
pub const BALENA_DATA_FSTYPE: &str = "ext4";
*/
