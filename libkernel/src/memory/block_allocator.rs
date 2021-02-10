use crate::{
    align_up_div,
    memory::{paging::VirtualAddressor, Frame, FrameIterator, Page},
    SYSTEM_SLICE_SIZE,
};
use alloc::vec::Vec;
use spin::{Mutex, RwLock};

/// Represents one page worth of memory blocks (i.e. 4096 bytes in blocks).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct BlockPage {
    blocks: [u64; 4],
}

impl BlockPage {
    /// Number of sections (primitive used to track blocks with its bits).
    const SECTIONS_COUNT: usize = 4;
    /// Number of blocks each block page contains.
    const BLOCKS_COUNT: usize = Self::SECTIONS_COUNT * 64;

    /// An empty block page (all blocks zeroed).
    const fn empty() -> Self {
        Self {
            blocks: [0u64; Self::SECTIONS_COUNT],
        }
    }

    /// Whether the block page is empty.
    pub const fn is_empty(&self) -> bool {
        (self.blocks[0] == 0)
            && (self.blocks[1] == 0)
            && (self.blocks[2] == 0)
            && (self.blocks[3] == 0)
    }

    /// Whether the block page is full.
    pub const fn is_full(&self) -> bool {
        (self.blocks[0] == u64::MAX)
            && (self.blocks[1] == u64::MAX)
            && (self.blocks[2] == u64::MAX)
            && (self.blocks[3] == u64::MAX)
    }

    /// Unset all of the block page's blocks.
    pub const fn set_empty(&mut self) {
        self.blocks[0] = 0;
        self.blocks[1] = 0;
        self.blocks[2] = 0;
        self.blocks[3] = 0;
    }

    /// Set all of the block page's blocks.
    pub const fn set_full(&mut self) {
        self.blocks[0] = u64::MAX;
        self.blocks[1] = u64::MAX;
        self.blocks[2] = u64::MAX;
        self.blocks[3] = u64::MAX;
    }

    /// Underlying section iterator.
    fn iter(&self) -> core::slice::Iter<u64> {
        self.blocks.iter()
    }

    /// Underlying mutable section iterator.
    fn iter_mut(&mut self) -> core::slice::IterMut<u64> {
        self.blocks.iter_mut()
    }
}

/// Allows tracking the state of the current block page's section
///  in a loop, so a block page's underlying global memory can be
///  allocated or deallocated accordingly.
#[derive(Debug, Clone, Copy)]
struct SectionState {
    had_bits: bool,
    has_bits: bool,
}

impl SectionState {
    /// An empty section state.
    const fn empty() -> Self {
        Self {
            had_bits: false,
            has_bits: false,
        }
    }

    /// Whether the section state indicates an empty section.
    const fn is_empty(&self) -> bool {
        !self.had_bits && !self.has_bits
    }

    /// Whether the section states indicates a section that should be allocated.
    const fn is_alloc(&self) -> bool {
        !self.had_bits && self.has_bits
    }

    /// Whether the section states indicates a section that should be deallocated.
    const fn is_dealloc(&self) -> bool {
        self.had_bits && !self.has_bits
    }

    /// Whether the given block page section states indicate an allocation.
    fn should_alloc(page_state: &[SectionState]) -> bool {
        page_state.iter().any(|state| state.is_alloc())
            && page_state
                .iter()
                .all(|state| state.is_alloc() || state.is_empty())
    }

    /// Whether the given block page section states indicate an deallocation.
    fn should_dealloc(page_state: &[SectionState]) -> bool {
        page_state.iter().any(|state| state.is_dealloc())
            && page_state
                .iter()
                .all(|state| state.is_dealloc() || state.is_empty())
    }
}

/// Allocator utilizing blocks of memory, in size of 16 bytes per block, to
///  easily and efficiently allocate.
pub struct BlockAllocator {
    addressor: Mutex<core::lazy::OnceCell<VirtualAddressor>>,
    map: RwLock<Vec<BlockPage>>,
}

impl BlockAllocator {
    /// The size of an allocator block.
    const BLOCK_SIZE: usize = 16;

    /// Base page the allocator uses to store the internal block page map.
    const ALLOCATOR_BASE: Page = Page::from_addr(x86_64::VirtAddr::new_truncate(
        (SYSTEM_SLICE_SIZE as u64) * 0xA,
    ));

