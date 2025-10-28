## Raw disk image vs. Flasher image
A raw disk image is copied onto the storage medium where it then can boot directly, for example an SD card for a Raspberry Pi 4. In contrast a flasher balenaOS image actually _contains_ the internal raw disk image plus an essential script. This script runs automatically on boot to prepare and flash the actual raw disk image to the storage medium. For example, generally balenaOS is installed from a flasher image via a USB drive to an x86 device.

For device types that typically use a flasher image, the takeover process itself performs the essential preparation for the raw disk image.
