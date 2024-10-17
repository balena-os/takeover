use std::io::Read;

use log::debug;

use reqwest::{blocking::Client, header};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::common::{Error, ErrorKind, Result, ToError};

const OS_VERSION_URL_ENDPOINT: &str = "/v6/release";

const OS_IMG_URL: &str = "/download";

const DEVICE__TYPE_URL_ENDPOINT: &str = "/v6/device_type";

pub(crate) type Versions = Vec<String>;

#[derive(Debug, Serialize, Deserialize)]
struct ImageRequestData {
    #[serde(rename = "deviceType")]
    device_type: String,
    version: String,
    #[serde(rename = "fileType")]
    file_type: String,
    #[serde(rename = "imageType")]
    image_type: Option<String>,
}
/// Structs corresponding to API response for endpoint /v6/releases
#[derive(Serialize, Deserialize, Debug)]
struct ReleasesApiResponse {
    d: Vec<VersionEntry>,
}

#[derive(Serialize, Deserialize, Debug)]
struct VersionEntry {
    raw_version: String,
}

/// Structs corresponding to API response for endpoint /v6/device_type and with $select=id
#[derive(Serialize, Deserialize, Debug)]
struct DeviceTypeIdApiResponse {
    d: Vec<DeviceIdEntry>,
}

#[derive(Serialize, Deserialize, Debug)]
struct DeviceIdEntry {
    id: u32,
}

