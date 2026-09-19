// Author: Jeff
// Date: 2026-09-18
// Description: The few facts only the C library can answer — clock ticks and who we are
// Notes: The crate denies unsafe code; each call here is the one allowed exception, kept to a
//        single line with the reason it cannot go wrong

// kernel clock the /proc tick counters use, when sysconf cannot say (it is 100 on every Linux build)
const FALLBACK_TICKS: f64 = 100.0;

// Ticks per second for the utime/stime counters in /proc
#[allow(unsafe_code)]
pub fn ticks_per_second() -> f64 {
    // SAFETY: sysconf only reads a constant and takes no pointers
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if ticks > 0 {
        ticks as f64
    } else {
        FALLBACK_TICKS
    }
}

// The real user id of this process — "mine" means owned by this uid
#[allow(unsafe_code)]
pub fn my_uid() -> u32 {
    // SAFETY: getuid cannot fail and takes no arguments
    unsafe { libc::getuid() }
}
