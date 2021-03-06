use core::mem::size_of;
use libkernel::{
    addr_ty::{Physical, Virtual},
    align_up_div,
    memory::{falloc, paging::VirtualAddressor, Frame, FrameIterator, Page},
    Address, SYSTEM_SLICE_SIZE,
};
use spin::RwLock;

/// Represents one page worth of memory blocks (i.e. 4096 bytes in blocks).
#[repr(C)]
#[derive(Clone)]
struct BlockPage {
    sections: [u64; Self::SECTION_COUNT],
}

impl BlockPage {
    /// Number of sections (primitive used to track blocks with its bits).
    const SECTION_COUNT: usize = 4;
    const SECTION_LEN: usize = size_of::<u64>() * 8;
    /// Number of blocks each block page contains.
    const BLOCK_COUNT: usize = Self::SECTION_COUNT * Self::SECTION_LEN;

    /// An empty block page (all blocks zeroed).
    const fn empty() -> Self {
        Self {
            sections: [0; Self::SECTION_COUNT],
        }
    }

    /// Whether the block page is empty.
    pub fn is_empty(&self) -> bool {
        self.iter().all(|section| *section == u64::MIN)
    }

    /// Whether the block page is full.
    pub fn is_full(&self) -> bool {
        self.iter().all(|section| *section == u64::MAX)
    }

    /// Unset all of the block page's blocks.
    pub fn set_empty(&mut self) {
        self.iter_mut().for_each(|section| *section = u64::MIN);
    }

    /// Set all of the block page's blocks.
    pub fn set_full(&mut self) {
        self.iter_mut().for_each(|section| *section = u64::MAX);
    }

    /// Underlying section iterator.
    fn iter(&self) -> core::slice::Iter<u64> {
        self.sections.iter()
    }

    /// Underlying mutable section iterator.
    fn iter_mut(&mut self) -> core::slice::IterMut<u64> {
        self.sections.iter_mut()
    }
}

impl core::ops::Index<usize> for BlockPage {
    type Output = u64;

    fn index(&self, index: usize) -> &Self::Output {
        &self.sections[index]
    }
}

impl core::ops::IndexMut<usize> for BlockPage {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.sections[index]
    }
}

impl core::fmt::Debug for BlockPage {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut debug_tuple = formatter.debug_tuple("BlockPage");

        self.iter().for_each(|section| {
            debug_tuple.field(section);
        });

        debug_tuple.finish()
    }
}

/// Allows tracking the state of the current block page's section
///  in a loop, so a block page's underlying global memory can be
///  allocated or deallocated accordingly.
#[derive(Clone, Copy)]
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

impl core::fmt::Debug for SectionState {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_tuple("SectionState")
            .field(&self.had_bits)
            .field(&self.has_bits)
            .finish()
    }
}

/// Allocator utilizing blocks of memory, in size of 16 bytes per block, to
///  easily and efficiently allocate.
pub struct BlockAllocator<'map> {
    // todo remove addressor from this struct
    addressor: RwLock<VirtualAddressor>,
    map: RwLock<&'map mut [BlockPage]>,
}

