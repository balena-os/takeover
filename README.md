# takeover

Brownfield device migration using *takeover* strategy.

**Warning**: The *takeover* command will attempt to install balena-os over your existing operating system. 
Make sure you do not accidentally call the command on the wrong host and test your setup before migrating a host.

The easiest way to test your setup is to run *takeover* with the ```--pretend``` option. This will test all stages of
migration except for the actual flashing of the image, rebooting your system in the process.         

## Howto 

Takeover consists of a single executable that supports automatic download of all assets required for migration. 
All that is needed to migrate a device to balena-os is a valid config.json typically obtained from the dashboard of your 
balena application. 

```shell script
> takeover --help

Options:
  -w, --work-dir <DIRECTORY>         Path to working directory
  -i, --image <IMAGE>                Path to balena-os image
  -v, --version <VERSION>            Version of balena-os image to download
  -c, --config <CONFIG_JSON>         Path to balena config.json
      --log-level <LOG_LEVEL>        Set log level, one of [error,warn,info,debug,trace] [default: info]
      --log-file <LOG_FILE>          Set stage1 log file name
      --backup-cfg <BACKUP-CONFIG>   Backup configuration file
      --s2-log-level <S2_LOG_LEVEL>  Set stage2 log level, one of [error,warn,info,debug,trace]
      --no-ack                       Scripted mode - no interactive acknowledgement of takeover
      --pretend                      Pretend mode, do not flash device
      --stage2                       Internal - stage2 invocation
      --tar-internal                 Use internal tar instead of external command
      --no-cleanup                   Debug - do not cleanup after stage1 failure
      --no-os-check                  Do not check if OS is supported
      --no-dt-check                  Do not check if the target device type is valid
      --no-api-check                 Do not check if balena API is available
      --no-vpn-check                 Do not check if balena VPN is available
      --no-efi-setup                 Do not setup EFI boot
      --no-nwmgr-check               Do not check network manager files exist
      --no-keep-name                 Do not migrate host-name
  -d, --download-only                Download image only, do not check device and migrate
      --check-timeout <TIMEOUT>      API/VPN check timeout in seconds.
  -l, --log-to <LOG_DEVICE>          Write stage2 log to LOG_DEVICE
  -f, --flash-to <INSTALL_DEVICE>    Use INSTALL_DEVICE to flash balena to
      --no-wifis                     Do not create network manager configurations for configured wifis
      --wifi <SSID>                  Create a network manager configuration for configured wifi with SSID
      --nwmgr-cfg <NWMGR_FILE>       Supply a network manager file to inject into balena-os
      --change-dt-to <DT_SLUG>       Device Type slug to change to
  -h, --help                         Print help
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
You can modify the stage1 log-level by using the ```--log-level``` option. Available log levels 
are *error*, *warn*, *info*, *debug*, and *trace*. 
Stage1 is the first part of migration - mainly the preparation of the migration process. Everything happening in stage1 
can be logged to the console.
 
At the end of stage1 *takeover* switches the file system root to a RAMFS file system and replaces the init process. 
This part of migration is called stage2. In stage2 the console does not receive output from *takeover* any more and 
ssh-sessions will usually be disconnected. 
Logging to the harddisk does not make sense, as that device will be overwritten with balena-os during the migration process. 
For this reason you can specify a log device using the ```-l / --log-to``` option. 
You should use a device that is independent from the disk that balena will be installed on. Usually a secondary disk 
or a USB stick works well. The log device should be formatted with a *vfat*, *ext3* or *ext4* file system.
It also makes sense to adapt the stage2 log level to see a maximum of information. This can be done using the 
```-s / --s2-log-level``` option. Log levels are as given above. 

Example, writing a stage2 log to /dev/sda1 with stage2 log level *debug*:
```shell script
sudo ./takeover -c config.json -l /dev/sda1 --s2-log-level debug -i balena-cloud-intel-nuc-2.50.1+rev1.dev.img.gz 
```

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

There are certain scenarios where devices are migrated from one device type to another. E.g From an Intel NUC(`intel-nuc`) to Generic x86_64 (`generic-amd64`). Passing `--change-dt-to` followed by the device type slug will change the device type of the device in balenaCloud.

E.g:

```sh
# sudo ./takeover --change-dt-to generic-amd64 [...other options...]
```

You can find the device type slug for each device type in [our docs](https://docs.balena.io/reference/base-images/devicetypes/) in the `BALENA_MACHINE_NAME` name.

## Compiling *takeover*

First off, if you are an end user you probably don't need to compile *takeover* yourself. Just visit our [releases page
on Github](https://github.com/balena-os/takeover/releases) and download a precompiled binary for the desired
architecture.

That said, if you want to build *takeover*, the easiest route (and the one we recommend!) is to use Rust's [`cross`
tool](https://github.com/cross-rs/cross). `cross` (along with the `Cross.toml` file we provide) will take care of the
two main technicalities of the compilation:

1. You need to compile for the architecture of the device you want to migrate away from.
2. You need to compile to a statically linked binary.

Once you have `cross` and its dependencies installed (see [instructions
here](https://github.com/cross-rs/cross?tab=readme-ov-file#dependencies)), compiling *takeover* should be just a matter
of running `cross build` passing the desired target platform. Here are some examples:

```shell
# To build a version of takeover that will run on an ARM device running a 32-bit
# operating system. This would be the typical case for a Raspberry Pi running a
# 32-bit version of Raspberry Pi OS.
cross build --release --target "armv7-unknown-linux-musleabihf"

