/// Initiates a breakpoint exception.
pub fn breakpoint() {
    unsafe {
        asm!("int 3");
    }
}

/// Enables interrupts via `sti`.
pub fn enable() {
    unsafe {
        asm!("sti", options(nostack, nomem));
    }
}

/// Disables interrupts via `cli`.
pub fn disable() {
    unsafe {
        asm!("cli", options(nostack, nomem));
    }
}

pub fn are_enabled() -> bool {
    use crate::registers::RFlags;

    RFlags::read().contains(RFlags::INTERRUPT_FLAG)
}

/// Executes given function with interrupts disabled, then
/// re-enables interrupts if they were previously enabled.
pub fn without_interrupts<F, R>(function: F) -> R
where
    F: FnOnce() -> R,
{
    let interrupts_enabled = are_enabled();

    if interrupts_enabled {
        disable();
    }

    let return_value = function();

    if interrupts_enabled {
        enable();
    }

    return_value
}