impl BlockAllocator<'_> {
    /// The size of an allocator block.
    pub const BLOCK_SIZE: usize = 16;

    /// Base page the allocator uses to store the internal block page map.
    const ALLOCATOR_BASE: Page =
        Page::from_addr(unsafe { Address::new_unsafe(SYSTEM_SLICE_SIZE * 0xA) });

    #[allow(const_item_mutation)]
    pub const fn new() -> Self {
        const EMPTY: [BlockPage; 0] = [];

        Self {
            // TODO make addressor use a RwLock
            addressor: RwLock::new(VirtualAddressor::null()),
            map: RwLock::new(&mut EMPTY),
        }
    }

    pub fn get_addressor(&self) -> spin::RwLockReadGuard<VirtualAddressor> {
        self.addressor.read()
    }

    pub unsafe fn get_addressor_mut(&self) -> spin::RwLockWriteGuard<VirtualAddressor> {
        self.addressor.write()
    }

    /* INITIALIZATION */

    pub unsafe fn init(&self, stack_frames: &mut libkernel::memory::FrameIterator) {
        {
            debug!("Initializing allocator's virtual addressor.");
            let mut addressor_mut = self.get_addressor_mut();
            *addressor_mut = VirtualAddressor::new(Page::null());

            debug!("Identity mapping all reserved global memory frames.");

            falloc::get()
                .iter()
                .enumerate()
                .filter(|(_, frame_state)| *frame_state == falloc::FrameState::Reserved)
                .for_each(|(frame_index, _)| {
                    addressor_mut.identity_map(&Frame::from_index(frame_index))
                });

            // Since we're using physical offset mapping for our page table modification
            //  strategy, the memory needs to be identity mapped at the correct offset.
            let phys_mapping_addr = falloc::virtual_map_offset();
            debug!("Mapping physical memory at offset: {:?}", phys_mapping_addr);
            addressor_mut.modify_mapped_page(Page::from_addr(phys_mapping_addr));

            // Swap the PML4 into CR3
            debug!("Writing kernel addressor's PML4 to the CR3 register.");
            addressor_mut.swap_into();
        }

        debug!("Allocating reserved global memory frames.");
        falloc::get()
            .iter()
            .enumerate()
            .filter(|(_, frame_state)| *frame_state == falloc::FrameState::Reserved)
            .for_each(|(frame_index, _)| self.identity_map(&Frame::from_index(frame_index), false));

        const STACK_SIZE: usize = 256 * 0x1000; /* 1MB in pages */

        debug!("Allocating new stack: {} bytes", STACK_SIZE);
        let new_stack_base = self.alloc::<u8>(
            core::alloc::Layout::from_size_align(STACK_SIZE, Self::BLOCK_SIZE).unwrap(),
        );
        let stack_base_cell = core::lazy::OnceCell::<*mut u8>::new();

        debug!("Copying data from bootloader-allocated stack.");
        for (index, frame) in stack_frames.enumerate() {
            let cur_offset = new_stack_base.add(index * 0x1000);
            let frame_ptr = frame.addr().as_usize() as *mut u8;
            stack_base_cell.set(frame_ptr).ok();

            core::ptr::copy_nonoverlapping(frame_ptr, cur_offset, 0x1000);
            // TODO also zero all antecedent stack frames
        }

        if let Some(stack_base) = stack_base_cell.get() {
            let base_offset = stack_base.offset_from(new_stack_base);

            debug!("Modifying `rsp` by base offset: 0x{:x}.", base_offset);
            use libkernel::registers::stack::RSP;
            if base_offset.is_positive() {
                RSP::sub(base_offset.abs() as u64);
            } else {
                RSP::add(base_offset.abs() as u64);
            }
        } else {
            panic!("failed to acquire current stack base pointer")
        }

        debug!("Unmapping bootloader-provided stack frames.");
        let mut addressor_mut = self.get_addressor_mut();
        stack_frames.reset();

        // `stack_frames` is invalid as we iterate and unmap the pages it exists on.
        {
            use core::mem::transmute;
            type FrameIteratorBytes = [u8; size_of::<FrameIterator>()];

            let temp = &mut [0u8; size_of::<FrameIterator>()];
            temp.copy_from_slice(transmute::<&mut FrameIterator, &mut FrameIteratorBytes>(
                stack_frames,
            ));

            transmute::<&mut FrameIteratorBytes, &mut FrameIterator>(temp)
        }
        .for_each(|frame| addressor_mut.unmap(&Page::from_index(frame.index())));

        debug!("Finished block allocator initialization.");
    }

    /* ALLOC & DEALLOC */

    // TODO consider returning a slice from this function rather than a raw pointer
    //      reasoning: possibly a more idiomatic way to return a sized chunk of memory
    pub fn alloc<T>(&self, layout: core::alloc::Layout) -> *mut T {
        const MINIMUM_ALIGNMENT: usize = 16;

        let size_in_blocks = (layout.size() + (Self::BLOCK_SIZE - 1)) / Self::BLOCK_SIZE;
        let alignment = if (layout.align() & (MINIMUM_ALIGNMENT - 1)) == 0 {
            layout.align()
        } else {
            trace!(
                "Unsupported allocator alignment: {}, defaulting to {}",
                layout.align(),
                Self::BLOCK_SIZE
            );

            MINIMUM_ALIGNMENT
        };

        trace!(
            "Allocation requested: {}{{by {}}} bytes ({} blocks)",
            layout.size(),
            alignment,
            size_in_blocks
        );

        let (mut block_index, mut current_run);
        while {
            block_index = 0;
            current_run = 0;

            'outer: for block_page in self.map.read().iter() {
                if block_page.is_full() {
                    current_run = 0;
                    block_index += BlockPage::BLOCK_COUNT;
                } else {
                    for section in block_page.iter().map(|section| *section) {
                        if section == u64::MAX {
                            current_run = 0;
                            block_index += BlockPage::SECTION_LEN;
                        } else {
                            for bit in (0..64).map(|shift| (section & (1 << shift)) > 0) {
                                if bit {
                                    current_run = 0;
                                } else if current_run > 0 || (block_index % alignment) == 0 {
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
            "Allocation fulfilling: {}..{}",
            start_block_index,
            end_block_index
        );

        let start_map_index = start_block_index / BlockPage::BLOCK_COUNT;
        let mut initial_section_skip =
            libkernel::align_down_div(block_index, BlockPage::SECTION_LEN)
                - (start_map_index * BlockPage::SECTION_COUNT);

        for (map_index, block_page) in self
            .map
            .write()
            .iter_mut()
            .enumerate()
            .skip(start_map_index)
            .take(align_up_div(end_block_index, BlockPage::BLOCK_COUNT) - start_map_index)
        {
            let mut page_state: [SectionState; BlockPage::SECTION_COUNT] =
                [SectionState::empty(); BlockPage::SECTION_COUNT];

            for (section_index, section) in block_page.iter_mut().enumerate() {
                page_state[section_index].had_bits = *section > 0;

                if initial_section_skip > 0 {
                    initial_section_skip -= 1;
                } else if block_index < end_block_index {
                    let (bit_count, bit_mask) = Self::calculate_bit_fields(
                        map_index,
                        section_index,
                        end_block_index,
                        block_index,
                    );

                    assert_eq!(
                        *section & bit_mask,
                        0,
                        "attempting to allocate blocks that are already allocated"
                    );

                    *section |= bit_mask;
                    block_index += bit_count;
                }

                page_state[section_index].has_bits = *section > 0;
            }

            if SectionState::should_alloc(&page_state) {
                // 'has bits', but not 'had bits'

                let page = &mut Page::from_index(map_index);

                unsafe {
                    self.get_addressor_mut()
                        .map(page, &falloc::get().lock_next().unwrap());
                    page.clear();
                }
            }
        }

        (start_block_index * Self::BLOCK_SIZE) as *mut T
    }

    pub fn dealloc<T>(&self, ptr: *mut T, size: usize) {
        let start_block_index = (ptr as usize) / Self::BLOCK_SIZE;
        let end_block_index = start_block_index + align_up_div(size, Self::BLOCK_SIZE);
        let mut block_index = start_block_index;
        trace!(
            "Deallocation requested: {}..{}",
            start_block_index,
            end_block_index
        );

        let start_map_index = start_block_index / BlockPage::BLOCK_COUNT;
        let end_map_index = align_up_div(end_block_index, BlockPage::BLOCK_COUNT) - start_map_index;
        let mut initial_section_skip =
            libkernel::align_down_div(block_index, BlockPage::SECTION_LEN)
                - (start_map_index * BlockPage::SECTION_COUNT);
        for (map_index, block_page) in self
            .map
            .write()
            .iter_mut()
            .enumerate()
            .skip(start_map_index)
            .take(end_map_index)
        {
            let mut page_state: [SectionState; BlockPage::SECTION_COUNT] =
                [SectionState::empty(); BlockPage::SECTION_COUNT];

            for (section_index, section) in block_page.iter_mut().enumerate() {
                page_state[section_index].had_bits = *section > 0;

                if initial_section_skip > 0 {
                    initial_section_skip -= 1;
                } else if block_index < end_block_index {
                    let (bit_count, bit_mask) = Self::calculate_bit_fields(
                        map_index,
                        section_index,
                        end_block_index,
                        block_index,
                    );

                    assert_eq!(
                        *section & bit_mask,
                        bit_mask,
                        "attempting to deallocate blocks that are already deallocated"
                    );

                    *section ^= bit_mask;
                    block_index += bit_count;
                }

                page_state[section_index].has_bits = *section > 0;
            }

            if SectionState::should_dealloc(&page_state) {
                // 'has bits', but not 'had bits'
                let mut addressor_mut = unsafe { self.get_addressor_mut() };
                let page = &Page::from_index(map_index);
                // todo FIX THIS (uncomment & build for error)
                unsafe {
                    falloc::get()
                        .free_frame(addressor_mut.translate_page(page).unwrap())
                        .unwrap()
                };
                addressor_mut.unmap(page);
            }
        }
    }

    /// Calculates the bit count and mask for a given set of block page parameters.
    fn calculate_bit_fields(
        map_index: usize,
        section_index: usize,
        end_block_index: usize,
        block_index: usize,
    ) -> (usize, u64) {
        let traversed_blocks =
            (map_index * BlockPage::BLOCK_COUNT) + (section_index * BlockPage::SECTION_LEN);
        let remaining_blocks = end_block_index - traversed_blocks;
        // Each block is one bit in our map, so we calculate the offset into
        //  the current section, at which our current index (`block_index`) lies.
        let bit_offset = block_index - traversed_blocks;
        let bit_count = core::cmp::min(BlockPage::SECTION_LEN, remaining_blocks) - bit_offset;
        // Finally, we acquire the respective bitmask to flip all relevant bits in
        //  our current section.
        (
            bit_count,
            libkernel::U64_BIT_MASKS[bit_count - 1] << bit_offset,
        )
    }

    /// Allocates a region of memory pointing to the frame region indicated by
    ///  given the iterator.
    ///
    /// This function assumed the frames are already locked or otherwise valid.
    pub fn alloc_to<T>(&self, frames: &FrameIterator) -> *mut T {
        let size_in_frames = frames.len();
        trace!("Allocation requested to: {} frames", size_in_frames);
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
            self.grow(size_in_frames * BlockPage::BLOCK_COUNT);
        }

        let start_index = map_index - current_run;
        trace!(
            "Allocation fulfilling: pages {}..{}",
            start_index,
            start_index + size_in_frames
        );

        {
            let mut addressor_mut = unsafe { self.get_addressor_mut() };
            for (map_index, block_page) in self
                .map
                .write()
                .iter_mut()
                .enumerate()
                .skip(start_index)
                .take(size_in_frames)
            {
                block_page.set_full();
                addressor_mut.map(&Page::from_index(map_index), &frames.start());
            }
        }

        (start_index * 0x1000) as *mut T
    }

    pub fn identity_map(&self, frame: &Frame, map: bool) {
        trace!("Identity mapping requested: {:?}", frame);

        let map_len = self.map.read().len();
        if map_len <= frame.index() {
            self.grow(((frame.index() - map_len) + 1) * BlockPage::BLOCK_COUNT);
        }

        let block_page = &mut self.map.write()[frame.index()];
        block_page.set_empty();
        assert!(
            block_page.is_empty(),
            "attempting to identity map page with previously allocated blocks: {:?} (map? {})\n {:?}",
            frame,
            map,
            block_page
        );
        block_page.set_full();

        if map {
            unsafe { self.get_addressor_mut() }.identity_map(frame);
        }
    }

    pub fn grow(&self, required_blocks: usize) {
        assert!(required_blocks > 0, "calls to grow must be nonzero");

        trace!("Growing map to faciliate {} blocks.", required_blocks);
        const BLOCKS_PER_MAP_PAGE: usize = 8 /* bits per byte */ * 0x1000;
        let map_read = self.map.upgradeable_read();
        let cur_map_len = map_read.len();
        let cur_page_offset = (cur_map_len * BlockPage::BLOCK_COUNT) / BLOCKS_PER_MAP_PAGE;
        let new_page_offset = (cur_page_offset
            + libkernel::align_up_div(required_blocks, BLOCKS_PER_MAP_PAGE))
        .next_power_of_two();

        trace!(
            "Growing map: {}..{} pages",
            cur_page_offset,
            new_page_offset
        );

        {
            let mut addressor_mut = unsafe { self.get_addressor_mut() };
            for offset in cur_page_offset..new_page_offset {
                let map_page = &mut Self::ALLOCATOR_BASE.offset(offset);
                addressor_mut.map(map_page, &falloc::get().lock_next().unwrap());
            }
        }

        let new_map_len = new_page_offset * (0x1000 / size_of::<BlockPage>());
        let mut map_write = map_read.upgrade();
        *map_write = unsafe {
            &mut *core::ptr::slice_from_raw_parts_mut(
                Self::ALLOCATOR_BASE.as_mut_ptr(),
                new_map_len,
            )
        };
        map_write[cur_map_len..].fill(BlockPage::empty());

        trace!(
            "Grew map: {} pages, {} block pages, {} blocks.",
            new_page_offset,
            new_map_len,
            new_map_len * BLOCKS_PER_MAP_PAGE
        );
    }

    pub unsafe fn physical_memory(&self, addr: Address<Physical>) -> Address<Virtual> {
        self.get_addressor().mapped_page().addr() + addr.as_usize()
    }
}

impl libkernel::memory::malloc::MemoryAllocator for BlockAllocator<'_> {
    fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        self.alloc(layout)
    }

    fn alloc_to(&self, frames: &FrameIterator) -> *mut u8 {
        self.alloc_to(frames)
    }

    fn dealloc(&self, ptr: *mut u8, layout: core::alloc::Layout) {
        self.dealloc(ptr, layout.size());
    }

    fn minimum_alignment(&self) -> usize {
        Self::BLOCK_SIZE
    }

    unsafe fn physical_memory(&self, addr: Address<Physical>) -> Address<Virtual> {
        self.physical_memory(addr)
    }
}