# For an ARM device running a 64-bit operating system. Typical for a Raspberry
# Pi running a 64-bit OS.
cross build --release --target "aarch64-unknown-linux-musl"

# For a device with an Intel CPU running a 64-bit operating system.
cross build --release --target "x86_64-unknown-linux-musl"
```

## How it works

The takeover process occurs in 2 stages, namely:

- Stage1: Gathers information about the device and the current operating system and prepares the system for Stage2
- Stage2: the `takeover` process is re-run as `init` (PID 1) and spawns a worker process that kills running processes, copies required files to RAMFS, unmounts partitions and handles the flashing of balenaOS

````mermaid

flowchart TD
    A(Start) --> B[[Stage1]]
    B --> C[[Stage2]]
    C--> E(End)

````

### Stage1
---

````mermaid

flowchart TB
    A(Start)
    subgraph S1 [Stage1]
    direction TB
        S1A[[Gather Migration Info]]
        S1B[[Prepare for takeover]]
    S1A --> S1B    
    end
    A --> S1
    S1 --> C[[Stage2]]
    C--> E(End)

````

Stage1 consists of 2 main processes:

#### 1. Gather Migration Info

- Check device type (can be skipped by passing option `--no_dt_check`)
- Read `config.json`
- Run checks
  - check if device type set in `config.json` is supported by the detected device type
  - check if can connect to API endpoint set in  `config.json`
  - check if can connect to VPN endpoint set in `config.json`
- Download latest balenaOS image if an image is not provided
- Create corresponding network manager connection files
- Backup files if required
- Replicate `hostname` if required

#### 2. Prepare for takeover
   
- Disable swap
- Copy files/binaries to RAMFS
- Setup new init process (`takeover` is bind-mounted over original `init` executable )
- Setup Stage2 log device if required
- Write Stage2 config file
- Restart init daemon -> since `takeover` is bind-mounted over `init`, `takeover` is actually ran as the init process (PID 1)

### Stage2
---

````mermaid
flowchart TB
    A(Start)-->S1[[Stage1]]
    subgraph S2 [Stage2]
    direction TB
        S2A[[Run as Init]]
        S2B[[Stage2 Worker]]
    S2A --> S2B    
    end
S1 --> S2
S2 --> E(End)
````
Stage2 also consists of 2 main processes:

#### Run as Init

- Close open files
- Setup stage2 logging to an external device
- Set mount propagation to private for rootfs -> mounts and unmounts within this mount point will not propagate to other mount points, and mounts and unmounts in other mount points will not propagate to this mount point. This effectively isolates the mount point from changes in other namespaces.
- change root filesystem via `pivot_root`
- Spawn Stage2 worker process

#### Stage2 Migration Worker
- setup Stage2 logging to external device if configured
- Kill running processes
- Copy required files to RAMFS
- unmount partitions
- Flash balenaOS image to disk
- Validate if image was written successfully
- Transfer files to respective destinations (`config.json`, system connection files)
- Setup EFI if required
- Restore backup files is required 