    /// Provides a simple mechanism in which the mask of a u64 can be acquired by bit count.
    const MASK_MAP: [u64; 64] = [
        0x1,
        0x3,
        0x7,
        0xF,
        0x1F,
        0x3F,
        0x7F,
        0xFF,
        0x1FF,
        0x3FF,
        0x7FF,
        0xFFF,
        0x1FFF,
        0x3FFF,
        0x7FFF,
        0xFFFF,
        0x1FFFF,
        0x3FFFF,
        0x7FFFF,
        0xFFFFF,
        0x1FFFFF,
        0x3FFFFF,
        0x7FFFFF,
        0xFFFFFF,
        0x1FFFFFF,
        0x3FFFFFF,
        0x7FFFFFF,
        0xFFFFFFF,
        0x1FFFFFFF,
        0x3FFFFFFF,
        0x7FFFFFFF,
        0xFFFFFFFF,
        0x1FFFFFFFF,
        0x3FFFFFFFF,
        0x7FFFFFFFF,
        0xFFFFFFFFF,
        0x1FFFFFFFFF,
        0x3FFFFFFFFF,
        0x7FFFFFFFFF,
        0xFFFFFFFFFF,
        0x1FFFFFFFFFF,
        0x3FFFFFFFFFF,
        0x7FFFFFFFFFF,
        0xFFFFFFFFFFF,
        0x1FFFFFFFFFFF,
        0x3FFFFFFFFFFF,
        0x7FFFFFFFFFFF,
        0xFFFFFFFFFFFF,
        0x1FFFFFFFFFFFF,
        0x3FFFFFFFFFFFF,
        0x7FFFFFFFFFFFF,
        0xFFFFFFFFFFFFF,
        0x1FFFFFFFFFFFFF,
        0x3FFFFFFFFFFFFF,
        0x7FFFFFFFFFFFFF,
        0xFFFFFFFFFFFFFF,
        0x1FFFFFFFFFFFFFF,
        0x3FFFFFFFFFFFFFF,
        0x7FFFFFFFFFFFFFF,
        0xFFFFFFFFFFFFFFF,
        0x1FFFFFFFFFFFFFFF,
        0x3FFFFFFFFFFFFFFF,
        0x7FFFFFFFFFFFFFFF,
        0xFFFFFFFFFFFFFFFF,
    ];

    pub const fn new() -> Self {
        Self {
            addressor: Mutex::new(core::lazy::OnceCell::new()),
            map: RwLock::new(Vec::new()),
        }
    }

    fn with_addressor<F, R>(&self, mut closure: F) -> R
    where
        F: FnMut(&mut VirtualAddressor) -> R,
    {
        if let Some(addressor) = self.addressor.lock().get_mut() {
            closure(addressor)
        } else {
            panic!("addressor has not been set")
        }
    }

    /* INITIALIZATION */

    pub fn init(&self, memory_map: &[crate::memory::UEFIMemoryDescriptor]) {
        debug!("Initializing global memory and mapping all frame allocator frames.");
        // TODO do this global memory init in a static / global context
        //  (allocators can't be considered global from the system's perspective)
        let global_memory_frames = unsafe { crate::memory::init_global_memory(memory_map) };

        unsafe {
            self.new_addressor();
            self.init_map();
        }

        let stack_descriptor = crate::memory::find_stack_descriptor(memory_map)
            .expect("failed to find stack memory region");

        self.with_addressor(|addressor| {
            global_memory_frames.for_each(|frame| addressor.identity_map(&frame));

            debug!("Temporary identity mapping stack frames.");
            for frame in stack_descriptor.frame_iter() {
                // This is a temporary identity mapping, purely
                //  so `rsp` isn't invalid after we swap the PML4.
                addressor.identity_map(&frame);
                unsafe { crate::memory::global_reserve(&frame) };
            }

            debug!("Identity mapping all reserved memory blocks.");
            for frame in memory_map
                .iter()
                .filter(|descriptor| crate::memory::is_uefi_reserved_memory_type(descriptor.ty))
                .flat_map(|descriptor| {
                    Frame::range_count(descriptor.phys_start, descriptor.page_count as usize)
                })
            {
                addressor.identity_map(&frame);
            }

            // Since we're using physical offset mapping for our page table modification
            //  strategy, the memory needs to be identity mapped at the correct offset.
            let phys_mapping_addr = crate::memory::global_top_offset();
            debug!("Mapping physical memory at offset: {:?}", phys_mapping_addr);
            addressor.modify_mapped_page(Page::from_addr(phys_mapping_addr));

            unsafe {
                // Swap the PML4 into CR3
                debug!("Writing kernel addressor's PML4 to the CR3 register.");
                addressor.swap_into();
            }
        });

        // 'Allocate' the null page
        trace!("Allocating null frame.");
        self.identity_map(&Frame::null());

        // 'Allocate' reserved memory
        trace!("Allocating reserved frames.");
        memory_map
            .iter()
            .filter(|descriptor| crate::memory::is_uefi_reserved_memory_type(descriptor.ty))
            .flat_map(|descriptor| {
                Frame::range_count(descriptor.phys_start, descriptor.page_count as usize)
            })
            .for_each(|frame| self.identity_map(&frame));

        self.alloc_stack_mapping(stack_descriptor);
    }

