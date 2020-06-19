use std::fs::{self, create_dir, remove_dir, File, OpenOptions};
use std::io::{copy, Read};
use std::path::{Path, PathBuf};

use log::{debug, error, info, warn, Level};

use semver::{Identifier, Version, VersionReq};

use crate::{
    common::{
        defs::NIX_NONE,
        disk_util::{Disk, PartitionIterator, PartitionReader},
        loop_device::LoopDevice,
        path_append,
        stream_progress::StreamProgress,
        MigErrCtx, MigError, MigErrorKind,
    },
    stage1::api_calls::{get_os_image, get_os_versions, Versions},
    stage1::defs::{
        DEV_TYPE_BBB, DEV_TYPE_BBG, DEV_TYPE_GEN_X86_64, DEV_TYPE_INTEL_NUC, DEV_TYPE_RPI3,
        DEV_TYPE_RPI4_64,
    },
};

use crate::common::is_admin;
use crate::stage1::migrate_info::balena_cfg_json::BalenaCfgJson;
use failure::ResultExt;
use flate2::{Compression, GzBuilder};
use nix::mount::{mount, umount, MsFlags};

const FLASHER_DEVICES: [&str; 4] = [
    DEV_TYPE_INTEL_NUC,
    DEV_TYPE_GEN_X86_64,
    DEV_TYPE_BBG,
    DEV_TYPE_BBB,
];
const SUPPORTED_DEVICES: [&str; 6] = [
    DEV_TYPE_RPI3,
    DEV_TYPE_RPI4_64,
    DEV_TYPE_INTEL_NUC,
    DEV_TYPE_GEN_X86_64,
    DEV_TYPE_BBG,
    DEV_TYPE_BBB,
];

const IMG_NAME_GEN_X86_64: &str = "resin-image-genericx86-64-ext.resinos-img";
const IMG_NAME_INTEL_NUC: &str = "resin-image-genericx86-64.resinos-img";
const IMG_NAME_BBG: &str = "resin-image-beaglebone-green.resinos-img";
const IMG_NAME_BBB: &str = "resin-image-beaglebone-black.resinos-img";

fn parse_versions(versions: &Versions) -> Vec<Version> {
    let mut sem_vers: Vec<Version> = versions
        .versions
        .iter()
        .map(|ver_str| Version::parse(ver_str))
        .filter_map(|ver_res| match ver_res {
            Ok(version) => Some(version),
            Err(why) => {
                error!("Failed to parse version, error: {:?}", why);
                None
            }
        })
        .collect();
    sem_vers.sort();
    sem_vers.reverse();
    sem_vers
}

fn determine_version(ver_str: &str, versions: &Versions) -> Result<Version, MigError> {
    match ver_str {
        "latest" => {
            info!("Selected latest version ({}) for download", versions.latest);
            Ok(
                Version::parse(&versions.latest).context(upstream_context!(&format!(
                    "Failed to parse version from '{}'",
                    versions.latest
                )))?,
            )
        }
        "default" => {
            let mut found: Option<Version> = None;
            for cmp_ver in parse_versions(&versions) {
                debug!("Looking at version {}", cmp_ver);
                if cmp_ver.is_prerelease() {
                    continue;
                } else if cmp_ver
                    .build
                    .contains(&Identifier::AlphaNumeric("prod".to_string()))
                {
                    found = Some(cmp_ver);
                    break;
                }
            }

            if let Some(found) = found {
                info!("Selected default version ({}) for download", found);
                Ok(found)
            } else {
                error!("No version found for '{}'", ver_str);
                Err(MigError::displayed())
            }
        }
        _ => {
            if ver_str.starts_with('^') || ver_str.starts_with('~') {
                let ver_req = VersionReq::parse(ver_str).context(MigErrCtx::from_remark(
                    MigErrorKind::Upstream,
                    &format!("Failed to parse version from '{}'", ver_str),
                ))?;
                let mut found: Option<Version> = None;
                for cmp_ver in parse_versions(&versions) {
                    if ver_req.matches(&cmp_ver)
                        && !cmp_ver.is_prerelease()
                        && cmp_ver
                            .build
                            .contains(&Identifier::AlphaNumeric("prod".to_string()))
                    {
                        found = Some(cmp_ver);
                        break;
                    }
                }
                if let Some(found) = found {
                    info!("Selected version {} for download", found);
                    Ok(found)
                } else {
                    error!("No version found for '{}'", ver_str);
                    Err(MigError::displayed())
                }
            } else {
                let ver_req = Version::parse(ver_str).context(MigErrCtx::from_remark(
                    MigErrorKind::Upstream,
                    &format!("Failed to parse version from '{}'", ver_str),
                ))?;

                let mut found: Option<Version> = None;
                for cmp_ver in parse_versions(&versions) {
                    if ver_req == cmp_ver
                        && !cmp_ver.is_prerelease()
                        && (cmp_ver.build == ver_req.build
                            || cmp_ver
                                .build
                                .contains(&Identifier::AlphaNumeric("prod".to_string())))
                    {
                        found = Some(cmp_ver);
                        break;
                    }
                }
                if let Some(found) = found {
                    info!("Selected version {} for download", found);
                    Ok(found)
                } else {
                    error!("No version found for '{}'", ver_str);
                    Err(MigError::displayed())
                }
            }
        }
    }
}

