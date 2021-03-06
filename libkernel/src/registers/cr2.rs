use crate::{addr_ty::Virtual, Address};

pub struct CR2;

impl CR2 {
    /// Read the current page fault linear address from the CR2 register.
    pub fn read() -> Address<Virtual> {
        let value: usize;

        unsafe {
            asm!("mov {}, cr2", out(reg) value, options(nomem, nostack));
        }

        Address::<Virtual>::new(value)
    }
}