/// Structs corresponding to API response for DeviceType Contract
#[derive(Debug, Deserialize)]
struct ContractData {
    media: Media,
    #[serde(default)]
    #[serde(rename = "flashProtocol")]
    flash_protocol: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Media {
    #[serde(default)]
    #[serde(rename = "altBoot")]
    alt_boot: Option<Vec<String>>,
    #[serde(rename = "defaultBoot")]
    default_boot: String,
}

#[derive(Debug, Deserialize)]
struct Contract {
    data: ContractData,
}

#[derive(Debug, Deserialize)]
struct DeviceTypeContractInfo {
    contract: Contract,
}

#[derive(Debug, Deserialize)]
struct DeviceContractInfoApiResponse {
    d: Vec<DeviceTypeContractInfo>,
}

pub(crate) fn get_os_versions(api_endpoint: &str, api_key: &str, device: &str) -> Result<Versions> {
    let headers = get_header(api_key)?;

    // We currently default to non-ESR releases and use a percent-encoded template
    // TODO: Improve in the future by percent-encoding in code here
    let request_url =  format!("{api_endpoint}{OS_VERSION_URL_ENDPOINT}?$select=raw_version&$filter=(is_final%20eq%20true)%20and%20(is_passing_tests%20eq%20true)%20and%20(is_invalidated%20eq%20false)%20and%20(status%20eq%20%27success%27)%20and%20(belongs_to__application/any(bta:((bta/is_host%20eq%20true)%20and%20(bta/is_for__device_type/any(iodt:iodt/slug%20eq%20%27{device}%27)))%20and%20(not(bta/application_tag/any(at:at/tag_key%20eq%20%27release-policy%27))%20or%20(bta/application_tag/any(at:(at/tag_key%20eq%20%27release-policy%27)%20and%20(at/value%20eq%20%27default%27))))))&$orderby=created_at%20desc");

    debug!("get_os_versions: request_url: '{}'", request_url);

    let res = Client::builder()
        .default_headers(headers)
        .build()
        .upstream_with_context("Failed to create https client")?
        .get(&request_url)
        .send()
        .upstream_with_context(&format!(
            "Failed to send https request url: '{}'",
            request_url
        ))?;

    debug!("Result = {:?}", res);

    let status = res.status();
    if status == 200 {
        // The API call returns a response with the following structure:
        // {
        //     "d": [
        //         {
        //             "raw_version": "5.1.20+rev1"
        //         },
        //         {
        //             "raw_version": "5.1.20"
        //         }
        //     ]
        // }
        // Deserialize the JSON string into the ApiResponse struct
        let parsed_data = res
            .json::<ReleasesApiResponse>()
            .upstream_with_context("Failed to parse request results")?;

        // Extract the `raw_version` values into a Vec<String>
        let versions: Vec<String> = parsed_data
            .d
            .into_iter()
            .map(|entry| entry.raw_version)
            .collect();
        Ok(versions)
    } else {
        Err(Error::with_context(
            ErrorKind::InvState,
            &format!("Balena API request failed with status: {}", status),
        ))
    }
}

fn get_header(api_key: &str) -> Result<header::HeaderMap> {
    let mut headers = header::HeaderMap::new();
    headers.insert(
        header::AUTHORIZATION,
        header::HeaderValue::from_str(format!("Bearer {api_key}").as_str())
            .upstream_with_context("Failed to create auth header")?,
    );
    Ok(headers)
}

pub(crate) fn get_os_image(
    api_endpoint: &str,
    api_key: &str,
    device: &str,
    version: &str,
) -> Result<Box<dyn Read>> {
    let headers = get_header(api_key)?;
    let request_url = format!("{}{}", api_endpoint, OS_IMG_URL);

    let post_data = if is_device_image_flasher(api_endpoint, api_key, device)? {
        debug!("Downloading raw image for device type {device}");
        ImageRequestData {
            device_type: String::from(device),
            version: String::from(version),
            file_type: String::from(".gz"),
            image_type: Some(String::from("raw")),
        }
    } else {
        ImageRequestData {
            device_type: String::from(device),
            version: String::from(version),
            file_type: String::from(".gz"),
            image_type: None,
        }
    };

    debug!("get_os_image: request_url: '{}'", request_url);
    debug!("get_os_image: data: '{:?}'", post_data);

    let res = Client::builder()
        .default_headers(headers)
        .build()
        .upstream_with_context("Failed to create https client")?
        .post(&request_url)
        .json(&post_data)
        .send()
        .upstream_with_context(&format!(
            "Failed to send https request url: '{}'",
            request_url
        ))?;

    debug!("Result = {:?}", res);

    Ok(Box::new(res))
}

pub(crate) fn patch_device_type(
    api_endpoint: &str,
    api_key: &str,
    dt_slug: &str,
    uuid: &str,
) -> Result<()> {
    let headers = get_header(api_key)?;

    // Before we can patch the deviceType, we need to get the deviceId corresponding to the slug
    let dt_id_request_url = get_device_type_info_url(api_endpoint, "id", dt_slug);

    debug!(
        "patch_device_type: dt_id_request_url: '{}'",
        dt_id_request_url
    );

    let res = Client::builder()
        .default_headers(headers.clone())
        .build()
        .upstream_with_context("Failed to create https client")?
        .get(&dt_id_request_url)
        .send()
        .upstream_with_context(&format!(
            "Failed to send https request url: '{}'",
            dt_id_request_url
        ))?;

    debug!("dt_id_request Result = {:?}", res);

    let status = res.status();
    if status.is_success() {
        // The API call returns a response with the following structure:
        // {
        //     "d": [
        //         {
        //             "id": 24
        //         }
        //     ]
        // }
        // Deserialize the JSON string into the ApiResponse struct
        let parsed_id_resp = res
            .json::<DeviceTypeIdApiResponse>()
            .upstream_with_context("Failed to parse request results")?;

        // Extract the device type id
        let id = &parsed_id_resp.d[0].id;
        debug!("device type {dt_slug} has id: {id}");

        // PATCH deviceType
        let patch_url = format!("{api_endpoint}/v6/device(uuid='{uuid}')");
        let patch_data = json!({
            "is_of__device_type": id
        });

        let patch_res = Client::builder()
            .default_headers(headers)
            .build()
            .upstream_with_context("Failed to create https client")?
            .patch(&patch_url)
            .json(&patch_data)
            .send()
            .upstream_with_context(&format!(
                "Failed to send https request url: '{}'",
                patch_url
            ))?;

        debug!("PATCH request Result = {:?}", patch_res);

        if patch_res.status().is_success() {
            debug!("Device type successfully patched to {dt_slug}");
            Ok(())
        } else {
            Err(Error::with_context(
                ErrorKind::InvState,
                &format!(
                    "Balena API request failed with status: {}",
                    patch_res.status()
                ),
            ))
        }
    } else {
        Err(Error::with_context(
            ErrorKind::InvState,
            &format!(
                "Balena API GET Device Type id request failed with status: {}",
                status
            ),
        ))
    }
}

// PATCH device state with HUP details
pub(crate) fn notify_hup_progress(api_endpoint: &str, api_key: &str, uuid: &str, progress_pct: &str, progress_msg: &str) -> Result<()> {
    let api_url = format!("{}/v6/device(uuid='{}')", api_endpoint, uuid);
    let headers = get_header(api_key)?;
    let patch_data = json!({
        "provisioning_progress": progress_pct,
        "provisioning_state": progress_msg,
        "status": "configuring"
    });

    let res = Client::builder()
        .default_headers(headers.clone())
        .build()
        .upstream_with_context("Failed to create https client")?
        .patch(&api_url)
        .json(&patch_data)
        .send()
        .upstream_with_context(&format!(
            "Failed to send https request url: {}",
            &api_url
        ))?;
    debug!("HUP progress result = {:?}", res);
    let status = res.status();
    let response = res
        .text()
        .upstream_with_context("Failed to read response")?;

    if status.is_success() {
        Ok(())
    } else {
        Err(Error::with_context(
            ErrorKind::InvState,
            &format!(
                "Got an unexpected reply from the API server @ {} : {}",
                &api_url, &response
            ),
        ))
    }
}

fn is_device_image_flasher(api_endpoint: &str, api_key: &str, device: &str) -> Result<bool> {
    let headers = get_header(api_key)?;

    let dt_contract_request_url = get_device_type_info_url(api_endpoint, "contract", device);
    let res = Client::builder()
        .default_headers(headers.clone())
        .build()
        .upstream_with_context("Failed to create https client")?
        .get(&dt_contract_request_url)
        .send()
        .upstream_with_context(&format!(
            "Failed to send https request url: '{}'",
            dt_contract_request_url
        ))?;

    debug!("dt_contract_request Result = {:?}", res);

    let status = res.status();
    if status.is_success() {
        let parsed_contract_resp = res
            .json::<DeviceContractInfoApiResponse>()
            .upstream_with_context("Failed to parse request results")?;

        // determine if device type's OS image is of flasher type
        // ref: https://github.com/balena-io/contracts/blob/d06ad25196f67c4d20ad309941192fdddf80e307/README.md?plain=1#L81
        let device_contract = &parsed_contract_resp.d[0];
        debug!("Device contract for {device} is {:?}", device_contract);

        // If the defaultBoot is internal and there is an alternative boot method like sdcard and no flashProtocol defined -> flasher
        if device_contract.contract.data.media.default_boot == "internal"
            && device_contract
                .contract
                .data
                .media
                .alt_boot
                .as_ref()
                .is_some_and(|alt_boot_vec| !alt_boot_vec.is_empty())
            && device_contract.contract.data.flash_protocol.is_none()
        {
            Ok(true)
        } else {
            Ok(false)
        }
    } else {
        Err(Error::with_context(
            ErrorKind::InvState,
            &format!(
                "Balena API GET Device Type contract request failed with status: {}",
                status
            ),
        ))
    }
}

fn get_device_type_info_url(api_endpoint: &str, select: &str, device: &str) -> String {
    format!("{api_endpoint}{DEVICE__TYPE_URL_ENDPOINT}?$orderby=name%20asc&$top=1&$select={select}&$filter=device_type_alias/any(dta:dta/is_referenced_by__alias%20eq%20%27{device}%27)")
}
