# takeover

Brownfield device migration using *takeover* strategy.

**PLEASE NOTE**: This repo is currenly deprecated and under limited maintenance. We are working on a new migrator for Linux and potentially Windows devices. If you have a usecase for the migrator and need support or have additional questions, please contact solutions@balena.io.  


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
takeover 0.1.1
Thomas Runte <thomasr@balena.io>


USAGE:
    takeover [FLAGS] [OPTIONS]

FLAGS:
    -d, --download-only     Download image only, do not check device and migrate
    -h, --help              Prints help information
        --no-ack            Scripted mode - no interactive acknoledgement of takeover
        --no-api-check      Do not check if balena API is available
        --no-cleanup        Debug - do not cleanup after stage1 failure
        --no-efi-setup      Do not setup EFI boot
        --no-keep-name      Do not migrate host-name
        --no-nwmgr-check    Do not check network manager files exist
        --no-os-check       Do not check if OS is supported
        --no-vpn-check      Do not check if balena VPN is available
        --no-wifis          Do not create network manager configurations for configured wifis
        --pretend           Pretend mode, do not flash device
        --stage2            Internal - stage2 invocation
        --tar-internal      Use internal tar instead of external command

OPTIONS:
        --backup-cfg <BACKUP-CONFIG>     Backup configuration file
        --check-timeout <TIMEOUT>        API/VPN check timeout in seconds.
    -c, --config <CONFIG_JSON>           Path to balena config.json
    -f, --flash-to <INSTALL_DEVICE>      Use INSTALL_DEVICE to flash balena to
    -i, --image <IMAGE>                  Path to balena-os image
        --log-file <LOG_FILE>            Set stage1 log file name
        --log-level <log-level>          Set log level, one of [error,warn,info,debug,trace] [default: info]
    -l, --log-to <LOG_DEVICE>            Write stage2 log to LOG_DEVICE
        --nwmgr-cfg <NWMGR_FILE>...      Supply a network manager file to inject into balena-os
        --s2-log-level <s2-log-level>    Set stage2 log level, one of [error,warn,info,debug,trace]
    -v, --version <VERSION>              Version of balena-os image to download
        --wifi <SSID>...                 Create a network manager configuation for configured wifi with SSID
    -w, --work-dir <DIRECTORY>           Path to working directory%                                                                              
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
 
The above command will download the latest production image for your platform and migrate the device to balena. 

Several options are availble to cover special situations: 

### Image Selection

The *takeover* command allows you to specify a balena-os version for download or an image to use for migration.

#### Downloading an image

By default *takeover* will download the latest production image for the platform specified in your config.json. 
If you need a development image or a version different from the latest you can use the ```--version``` option to specify 
a version. 
The ```--version``` option accepts either a full image name (eg. ```--version 2.50.1+rev1.dev```) or parsing 
of ~x.y.z and ^x.y.z requirements as defined at [semver](https://www.npmjs.com/package/semver)
 (eg. ```--version ~2.48```).
 Example: 
 ```shell script
./sudo takeover -c config.json --version 2.50.1+rev1.dev
```
   
When downloading images,  certain platforms (mainly intel-nuc, Generic-x86_64, beaglebone) require unpacking the image and 
extracting the actual OS-image. The *takeover* command does this automatically but the process of unpacking temporarilly 
requires up to 2.3GB of disk space. You can use the --work-dir option to specify a working directory that has sufficient 
disk space (eg. a memory stick) to unpack if your current directory does not. Otherwise you can use *takeover* 
on a computer with sufficient diskspace to download the image, copy it to the target device and use the 
```-i / --image``` as described below.

The ```-d / --download-only``` option allows you to download an image without installing it. This option also 
disables most checks, so that you can download an image e.g. for your RaspberryPI 3 using your X86 PC. 
All you need to do is use a config.json for a raspberry PI and the ```-d``` option.

Example - Dowload only of a balena OS image: 
```shell script
sudo ./takeover -d --version 2.50.1+rev1.dev -c config.json 
```
 

#### Specifying an existing image

You can use the ```-i / --image``` option to specify any valid balena-os image. 

**Warning:** Please be aware that specifying an invalid 
image might lead to your target device being flashed with something invalid which will very likely lead to it not booting. 

Be carefull with images you have downloaded from the [balena dashboard](https://balena.io). These images are zip encoded 
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
migrating a configuration that will not be able to come online. This check can be overridden by specifyng the 
```--np-nwmgr-check``` option. 
   
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
You should use a device that is independant from the disk that balena will be installed on. Usually a secondary disk 
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
made available to takeover using the ```--backup-cfg``` comand line option.

**Warning**: Plaese be aware that the backup file will be stored in RAMFS together with the balena-os image and some other 
files at some point of stage2 takeover processing. 
For this reason the backup size should be restricted to a size that fits into the devices ram leaving ample space. 
*takeover* will fail in stage2 if unsufficient ram is found to transfer all files.    
 
     
The backup is grouped into volumes. 
Each volume can be configured to contain a complex directory structure. Volumes correspond to application container 
volumes of the application that is loaded on the device once balena OS is running. 
The balena-supervisor will scan the created backup for volumes declared in the application containers and automatically 
restore the backed up data to the appropriate container volumes. 
The supervisor will delete the backup once this process is terminated. Backup directories with no corresponding volumes 
are not retained. 

Backup volume definitions can contain one or more ```items```. An Item consists of a mandatory ```source``` source path definition
and the following optionial fields: 
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

    
## Compiling takeover

*takeover* needs to be compiled for the target platform. For Raspberry PI & beaglebone devices that is *armv7* and 
for *intel-nuc* and *Generic X86-64* that is the X86-64 platform. 

Cross compiling takeover is easiest done using the  [rust-embedded cross](https://github.com/rust-embedded/cross) 
cross compilation tools.  
After installing cross and the appropriate targets for the target platform *takeover* 
can be cross-compiled using
```shell script
cross build --target <target-tripple> --release 
```   
For arm v7 devices this could be 
```
cross build --release --target "armv7-unknown-linux-gnueabihf"
```
or 
```
cross build --release --target "armv7-unknown-linux-musleabihf"
```

   
