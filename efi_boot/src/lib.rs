#![no_std]
#![feature(abi_efiapi)]
#![feature(core_intrinsics)]

pub use uefi::{
    table::{
        boot::{MemoryDescriptor, MemoryType},
        Runtime, SystemTable,
    },
    Status,
};

pub const KERNEL_CODE: MemoryType = MemoryType::custom(0xFFFFFF00);
pub const KERNEL_DATA: MemoryType = MemoryType::custom(0xFFFFFF01);

#[repr(C)]
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct Size {
    pub width: usize,
    pub height: usize,
}

// this is used to construct a FramebufferDriver from the kernel
#[repr(C)]
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct FramebufferPointer {
    pub pointer: *mut u8,
    pub size: Size,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FFIOption<T> {
    None,
    Some(T),
}

impl<T> Into<Option<T>> for FFIOption<T> {
    fn into(self) -> Option<T> {
        match self {
            FFIOption::Some(some) => Some(some),
            FFIOption::None => None,
        }
    }
}

#[repr(C)]
pub struct BootInfo<'info, 'item> {
    memory_map: &'info mut dyn ExactSizeIterator<Item = &'item MemoryDescriptor>,
    runtime_table: SystemTable<Runtime>,
    framebuffer: FFIOption<FramebufferPointer>,
}

impl<'info, 'item> BootInfo<'info, 'item> {
    pub fn new(
        memory_map: &'info mut impl ExactSizeIterator<Item = &'item MemoryDescriptor>,
        runtime_table: SystemTable<Runtime>,
        framebuffer: Option<FramebufferPointer>,
    ) -> Self {
        Self {
            memory_map,
            runtime_table,
            framebuffer: match framebuffer {
                Some(some) => FFIOption::Some(some),
                None => FFIOption::None,
            },
        }
    }

    pub fn memory_map(&mut self) -> &mut dyn ExactSizeIterator<Item = &'item MemoryDescriptor> {
        self.memory_map
    }

    pub fn runtime_table(&self) -> &SystemTable<Runtime> {
        &self.runtime_table
    }

    pub fn framebuffer_pointer(&self) -> Option<FramebufferPointer> {
        self.framebuffer.into()
    }
}

pub type KernelMain = extern "win64" fn(crate::BootInfo) -> Status;

#[macro_export]
macro_rules! entrypoint {
    ($path:path) => {
        #[export_name = "_start"]
        pub extern "win64" fn __impl_kernel_main(boot_info: $crate::BootInfo) -> $crate::Status {
            let function: $crate::KernelMain = $path;
            function(boot_info)
        }
    };
}