fn extract_image<P1: AsRef<Path>, P2: AsRef<Path>>(
    stream: Box<dyn Read>,
    image_file_name: P1,
    device_type: &str,
    work_dir: P2,
) -> Result<(), MigError> {
    let work_dir = work_dir.as_ref();
    let progress = StreamProgress::new(stream, 10, Level::Info, None);
    let mut disk = Disk::from_gzip_stream(progress)?;
    let mut part_iterator = PartitionIterator::new(&mut disk)?;
    if let Some(part_info) = part_iterator.nth(1) {
        let mut reader = PartitionReader::from_part_iterator(&part_info, &mut part_iterator);
        let extract_file_name = path_append(work_dir, "root_a.img");
        let mut tmp_file = File::create(&extract_file_name).context(MigErrCtx::from_remark(
            MigErrorKind::Upstream,
            &format!(
                "Failed to create temporary file '{}'",
                extract_file_name.display()
            ),
        ))?;

        // TODO: show progress
        copy(&mut reader, &mut tmp_file).context(MigErrCtx::from_remark(
            MigErrorKind::Upstream,
            &format!(
                "Failed to extract root_a partition to temporary file '{}'",
                extract_file_name.display()
            ),
        ))?;

        info!("Finished root_a partition extraction, now mounting to extract balena OS image");

        let mut loop_device = LoopDevice::for_file(&extract_file_name, None, None, None, true)?;

        debug!("loop device is '{}'", loop_device.get_path().display());

        let mount_path = path_append(work_dir, "mnt_root_a");
        if !mount_path.exists() {
            create_dir(&mount_path).context(MigErrCtx::from_remark(
                MigErrorKind::Upstream,
                &format!("Failed to create directory '{}'", mount_path.display()),
            ))?;
        }

        debug!("mount path is '{}'", mount_path.display());
        mount(
            Some(loop_device.get_path()),
            &mount_path,
            Some(b"ext4".as_ref()),
            MsFlags::empty(),
            NIX_NONE,
        )
        .context(MigErrCtx::from_remark(
            MigErrorKind::Upstream,
            &format!(
                "Failed to mount '{}' on '{}",
                loop_device.get_path().display(),
                mount_path.display()
            ),
        ))?;

        debug!("retrieving path for device type '{}'", device_type);
        let img_path = match device_type {
            DEV_TYPE_INTEL_NUC => path_append(path_append(&mount_path, "opt"), IMG_NAME_INTEL_NUC),
            DEV_TYPE_GEN_X86_64 => {
                path_append(path_append(&mount_path, "opt"), IMG_NAME_GEN_X86_64)
            }
            DEV_TYPE_BBB => path_append(path_append(&mount_path, "opt"), IMG_NAME_BBB),
            DEV_TYPE_BBG => path_append(path_append(&mount_path, "opt"), IMG_NAME_BBG),
            _ => {
                error!(
                    "Encountered undefined image name for device type {}",
                    device_type
                );
                return Err(MigError::displayed());
            }
        };

        debug!("image path is '{}'", img_path.display());
        let img_file_name = image_file_name.as_ref();

        {
            let mut gz_writer = GzBuilder::new().write(
                File::create(img_file_name).context(MigErrCtx::from_remark(
                    MigErrorKind::Upstream,
                    &format!(
                        "Failed to open image file for writing: '{}'",
                        img_file_name.display()
                    ),
                ))?,
                Compression::best(),
            );

            let img_reader =
                OpenOptions::new()
                    .read(true)
                    .open(&img_path)
                    .context(MigErrCtx::from_remark(
                        MigErrorKind::Upstream,
                        &format!(
                            "Failed to open image file for reading: '{}'",
                            img_path.display()
                        ),
                    ))?;

            info!("Recompressing OS image to {}", img_file_name.display());

            let size = if let Ok(metadata) = img_reader.metadata() {
                Some(metadata.len())
            } else {
                None
            };

            let mut stream_progress = StreamProgress::new(img_reader, 10, Level::Info, size);

            copy(&mut stream_progress, &mut gz_writer).context(MigErrCtx::from_remark(
                MigErrorKind::Upstream,
                &format!(
                    "Failed to compress image '{}' to '{}'",
                    img_path.display(),
                    img_file_name.display()
                ),
            ))?;
        }

        info!(
            "The balena OS image was successfully written to '{}', cleaning up",
            img_file_name.display()
        );

        match umount(&mount_path) {
            Ok(_) => {
                if let Err(why) = remove_dir(&mount_path) {
                    warn!(
                        "Failed to remove mount temporary directory '{}', error: {:?}",
                        mount_path.display(),
                        why
                    );
                }
            }
            Err(why) => {
                warn!(
                    "Failed to unmount temporary mount from '{}', error: {:?}",
                    mount_path.display(),
                    why
                );
            }
        }

        loop_device.unset()?;

        if let Err(why) = fs::remove_file(&extract_file_name) {
            warn!(
                "Failed to remove extracted partition '{}', error: {:?}",
                extract_file_name.display(),
                why
            );
        }
        Ok(())
    } else {
        error!("Failed to find root_a partition in downloaded image");
        Err(MigError::displayed())
    }
}