    unsafe fn new_addressor(&self) {
        if self
            .addressor
            .lock()
            .set(VirtualAddressor::new(Page::null()))
            .is_err()
        {
            panic!("addressor has already been set for allocator");
        }
    }

    unsafe fn init_map(&self) {
        *self.map.write() = Vec::from_raw_parts(
            Self::ALLOCATOR_BASE.mut_ptr(),
            0,
            SYSTEM_SLICE_SIZE / core::mem::size_of::<BlockPage>(),
        );
    }

    fn alloc_stack_mapping(&self, stack_descriptor: &crate::memory::UEFIMemoryDescriptor) {
        stack_descriptor
            .frame_iter()
            .for_each(|frame| self.identity_map(&frame));

        // adjust the stack pointer to our new stack
        unsafe {
            let cur_stack_base = stack_descriptor.phys_start.as_u64();
            let stack_ptr = self.alloc_to(stack_descriptor.frame_iter()) as u64;

            if cur_stack_base > stack_ptr {
                crate::registers::stack::RSP::sub(cur_stack_base - stack_ptr);
            } else {
                crate::registers::stack::RSP::add(stack_ptr - cur_stack_base);
            }
        }

        // unmap the old stack mappings
        self.with_addressor(|addressor| {
            stack_descriptor
                .frame_iter()
                .for_each(|frame| addressor.unmap(&Page::from_index(frame.index())))
        });
    }

    /* ALLOC & DEALLOC */

    fn raw_alloc(&self, size: usize) -> *mut u8 {
        trace!("Allocation requested: {} bytes", size);

        let size_in_blocks = (size + (Self::BLOCK_SIZE - 1)) / Self::BLOCK_SIZE;
        let (mut block_index, mut current_run);

        while {
            block_index = 0;
            current_run = 0;

            'outer: for block_page in self.map.read().iter() {
                if block_page.is_full() {
                    current_run = 0;
                    block_index += BlockPage::BLOCKS_COUNT;
                } else {
                    for block_section in block_page.iter().map(|section| *section) {
                        if block_section == u64::MAX {
                            current_run = 0;
                            block_index += 64;
                        } else {
                            for bit in (0..64).map(|shift| (block_section & (1 << shift)) > 0) {
                                if bit {
                                    current_run = 0;
                                } else {
                                    current_run += 1;
                                }

                                block_index += 1;

                                if current_run == size_in_blocks {
                                    break 'outer;
                                }
                            }
                        }
                    }
                }
            }

            current_run < size_in_blocks
        } {
            self.grow(size_in_blocks);
        }

        let start_block_index = block_index - current_run;
        let end_block_index = block_index;
        block_index = start_block_index;
        trace!(
            "Allocating fulfilling: {}..{}",
            start_block_index,
            end_block_index
        );

        let start_map_index = start_block_index / BlockPage::BLOCKS_COUNT;
        for (map_index, block_page) in self
            .map
            .write()
            .iter_mut()
            .enumerate()
            .skip(start_map_index)
            .take(align_up_div(end_block_index, BlockPage::BLOCKS_COUNT) - start_map_index)
        {
            let mut page_state: [SectionState; 4] = [SectionState::empty(); 4];

            for (section_index, section) in block_page.iter_mut().enumerate() {
                page_state[section_index].had_bits = *section > 0;

                if block_index < end_block_index {
                    let traversed_blocks =
                        (map_index * BlockPage::BLOCKS_COUNT) + (section_index * 64);
                    let start_byte_bits = block_index - traversed_blocks;
                    let total_bits =
                        core::cmp::min(64, end_block_index - traversed_blocks) - start_byte_bits;
                    let bits_mask = Self::MASK_MAP[total_bits - 1] << start_byte_bits;

                    debug_assert_eq!(
                        *section & bits_mask,
                        0,
                        "attempting to allocate blocks that are already allocated"
                    );

                    *section |= bits_mask;
                    block_index += total_bits;
                }

                page_state[section_index].has_bits = *section > 0;
            }

            if SectionState::should_alloc(&page_state) {
                // 'has bits', but not 'had bits'
                self.with_addressor(|addressor| {
                    addressor.map(&Page::from_index(map_index), unsafe {
                        &crate::memory::global_lock_next().unwrap()
                    });
                });
            }
        }

