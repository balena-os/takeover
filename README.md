Takeover is a CLI tool to migrate existing live Linux devices to balenaOS. It has been used to migrate from RaspberryPi OS, Ubuntu/Debian, and others.

**Warning: *takeover* overwrites the currently running operating system with balenaOS.** Test on a lab device before running on your current fleet. Plan ahead to save or transfer any important data. 

We can't guarantee that takeover will work for your fleet. However, we provide a `--pretend` option to help you test it out. _Our goal is to make it easy to migrate to balenaOS!_ Contact us in the [balena Forums](https://forums.balena.io/) with any questions.

The section below covers the most used scenarios and is the best place to start. Also see the separate [advanced options](docs/advanced.md) and [development/architecture](docs/development.md) docs.

# Getting Started
Takeover is a single executable with many options. The sections below group these options by functionality, like downloading the image or configuration.

First download the latest takeover executable for the target host architecture, [available](https://github.com/balena-os/takeover/releases/latest) from the repository.

## Common options

The only required option is `-c` for the balenaOS `config.json`. This file includes important configuration like the fleet for the device.

Pretend mode allows you to run takeover until just before flashing balenaOS, and it also reboots the device when complete.
```
-c, --config <CONFIG_JSON>
    Path to balena config.json
--pretend
    Pretend mode, do not flash device
```

You can use the balenaCLI tool to [generate](https://docs.balena.io/reference/balena-cli/latest/#config-generate) a configuration for the target fleet, or you can download one from the [Add Device](https://docs.balena.io/learn/getting-started/var-som-mx6/rust/#add-a-device-and-download-os) dialog in the dashboard.

## balenaOS image
Takeover defaults to downloading the latest balenaOS image, or you may specify a particular version. You also can avoid downloading altogether and separately provide an image. You also may only download an image without running the takeover process itself.

```
-v, --version <VERSION>
    Version of balena-os image to download
-i, --image <IMAGE>
    Path to balenaOS image
-d, --download-only
    Download image only, do not check device and migrate
```

## Howto 

Takeover consists of a single executable that supports automatic download of all assets required for migration. 
All that is needed to migrate a device to balena-os is a valid config.json typically obtained from the dashboard of your 
balena application. 

```shell script
> takeover --help

Options:
  -w, --work-dir <DIRECTORY>
          Path to working directory
  -i, --image <IMAGE>
          Path to balena-os image
  -v, --version <VERSION>
          Version of balena-os image to download
  -c, --config <CONFIG_JSON>
          Path to balena config.json
      --log-level <LOG_LEVEL>
          Set log level, one of [error,warn,info,debug,trace] [default: info]
      --log-file <LOG_FILE>
          Set stage1 log file name
      --fallback-log
          Logs to RAM and then dumps logs to balenaOS disk after flashing
      --fallback-log-filename <FALLBACK_LOG_FILENAME>
          Set the name of the fallback log [default: fallback.log]
      --fallback-log-dir <FALLBACK_LOG_DIR>
          Set the directory name where fallback logs will be persisted on data partition [default: fallback_log]
      --backup-cfg <BACKUP-CONFIG>
          Backup configuration file
      --s2-log-level <S2_LOG_LEVEL>
          Set stage2 log level, one of [error,warn,info,debug,trace]
      --no-ack
          Scripted mode - no interactive acknowledgement of takeover
      --pretend
          Pretend mode, do not flash device
      --stage2
          Internal - stage2 invocation
      --report-hup-progress
          Internal - notify balena API on success/failure
      --tar-internal
          Use internal tar instead of external command
      --no-cleanup
          Debug - do not cleanup after stage1 failure
      --no-os-check
          Do not check if OS is supported
      --no-dt-check
          Do not check if the target device type is valid
      --no-api-check
          Do not check if balena API is available
      --no-vpn-check
          Do not check if balena VPN is available
      --no-efi-setup
          Do not setup EFI boot
      --no-nwmgr-check
          Do not check network manager files exist
      --no-keep-name
          Do not migrate host-name
  -d, --download-only
          Download image only, do not check device and migrate
      --check-timeout <TIMEOUT>
          API/VPN check timeout in seconds.
  -l, --log-to <LOG_DEVICE>
          Write stage2 log to LOG_DEVICE
  -f, --flash-to <INSTALL_DEVICE>
          Use INSTALL_DEVICE to flash balena to
      --no-wifis
          Do not create network manager configurations for configured wifis
      --wifi <SSID>
          Create a network manager configuration for configured wifi with SSID
      --nwmgr-cfg <NWMGR_FILE>
          Supply a network manager file to inject into balena-os
      --change-dt-to <DT_SLUG>
          Device Type slug to change to
  -h, --help
          Print help
```

To download a config.json, please direct your browser to  the [balena dashboard](https://balena.io), logging in to to your user 
account and selecting the application you want to migrate the device to. From there you can press the 'add device' button 
in the top left, in the 'Add new device' dialog select 'Advanced' and  'Download configuration file only'. 

In the most simple case all you need to do now, is copy the *takeover* executable and the config.json to a folder on the 
device you would like to migrate and execute 
```
sudo ./takeover -c config.json
``` 
on the command line.
 
The above command will download the latest image for your platform and migrate the device to balena. 

Several options are available to cover special situations: 

### Image Selection

The *takeover* command allows you to specify a balena-os version for download or an image to use for migration.

#### Downloading an image

By default *takeover* will download the latest image for the platform specified in your config.json. 
If you need a version different from the latest you can use the ```--version``` option to specify 
a version. 
The ```--version``` option accepts either a full image name (eg. ```--version 5.1.20+rev1```) or parsing 
of ~x.y.z and ^x.y.z requirements as defined at [semver](https://www.npmjs.com/package/semver)
 (eg. ```--version ~5.1```).
 Example: 
 ```shell script
sudo ./takeover -c config.json --version 5.1.20+rev1
```
   
When downloading images,  certain platforms (mainly intel-nuc, Generic-x86_64, beaglebone) require unpacking the image and 
extracting the actual OS-image. The *takeover* command does this automatically but the process of unpacking temporarily 
requires up to 2.3GB of disk space. You can use the --work-dir option to specify a working directory that has sufficient 
disk space (eg. a memory stick) to unpack if your current directory does not. Otherwise you can use *takeover* 
on a computer with sufficient disk space to download the image, copy it to the target device and use the 
```-i / --image``` as described below.

The ```-d / --download-only``` option allows you to download an image without installing it. This option also 
disables most checks, so that you can download an image e.g. for your RaspberryPI 3 using your X86 PC. 
All you need to do is use a config.json for a raspberry PI and the ```-d``` option.

Example - Download only of a balena OS image: 
```shell script
sudo ./takeover -d --version 2.50.1+rev1.dev -c config.json 
```
 

#### Specifying an existing image

You can use the ```-i / --image``` option to specify any valid balena-os image. 

**Warning:** Please be aware that specifying an invalid 
image might lead to your target device being flashed with something invalid which will very likely lead to it not booting. 

Be careful with images you have downloaded from the [balena dashboard](https://balena.io). These images are zip encoded 
and need to be unpacked and recompressed using gzip as follows: 
```shell script
unzip <image-name>
gzip <unpacked image name>
```  
For certain device types (mainly intel-nuc., Generic x86_64, beaglebone) the image downloaded will be a flasher image
that contains the actual balena-os image. For these platforms it is easier to let *takeover* do the download and extraction. 
     
### Network Setup

The *takeover* command will try to migrate your existing wifi configuration unless you have disabled it using the 
```--no-wifis``` option. *takeover* will scan for connmanager, wpa_supplicant and NetworkManager configurations. 

Using the ```--wifi``` option you can instruct *takeover* to migrate only specified wifis. 

You can also specify your own NetworkManager configuration file using the ```--nwmgr-cfg``` option. 

If no network configurations are found *takeover* will print an error message and abort to keep you from accidentally 
migrating a configuration that will not be able to come online. This check can be overridden by specifying the 
```--no-nwmgr-check``` option.

By default *takeover* will migrate the devices hostname. This can be disabled using the ```--no-keep-name``` option. 

### Logging
By default *takeover* runs at *info* log level. It will log to the console. 
You can modify the stage1 log-level by using the ```--log-level``` option. Available log levels are:
- `error`
- `warn`
- `info`
- `debug`
- `trace`
 
Stage1 is the first part of migration - mainly the preparation of the migration process. Everything happening in stage1 
can be logged to the console.
 
At the end of stage1 *takeover* switches the file system root to a RAMFS file system and replaces the init process. 
This part of migration is called stage2. In stage2 the console does not receive output from *takeover* any more and 
ssh-sessions will usually be disconnected. In stage2, the disk running the original OS is overwritten. The log device should be formatted with one  *vfat*, *ext3* or *ext4* file system.

#### Logging to an external disk or secondary internal disk
The recommended method to capture logs during the migration process is to provide log device/disk which is independent from the disk that balena will be installed on. Usually a secondary (internal) disk or a USB stick works well.

For this reason you can specify a log device using the ```-l / --log-to``` option. 
You should use a device that is independent from the disk that balena will be installed on. Usually a secondary disk 
or a USB stick works well. The log device should be formatted with one fhe following file systems:
- `vfat`
- `ext3` 
- `ext4`

You can specify a log device using the `-l / --log-to` option. 
It also makes sense to adapt the stage2 log level to see a maximum of information. This can be done using the ```-s / --s2-log-level``` option. Log levels are as given above. 

Example, writing a stage2 log to /dev/sda1 with stage2 log level `debug`:
```shell script
sudo ./takeover -c config.json -l /dev/sda1 --s2-log-level debug -i balena-cloud-intel-nuc-2.50.1+rev1.dev.img.gz 
```

#### Logging to the target disk that will run balenaOS

There are certain scenarios whereby it is not practical to add an external disk or the device being migrated does not have a secondary disk. E.g migrating a device which in the field remotely.

`takeover` provides a fallback logging mechanism that logs to RAMFS during the migration process and then persists the logs to disk in case of failure or successful migration.
Logs will be at `/mnt/data/fallback_log/fallback.log` by default. 

You can use option `--fallback-log` to enable this mechanism.
Optionally, the following options can be used in conjuction with `--fallback-log`:
- `--fallback-log-filename` : Set the name of the fallback logfile. Default is `fallback.log`
- `--fallback-log-dir` : Set the directory name where fallback log is persisted on data partition. Default is `fallback_log`.
**_Note_**:
- The current mechanism persists the fallback logs from `tmpfs` to the data partition (`/mnt/data`)
- Given that the migration process might be interrupted owing to errors, logs will be persisted on the _old_ os assuming the flashing process failed

### Configuring a Backup

*takeover* can be configured to create a backup that will automatically be converted to volumes once 
balena-os is running on the device. The backup is configured using a file in YAML syntax which is 
made available to takeover using the ```--backup-cfg``` command line option.

**Warning**: Please be aware that the backup file will be stored in RAMFS together with the balena-os image and some other 
files at some point of stage2 takeover processing. 
For this reason the backup size should be restricted to a size that fits into the devices ram leaving ample space. 
*takeover* will fail in stage2 if insufficient ram is found to transfer all files.    
 
     
The backup is grouped into volumes. 
Each volume can be configured to contain a complex directory structure. Volumes correspond to application container 
volumes of the application that is loaded on the device once balena OS is running. 
The balena-supervisor will scan the created backup for volumes declared in the application containers and automatically 
restore the backed up data to the appropriate container volumes. 
The supervisor will delete the backup once this process is terminated. Backup directories with no corresponding volumes 
are not retained. 

Backup volume definitions can contain one or more ```items```. An Item consists of a mandatory ```source``` source path definition
and the following optional fields: 
- ```target``` - an alternative target directory name - if not present the files will be copied to the root of the volume.
- ```filter``` - a regular expression that will be applied to the source path. Only files matching the filter will be copied. 
If no filter is given, all files will be copied.      

*Backup configuration example:*

```yaml
## create a volume test volume 1
- volume: "test volume 1"
 items:
 ## backup all from source and store in target inside the volume  
 - source: /home/thomas/develop/balena.io/support
   target: "target dir 1.1"
 - source: "/home/thomas/develop/balena.io/customer/"
   target: "target dir 1.2"
## create another volume 
- volume: "test volume 2"
 items:
 ## store all files from source that match the filter in target
 - source: "/home/thomas/develop/balena.io/migrate"
   target: "target dir 2.2"
   filter: 'balena-.*'
## store all files from source that match the filter
## in the root of the volume directory
- volume: "test_volume_3"
 items:
  - source: "/home/thomas/develop/balena.io/migrate/migratecfg/init-scripts"
    filter: 'balena-.*'
```

### Working with unsupported scenarios

**Warning**: *Use these options at your own risk.* They allow you to run *takeover* in scenarios that were never tested
by balena. You may have success, but you may also hit serious issues, including rendering your device unbootable. Please
test thoroughly before using these options in production.

There is a huge number of combinations of device types, device models, and operating systems. *takeover* was developed
and tested with a subset of these combinations in mind, and therefore will refuse to run with other combinations. That
said, we do provide the following options to skip or override these checks, so that you can try to use the tool in
different scenarios.

### ```--no-os-check```

With this option, *takeover* will not check if the OS currently running on the device is supported. This allows you to
attempt to migrate away from an unsupported OS (for example, a newly released version of an OS).

### ```--no-dt-check```

Normally, *takeover* performs some checks that are related with the device type and model. For example:

- Is this device model known to work with *takeover*?
- Is this device model compatible with the device type of the target fleet?
- Is this combination of device model and source OS known to work with *takeover*?

Passing in the ```--no-dt-check``` option will skip all these checks. This can be useful to enable migrations that are
technically valid but were not tested. Please be careful to use only compatible architectures. A mistake here can cause
your device to be flashed with an OS for an incompatible architecture, rendering it unbootable!

Notice from the list above that checks for OS compatibility are dependent on the device type, so using this option also
effectively disables the OS checks (similar to what ```--no-os-check``` does).

Here's an example: *takeover* was never officially tested to migrate *Raspberry Pi 3*s to 64-bit balenaOS. So, even
though migrating a Pi 3 to a fleet with device type ```raspberrypi3-64``` would be technically valid, *takeover* will
not allow you to do that -- unless you force it by using

```sh
# sudo ./takeover --no-dt-check [...other options...]
```

### `--change_dt_to`

There are certain scenarios where devices are migrated from one device type to another. E.g From an Intel NUC (`intel-nuc`) to Generic x86_64 (`generic-amd64`). Passing `--change-dt-to` followed by the device type slug will change the device type of the device in balenaCloud.

E.g:

```sh
# sudo ./takeover --change-dt-to generic-amd64 [...other options...]
```

You can find the device type slug for each device type in [our docs](https://docs.balena.io/reference/base-images/devicetypes/) in the `BALENA_MACHINE_NAME` column.

