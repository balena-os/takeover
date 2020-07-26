pub(crate) const SWAPOFF_CMD: &str = "swapoff";
pub(crate) const TELINIT_CMD: &str = "telinit";

pub(crate) const MOKUTIL_CMD: &str = "mokutil";
pub(crate) const WHEREIS_CMD: &str = "whereis";
pub(crate) const PIDOF_CMD: &str = "pidof";
pub(crate) const PIVOT_ROOT_CMD: &str = "pivot_root";
pub(crate) const MOUNT_CMD: &str = "mount";
pub(crate) const BLKID_CMD: &str = "blkid";

pub(crate) const EFIBOOTMGR_CMD: &str = "efibootmgr";
pub(crate) const DD_CMD: &str = "dd";

pub(crate) const TAR_CMD: &str = "tar";

pub(crate) const TAKEOVER_DIR: &str = "/balena-takeover";
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

pub const SYS_EFI_DIR: &str = "/sys/firmware/efi";
pub const SYS_EFIVARS_DIR: &str = "/sys/firmware/efi/efivars";

pub const BACKUP_ARCH_NAME: &str = "backup.tgz";

pub const NIX_NONE: Option<&'static [u8]> = None;

cfg_if::cfg_if! {
    if #[cfg(target_env = "musl")] {
        pub(crate) type IoctlReq = i32;
    } else if #[cfg(target_arch = "x86_64")] {
        pub(crate) type IoctlReq = u64;
    } else if #[cfg(target_arch = "arm")] {
        pub(crate) type IoctlReq = u32;
    } else if #[cfg(target_arch = "aarch64")]{
        pub(crate) type IoctlReq = u64;
    }
}
