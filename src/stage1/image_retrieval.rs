use std::fs::File;
use std::io::copy;
use std::path::{Path, PathBuf};

use log::{debug, error, info, warn, Level};

use semver::{Version, VersionReq};

use crate::{
    common::{
        api_calls::{get_os_image, get_os_versions, Versions},
        path_append,
        stream_progress::StreamProgress,
        Error, Options, Result, ToError,
    },
    stage1::{
        defs::{
            DEV_TYPE_BBB, DEV_TYPE_BBG, DEV_TYPE_GEN_X86_64, DEV_TYPE_INTEL_NUC,
            DEV_TYPE_JETSON_XAVIER, DEV_TYPE_RPI1, DEV_TYPE_RPI2, DEV_TYPE_RPI3, DEV_TYPE_RPI4_64,
        },
        migrate_info::balena_cfg_json::BalenaCfgJson,
    },
    ErrorKind,
};

const SUPPORTED_DEVICES: [&str; 9] = [
    DEV_TYPE_RPI3,
    DEV_TYPE_RPI2,
    DEV_TYPE_RPI4_64,
    DEV_TYPE_RPI1,
    DEV_TYPE_INTEL_NUC,
    DEV_TYPE_GEN_X86_64,
    DEV_TYPE_BBG,
    DEV_TYPE_BBB,
    DEV_TYPE_JETSON_XAVIER,
];

fn parse_versions(versions: &Versions) -> Vec<Version> {
    let mut sem_vers: Vec<Version> = versions
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

fn determine_version(ver_str: &str, versions: &Versions) -> Result<Version> {
    match ver_str {
        "default" => {
            let mut found: Option<Version> = None;
            for cmp_ver in parse_versions(versions) {
                debug!("Looking at version {}", cmp_ver);
                if cmp_ver.is_prerelease() {
                    continue;
                } else {
                    found = Some(cmp_ver);
                    break;
                }
            }

            if let Some(found) = found {
                info!("Selected default version ({}) for download", found);
                Ok(found)
            } else {
                Err(Error::with_context(
                    ErrorKind::InvParam,
                    &format!("No version found for '{}'", ver_str),
                ))
            }
        }
        _ => {
            if ver_str.starts_with('^') || ver_str.starts_with('~') {
                let ver_req = VersionReq::parse(ver_str).upstream_with_context(&format!(
                    "Failed to parse version from '{}'",
                    ver_str
                ))?;
                let mut found: Option<Version> = None;
                for cmp_ver in parse_versions(versions) {
                    if ver_req.matches(&cmp_ver) && !cmp_ver.is_prerelease() {
                        found = Some(cmp_ver);
                        break;
                    }
                }
                if let Some(found) = found {
                    info!("Selected version {} for download", found);
                    Ok(found)
                } else {
                    Err(Error::with_context(
                        ErrorKind::InvParam,
                        &format!("No version found for '{}'", ver_str),
                    ))
                }
            } else {
                let ver_req = Version::parse(ver_str).upstream_with_context(&format!(
                    "Failed to parse version from '{}'",
                    ver_str
                ))?;

                let mut found: Option<Version> = None;
                for cmp_ver in parse_versions(versions) {
                    if ver_req == cmp_ver
                        && !cmp_ver.is_prerelease()
                        && (cmp_ver.build == ver_req.build)
                    {
                        found = Some(cmp_ver);
                        break;
                    }
                }
                if let Some(found) = found {
                    info!("Selected version {} for download", found);
                    Ok(found)
                } else {
                    Err(Error::with_context(
                        ErrorKind::InvParam,
                        &format!("No version found for '{}'", ver_str),
                    ))
                }
            }
        }
    }
}

pub(crate) fn download_image(
    opts: &Options,
    balena_cfg: &BalenaCfgJson,
    work_dir: &Path,
    device_type: &str,
    version: &str,
) -> Result<PathBuf> {
    if !SUPPORTED_DEVICES.contains(&device_type) {
        if opts.dt_check() {
            return Err(Error::with_context(
                ErrorKind::InvParam,
                &format!(
                    "OS download is not supported for device type '{}', to override this check use the no-dt-check option on the command line",
                    device_type
                ),
            ));
        } else {
            warn!(
                "OS download is not supported for device type '{}', proceeding due to no-dt-check option",
                device_type
            );
        }
    }

    let api_key = balena_cfg.get_api_key().upstream_with_context(
        "Failed to retrieve api-key from config.json - unable to retrieve os-image",
    )?;

    let api_endpoint = balena_cfg.get_api_endpoint().upstream_with_context(
        "Failed to retrieve api-endpoint from config.json - unable to retrieve os-image",
    )?;

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
        format!("balena-cloud-{}-{}.img.gz", device_type, version),
    );

    debug!("Downloading file '{}'", img_file_name.display());
    let mut file = File::create(&img_file_name).upstream_with_context(&format!(
        "Failed to create file: '{}'",
        img_file_name.display()
    ))?;

    // TODO: show progress
    let mut progress = StreamProgress::new(stream, 10, Level::Info, None);
    copy(&mut progress, &mut file).upstream_with_context(&format!(
        "Failed to write downloaded data to '{}'",
        img_file_name.display()
    ))?;
    info!(
        "The balena OS image was successfully written to '{}'",
        img_file_name.display()
    );

    Ok(img_file_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    const VERSIONS: [&str; 6] = [
        "5.1.20+rev1",
        "3.2.25",
        "3.3.0",
        "4.0.26+rev",
        "5.0.1+rev1",
        "0.0.0+rev60",
    ];
    use mod_logger::Logger;

    #[test]
    fn returns_latest_version_by_default() {
        Logger::set_default_level(Level::Trace);

        let selection = "default";
        debug!("Selection is {}", selection);

        let versions: Versions = VERSIONS.iter().map(|&s| s.to_string()).collect();

        let result = determine_version(selection, &versions);
        assert_eq!(
            result.unwrap(),
            Version::parse("5.1.20+rev1").expect("Could not parse version")
        );
    }

    #[test]
    fn returns_specific_version() {
        Logger::set_default_level(Level::Trace);
        let selection = "4.0.26+rev";
        debug!("Selection is {}", selection);

        let versions: Versions = VERSIONS.iter().map(|&s| s.to_string()).collect();

        let result = determine_version(selection, &versions);
        assert_eq!(
            result.unwrap(),
            Version::parse("4.0.26+rev").expect("Could not parse version")
        );
    }

    #[test]
    fn returns_compatible_version() {
        Logger::set_default_level(Level::Trace);
        let selection = "^3.2";
        debug!("Selection is {}", selection);

        let versions: Versions = VERSIONS.iter().map(|&s| s.to_string()).collect();

        let result = determine_version(selection, &versions);
        assert_eq!(
            result.unwrap(),
            Version::parse("3.3.0").expect("Could not parse version")
        );
    }

    #[test]
    fn returns_closest_version() {
        Logger::set_default_level(Level::Trace);
        let selection = "~3.2.8";
        debug!("Selection is {}", selection);

        let versions: Versions = VERSIONS.iter().map(|&s| s.to_string()).collect();

        let result = determine_version(selection, &versions);
        assert_eq!(
            result.unwrap(),
            Version::parse("3.2.25").expect("Could not parse version")
        );
    }
}
