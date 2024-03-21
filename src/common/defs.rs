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

// The mdtd_debug tool is used on Xavier NX devices to clear and write the QSPI
// with the boot blob included in the target OS image.
// This tools is provided by the mtd-utils package at
// http://git.infradead.org/?p=mtd-utils.git
pub(crate) const MTD_DEBUG_CMD: &str = "mtd_debug";

pub(crate) const TAR_CMD: &str = "tar";

pub(crate) const TAKEOVER_DIR: &str = "/balena-takeover";
pub(crate) const STAGE2_CONFIG_NAME: &str = "stage2-config.yml";

pub(crate) const BALENA_IMAGE_NAME: &str = "balena.img.gz";
pub(crate) const BALENA_IMAGE_PATH: &str = "/balena.img.gz";

pub(crate) const BALENA_CONFIG_PATH: &str = "/config.json";

pub const DISK_BY_LABEL_PATH: &str = "/dev/disk/by-label";

// balena boot partition name
pub const BALENA_BOOT_PART: &str = "resin-boot";

// Default balena boot partition filesystem type
pub const BALENA_BOOT_FSTYPE: &str = "vfat";

// balena rootA partition name
pub const BALENA_ROOTA_PART: &str = "resin-rootA";

// balena rootA partition filesystem type
pub const BALENA_ROOTA_FSTYPE: &str = "ext4";

// balena data partition name
pub const BALENA_DATA_PART: &str = "resin-data";

// balena data partition filesystem type
pub const BALENA_DATA_FSTYPE: &str = "ext4";

pub const OLD_ROOT_MP: &str = "/mnt/old_root";
pub const BALENA_BOOT_MP: &str = "/mnt/balena-boot";
pub const BALENA_PART_MP: &str = "/mnt/balena-part";

// balena directory which holds NetworkManager connection files.
// this directory is located in the resin-boot partition, in balenaOS
pub const SYSTEM_CONNECTIONS_DIR: &str = "system-connections";

// balena directory which holds redsocks proxy configuration files
pub const SYSTEM_PROXY_DIR: &str = "system-proxy";

// default mountpoint for the balenaOS data partition
pub const BALENA_DATA_MP: &str = "/mnt/data/";
pub const BALENA_OS_NAME: &str = "balenaOS";

pub const BALENA_SYSTEM_CONNECTIONS_BOOT_PATH: &str = "/mnt/boot/system-connections/";
pub const BALENA_SYSTEM_PROXY_BOOT_PATH: &str = "/mnt/boot/system-proxy/";

// Enables writing to the hardware-defined boot partition on AGX Xavier.
// For details on boot partitions access in Linux,
// see https://www.kernel.org/doc/Documentation/mmc/mmc-dev-parts.txt
pub const JETSON_XAVIER_HW_PART_FORCE_RO_FILE: &str = "/sys/block/mmcblk0boot0/force_ro";

// Hardware-defined boot partition for Jetson AGX Xavier
pub const BOOT_BLOB_PARTITION_JETSON_XAVIER: &str = "/dev/mmcblk0boot0";

// QSPI device - Jetson Xavier NX SD and NX eMMC
pub const BOOT_BLOB_PARTITION_JETSON_XAVIER_NX: &str = "/dev/mtd0";

// Stage 2 boot blob file names on AGX Xavier and Xavier NX. These are used
// for programming the QSPI, are provided by the target balenaOS image we migrate
// to, and they've been obtained from a device flashed with balenaOS using
// the Nvidia flashing tools.
pub const BOOT_BLOB_NAME_JETSON_XAVIER: &str = "boot0_mmcblk0boot0.img";
pub const BOOT_BLOB_NAME_JETSON_XAVIER_NX: &str = "boot0_mtdblock0.img";

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
