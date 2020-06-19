# takeover

Brownfield device migration using takeover strategy.

## Howto 

Takeover consists of a single executable that contains or allows download of all assets required for migration.

```shell script
> takeover --help
takeover 0.1.0
Thomas Runte <thomasr@balena.io>


USAGE:
    takeover [FLAGS] [OPTIONS]

FLAGS:
        --build-num         print build num and exit
    -h, --help              Prints help information
        --init              Internal - init process invocation
        --no-api-check      Do not check if balena API is available
        --no-cleanup        Debug - do not cleanup after stage1 failure
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
        --log-level <log-level>          Set log level, one of [error,warn,info,debug,trace] [default: info]
    -l, --log-to <LOG_DEVICE>            Write stage2 log to LOG_DEVICE
        --nwmgr-cfg <NWMGR_FILE>...      Supply a network manager file to inject into balena-os
    -s, --s2-log-level <s2-log-level>    Set stage2 log level, one of [error,warn,info,debug,trace]
    -v, --version <VERSION>              Version of balena-os image to download
        --wifi <SSID>...                 Create a network manager configuation for configured wifi with SSID
    -w, --work-dir <DIRECTORY>           Path to working directory

```   

All that is needed to migrate a device to balena-os is a valid config.json typically obtained from the dashboard of your 
balena application. 

This can be done by directing your browser to the [balena dashboard](balena.io, https://balena.io), logging in to to your user 
account and selecting the application you want to migrate the device to. From there you can press the 'add device' button 
in the top left and in the 'Add new device' dialog select 'Advanced' and  'Download configuration file only'. 

In the most simple case all you need to do now is copy the takeover executable and the config.json to a folder on the 
device you would like to migrate and execute 
```
sudo ./takeover -c config.json
``` 
on the command line.
 
The above command will download the latest production image for your platform and migrate the device to balena. 

Several options are availble to coverspecial situations: 

### Image Selection


The takeover command allows you to specify a balena-os version for download or an image to use for migration.

#### Downloading an image

By default takeover will download the latest production image for the platform specified in your config.json. 
If you need a development image or a version different from the latest you can use the ```--version``` option to specify 
a version. 
The ```--version``` option accepts either a full image name (eg. ```--version 2.50.1+rev1.dev```) or parsing 
of ~x.y.z and ^x.y.z requirements as defined at [https://www.npmjs.com/package/semver](https://www.npmjs.com/package/semver)
 (eg. ```--version ~2.48```).
 Example: 
 ```shell script
./sudo takeover -c config.json --version 2.50.1+rev1.dev
```
   
When downloading images,  certain platforms (mainly intel-nuc, Generic-x86_64, beaglebone) require unpacking the image and 
extracting the actual OS-image. The takeover command does this automatically but the process of unpacking temporarilly 
requires up to 2GB of disk space. You can use the --work-dir option to specify a working directory that has sufficient 
disk space (eg. a memory stick) to unpack if your current directory does not. Otherwise you can use takeover 
on a computer with sufficient diskspace to download the image, copy it to the target device and use the 
```--image``` option to specify a specific image: 
```shell script
./sudo takeover -c config.json -i <your balean os image>
``` 

#### Specifying an existing image

You can use the ```--image``` option to specify any valid balena-os image. Please be aware that specifying an invalid 
image might lead to your target device being flashed with something invalid which will very likely lead to it not booting. 

Be carefull with images you have downloaded from the [balena dashboard](https://balena.io). These images are zip encoded 
and need to be unpacked and recompressed using gzip as follows: 
```shell script
unzip <image-name>
gzip <unpacked image name>
```  
For certain device types (mainly intel-nuc., Generic x86_64, beaglebone) the image downloaded will be a flasher image
that contains the actual balena-os image. For these platforms it is easier to let takeover do the download and extraction. 
     
  
### Network Setup


 
   
  
 
  
