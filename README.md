Takeover is a CLI tool to migrate a running Linux device to balenaOS. It has been used to migrate thousands of Linux devices on distros like RaspberryPi OS and Ubuntu/Debian. We provide a `--pretend` option to help you test it out. _Our goal is to make it easy to migrate to balenaOS!_ Contact us in the [balena Forums](https://forums.balena.io/) with any questions.

**Warning: Takeover overwrites the currently running operating system with balenaOS.** Test on a lab device before running on your current fleet. Plan ahead to save or transfer any important data. 

The section below covers the most used scenarios and is the best place to start. Also see the [advanced use](docs/advanced.md) and [development/architecture](docs/development.md) docs for more in-depth information.

# Getting Started
Takeover is a single executable with many options. The sections below group these options by functionality, like downloading the target image and configuration.

First download the latest takeover executable for the target host architecture, [available](https://github.com/balena-os/takeover/releases/latest) from the repository.

## Common options
The only required option is `-c` for the balenaOS [config.json](https://docs.balena.io/reference/OS/configuration). This file includes important configuration like the fleet for the device.

Pretend mode allows you to run takeover until just before flashing balenaOS, and it also reboots the device when complete.
```
-c, --config <CONFIG_JSON>
    Path to balena config.json
--pretend
    Pretend mode, do not flash device
```

You can use the balenaCLI tool to [generate](https://docs.balena.io/reference/balena-cli/latest/#config-generate) a configuration for the target fleet, or you can download one from the [Add Device](https://docs.balena.io/learn/getting-started/var-som-mx6/rust/#add-a-device-and-download-os) dialog in the dashboard.

## balenaOS image
Takeover defaults to downloading the latest balenaOS image, or you may specify a particular version. You also can avoid downloading altogether and separately provide an image, but see the caution below. You also may only download an image without running the takeover process itself.
```
-v, --version <VERSION>
    Version of balena-os image to download
-i, --image <IMAGE>
    Path to balenaOS image
-d, --download-only
    Download image only, do not check device and migrate
```

### Download the image via takeover!
Depending on the device type and other context, balenaOS may be distributed as a _raw disk image_ or as a _flasher image_. See the link below, but **presently takeover requires a raw disk image.** The best way to retrieve a raw disk image is to let takeover download the image itself, either automatically or via the `-d` option described above. See the [advanced use]([advanced use](docs/advanced.md)) doc for more details.

## Network Setup
balenaOS uses NetworkManager for network configuration. By default, takeover generates this configuration by adapting any existing network configuration for tools like connman, wpa_supplicant and NetworkManager ifself. You also can explicitly supply configuration for a particular interface, or avoid generation of network configuration at all. For example, NetworkManager does not require configuration for a generic DHCP based Ethernet interface.

For WiFi, you can specify use of a single SSID, or avoid creation of any configurations.
```
--nwmgr-cfg <NWMGR_FILE>
    Supply a network manager file to inject into balenaOS
--no-nwmgr-check
    Do not check network manager files exist
--wifi <SSID>
    Create a network manager configuration for configured wifi with SSID
--no-wifis
    Do not create network manager configurations for configured wifis
```

## Logging