# takeover

Brownfield device migration using *takeover* strategy.

**Warning**: The *takeover* command will attempt to install balena-os over your existing operating system. 
Make sure you do not accidentally call the command on the wrong host and test your setup before migrating a host.

The easiest way to test your setup is to run *takeover* with the ```--pretend``` option. This will test all stages of
migration except for the actual flashing of the image, rebooting your system in the process.         

## Howto 

Takeover consists of a single executable that contains or allows download of all assets required for migration. 
All that is needed to migrate a device to balena-os is a valid config.json typically obtained from the dashboard of your 
balena application. 


```shell script
> takeover --help
takeover 0.1.1
Thomas Runte <thomasr@balena.io>


USAGE:
    takeover [FLAGS] [OPTIONS]

FLAGS:
        --build-num         Debug - print build num and exit
    -d, --download-only     Download image only, do not check device and migrate
    -h, --help              Prints help information
        --init              Internal - init process invocation
        --no-ack            Scripted mode - no interactive acknoledgement of takeover
        --no-api-check      Do not check if balena API is available
        --no-cleanup        Debug - do not cleanup after stage1 failure
        --no-fail-on-efi    Do not fail if EFI setup fails
        --no-keep-name      Do not migrate host-name
        --no-nwmgr-check    Do not check network manager files exist
        --no-os-check       Do not check if OS is supported
        --no-vpn-check      Do not check if balena VPN is available
        --no-wifis          Do not create network manager configurations for configured wifis
        --pretend           Pretend mode, do not flash device
        --stage2            Internal - stage2 invocation

OPTIONS:
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
    -w, --work-dir <DIRECTORY>           Path to working directory
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
sudo ./takeover -d --version --version 2.50.1+rev1.dev -c config.json 
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
By default *takeover* runs at *info* log level. It will log to the console and to a logfile in the current directory 
called ```stage1.log```. You can modify the stage1 log-level by using the ```--log-level``` option. Available log levels 
are *error*, *warn*, *info*, *debug*, and *trace*.
Stage1 is the first part of migration - mainly the preparation of the migration process. Everything happening in stage1 
can be logged to the console.
 
At the end of stage1 *takeover* switches the file system root to a RAMFS file system and replaces the init process. 
This part of migration is called stage2. In stage2 the console does not receive output from *takeover* any more and 
ssh-sessions will usually be disconnected. 
Logging to the harddisk does not make sense, as that device will be overwritten with balena-os during the migration process. 
For this reason you can specify a log device using the ```-l / --log-to``` option. 
You should use a device that is independant from the disk that balena will be installed on. Usually a secondary disk 
or a USB stick works well. The log device should be formatted with a *FAT32* or *ext4* file system.
It also makes sense to adapt the stage2 log level to see a maximum of information. This can be done using the 
```-s / --s2-log-level``` option. Log levels are as given above. 

Example, writing a stage2 log to /dev/sda1 with stage2 log level *debug*:
```shell script
sudo ./takeover -c config.json -l /dev/sda1 --s2-log-level debug -i balena-cloud-intel-nuc-2.50.1+rev1.dev.img.gz 
```
    
## Compiling takeover

*takeover* needs to be compiled for the target platform. For Raspberry PI & beaglebone devices that is *armv7* and 
for *intel-nuc* and *Generic X86-64* that is the X86-64 platform. 

In stage2 *takeover* runs in a self created root file 
system running on RAMFS. To minimize the amount of data that needs to be copied to this filesystem, this part 
of *takeover* uses a busybox cmd environment and no system libraries are copied. To be self-contained *takeover* has to 
be statically linked against  *libmusl*. 

This can be done using the  [rust-embedded cross](https://github.com/rust-embedded/cross) cross compilation tools.  
After installing cross and the *x86_64-unknown-linux-musl* and *armv7-unknown-linux-musleabihf* targets *takeover* 
can be cross-compiled using
```shell script
cross build --target=armv7-unknown-linux-musleabihf --release 
```   
for the armv7 platform or 
```shell script
cross build --target=x86_64-unknown-linux-musl --release 
```   
for the X86-64 platform. 

   
