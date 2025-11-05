This document describes development topics for takeover, including compilation and a description of how takeover works, detailing the steps taken by the process.

# Compiling _takeover_

The easiest route to build the takeover executable is Rust's [cross](https://github.com/cross-rs/cross) tool, which generates a single executable for a target's system's architecture.

See the [instructions](https://github.com/cross-rs/cross?tab=readme-ov-file#dependencies) to install `cross`. , compiling *takeover* should be just a matter
of running `cross build` passing the desired target platform. Here are some examples:

```shell
# 64-bit ARM device, like a Raspberry Pi 4
cross build --release --target "aarch64-unknown-linux-musl"

# 32-bit ARM device, like a Raspberry Pi 3
cross build --release --target "armv7-unknown-linux-musleabihf"

# x86 device, like an Intel NUC
cross build --release --target "x86_64-unknown-linux-musl"
```

# How _takeover_ works

The takeover process occurs in 2 stages:

1. Gathers information about the device and the current operating system and prepares the system for Stage 2

1. Re-runs the `takeover` process as `init` (PID 1) and spawns a worker process that kills running processes, copies required files to RAMFS, unmounts partitions and handles the flashing of balenaOS

````mermaid

flowchart TD
    A(Start) --> B[[Stage1]]
    B --> C[[Stage2]]
    C--> E(End)

````

### Stage 1
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

Stage 1 initially gathers information based on the provided options, then it prepares to run Stage 2.

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

### Stage 2
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

Stage 2 restarts takeover as a transitional init process to setup the new root filesystem. It then spawns a separate worker process to perform the actual migration.

#### Run as Init

- Close open files
- Setup stage 2 logging to an external device
- Set mount propagation to private for rootfs -> mounts and unmounts within this mount point will not propagate to other mount points, and mounts and unmounts in other mount points will not propagate to this mount point. This effectively isolates the mount point from changes in other namespaces.
- Change root filesystem via `pivot_root`
- Spawn Stage 2 worker process

#### Stage 2 Migration Worker
- Setup Stage 2 logging to external device if configured
- Kill running processes
- Copy required files to RAMFS
- Unmount partitions
- Flash balenaOS image to disk
- Validate if image was written successfully
- Transfer files to respective destinations (`config.json`, system connection files)
- Setup EFI if required
- Restore backup files is required 