pub(crate) fn download_image(
    balena_cfg: &BalenaCfgJson,
    work_dir: &Path,
    device_type: &str,
    version: &str,
) -> Result<PathBuf, MigError> {
    if !SUPPORTED_DEVICES.contains(&device_type) {
        error!(
            "OS download is not supported for device type {}",
            device_type
        );
        return Err(MigError::displayed());
    }

    let api_key = balena_cfg.get_api_key().context(upstream_context!(
        "Failed to retrieve api-key from config.json - unable to retrieve os-image"
    ))?;

    let api_endpoint = balena_cfg.get_api_endpoint().context(upstream_context!(
        "Failed to retrieve api-endpoint from config.json - unable to retrieve os-image"
    ))?;

    let versions = get_os_versions(&api_endpoint, &api_key, device_type)?;

    let version = determine_version(version, &versions)?;

    info!(
        "Downloading Balena OS image, selected version is: '{}'",
        version.to_string()
    );

    // TODO: extract OS image for flasher

    let stream = get_os_image(&api_endpoint, &api_key, device_type, &version.to_string())?;

    let img_file_name = path_append(
        work_dir,
        &format!(
            "balena-cloud-{}-{}.img.gz",
            device_type,
            version.to_string()
        ),
    );

    if FLASHER_DEVICES.contains(&device_type) {
        if !is_admin()? {
            error!("please run this program as root");
            return Err(MigError::from(MigErrorKind::Displayed));
        }
        extract_image(stream, &img_file_name, device_type, work_dir)?;
    } else {
        debug!("Downloading file '{}'", img_file_name.display());
        let mut file = File::create(&img_file_name).context(MigErrCtx::from_remark(
            MigErrorKind::Upstream,
            &format!("Failed to create file: '{}'", img_file_name.display()),
        ))?;

        // TODO: show progress
        let mut progress = StreamProgress::new(stream, 10, Level::Info, None);
        copy(&mut progress, &mut file).context(MigErrCtx::from_remark(
            MigErrorKind::Upstream,
            &format!(
                "Failed to write downloaded data to '{}'",
                img_file_name.display()
            ),
        ))?;
        info!(
            "The balena OS image was successfully written to '{}'",
            img_file_name.display()
        );
    }

    Ok(img_file_name)
}