        (start_block_index * Self::BLOCK_SIZE) as *mut u8
    }

    pub fn alloc_to(&self, mut frames: FrameIterator) -> *mut u8 {
        trace!("Allocation requested to: {} frames", frames.remaining());
        let size_in_frames = frames.remaining();
        let (mut map_index, mut current_run);

        while {
            map_index = 0;
            current_run = 0;

            for block_page in self.map.read().iter() {
                if block_page.is_empty() {
                    current_run += 1;
                } else {
                    current_run = 0;
                }

                map_index += 1;

                if current_run == size_in_frames {
                    break;
                }
            }

            current_run < size_in_frames
        } {
            self.grow(size_in_frames * BlockPage::BLOCKS_COUNT);
        }

        let start_index = map_index - current_run;
        trace!(
            "Allocation fulfilling: pages {}..{}",
            start_index,
            start_index + size_in_frames
        );

        self.with_addressor(|addressor| {
            for (map_index, block_page) in self
                .map
                .write()
                .iter_mut()
                .enumerate()
                .skip(start_index)
                .take(size_in_frames)
            {
                addressor.map(
                    &Page::from_index(map_index),
                    &frames.next().expect("invalid end of frame iterator"),
                );
                block_page.set_full();
            }
        });

        (start_index * 0x1000) as *mut u8
    }

    pub fn identity_map(&self, frame: &Frame) {
        trace!("Identity mapping requested: {:?}", frame);

        let map_len = self.map.read().len();
        if map_len <= frame.index() {
            self.grow((frame.index() - map_len) * BlockPage::BLOCKS_COUNT)
        }

        self.with_addressor(|addressor| {
            let block_page = &mut self.map.write()[frame.index()];

            if block_page.is_empty() {
                block_page.set_full();
                addressor.identity_map(frame);
            } else {
                panic!("attempting to identity map page with previously allocated blocks");
            }
        });
    }

    fn raw_dealloc(&self, ptr: *mut u8, size: usize) {
        let start_block_index = (ptr as usize) / Self::BLOCK_SIZE;
        let end_block_index = start_block_index + align_up_div(size, Self::BLOCK_SIZE);
        let mut block_index = start_block_index;
        trace!(
            "Deallocating requested: {}..{}",
            start_block_index,
            end_block_index
        );

        let start_map_index = start_block_index / BlockPage::BLOCKS_COUNT;
        for (map_index, block_page) in self
            .map
            .write()
            .iter_mut()
            .enumerate()
            .skip(start_map_index)
            .take(align_up_div(end_block_index, BlockPage::BLOCKS_COUNT) - start_map_index)
        {
            let mut page_state: [SectionState; 4] = [SectionState::empty(); 4];

            for (section_index, section) in block_page.iter_mut().enumerate() {
                page_state[section_index].had_bits = *section > 0;

                if block_index < end_block_index {
                    let traversed_blocks =
                        (map_index * BlockPage::BLOCKS_COUNT) + (section_index * 64);
                    let start_byte_bits = block_index - traversed_blocks;
                    let total_bits =
                        core::cmp::min(64, end_block_index - traversed_blocks) - start_byte_bits;
                    let bits_mask = Self::MASK_MAP[total_bits - 1] << start_byte_bits;

                    debug_assert_eq!(
                        *section & bits_mask,
                        bits_mask,
                        "attempting to allocate blocks that are already allocated"
                    );

                    *section ^= bits_mask;
                    block_index += total_bits;
                }

                page_state[section_index].has_bits = *section > 0;
            }

            if SectionState::should_dealloc(&page_state) {
                // 'has bits', but not 'had bits'
                self.with_addressor(|addressor| {
                    let page = &Page::from_index(map_index);
                    unsafe { crate::memory::global_free(&addressor.translate_page(page).unwrap()) };
                    addressor.unmap(page);
                });
            }
        }
    }

    pub fn grow(&self, required_blocks: usize) {
        self.with_addressor(|addressor| {
            let map_read = self.map.upgradeable_read();
            let new_map_len = usize::next_power_of_two(
                (map_read.len() * BlockPage::BLOCKS_COUNT) + required_blocks,
            );

            use core::mem::size_of;
            let frame_usage = ((map_read.len() * size_of::<BlockPage>()) + 0xFFF) / 0x1000;
            let new_frame_usage = ((new_map_len * size_of::<BlockPage>()) + 0xFFF) / 0x1000;
            trace!("Growth frame usage: {} -> {}", frame_usage, new_frame_usage);
            for offset in frame_usage..new_frame_usage {
                addressor.map(&Self::ALLOCATOR_BASE.offset(offset), unsafe {
                    &crate::memory::global_lock_next().unwrap()
                });
            }

            map_read.upgrade().resize(new_map_len, BlockPage::empty());
            trace!("Successfully grew allocator map.");
        });
    }
}

unsafe impl core::alloc::GlobalAlloc for BlockAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        self.raw_alloc(layout.size())
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: core::alloc::Layout) {
        self.raw_dealloc(ptr, layout.size());
    }
}
