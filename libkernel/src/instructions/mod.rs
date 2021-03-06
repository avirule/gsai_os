mod cpuid;

pub use cpuid::*;
pub mod interrupts;
pub mod pwm;
pub mod tlb;

pub fn hlt() {
    unsafe {
        asm!("hlt", options(nomem, nostack));
    }
}

pub fn hlt_indefinite() -> ! {
    loop {
        hlt();
    }
}

pub unsafe fn init_segment_registers(value: u16) {
    asm!(
        "mov ds, ax",
        "mov es, ax",
        "mov gs, ax",
        "mov fs, ax",
        "mov ss, ax",
        in("ax") value
    );
}
