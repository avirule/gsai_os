#![no_std]
#![no_main]
#![feature(asm)]
#![feature(alloc_error_handler)]

mod drivers;
mod io;

use core::{alloc::Layout, panic::PanicInfo};
use efi_boot::{
    drivers::graphics::{Color8i, ProtocolGraphics},
    entrypoint,
};

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}

#[alloc_error_handler]
fn alloc_error(error: Layout) -> ! {
    loop {}
}

entrypoint!(kernel_main);
fn kernel_main(mut protocol_graphics: ProtocolGraphics) -> i32 {
    loop {
        protocol_graphics.clear(Color8i::new(125, 150, 22), true);
    }
}
