mod frame;
mod page;
mod uefi;

#[cfg(feature = "global_allocator")]
mod galloc;

pub use frame::*;
pub use page::*;
pub use uefi::*;
pub mod falloc;
pub mod malloc;
pub mod mmio;
pub mod paging;

pub const KIBIBYTE: usize = 0x400; // 1024
pub const MIBIBYTE: usize = KIBIBYTE * KIBIBYTE;

pub const fn to_kibibytes(value: usize) -> usize {
    value / KIBIBYTE
}

pub const fn to_mibibytes(value: usize) -> usize {
    value / MIBIBYTE
}

#[macro_export]
macro_rules! alloc {
    ($size:expr) => {
        $crate::alloc!($size, $crate::memory::malloc::get().minimum_alignment()) as *mut _
    };
    ($size:expr, $align:expr) => {
        $crate::memory::malloc::get()
            .alloc(core::alloc::Layout::from_size_align($size, $align).unwrap()) as *mut _
    };
}

#[macro_export]
macro_rules! alloc_to {
    ($frames:expr) => {
        $crate::memory::malloc::get().alloc_to($frames) as *mut _
    };
}
