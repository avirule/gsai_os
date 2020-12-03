all: image/EFI/BOOT/BOOTX64.efi image/EFI/gsai/kernel.elf

image/EFI/BOOT/BOOTX64.efi:
	cd /media/carl/GitHub/gsai/efi-boot/;\
		cargo build --release --target x86_64-unknown-uefi -Z unstable-options

image/EFI/gsai/kernel.elf:
	cd /media/carl/GitHub/gsai/kernel/;\
		cargo build --release -Z unstable-options;\
		mv ../image/EFI/gsai/kernel ../image/EFI/gsai/kernel.elf