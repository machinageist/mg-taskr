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

// Let a closed pipe end the program quietly, as other command-line tools do
// Rust ignores SIGPIPE by default, so `mg-taskr processes | head` would panic on the next write
#[allow(unsafe_code)]
pub fn exit_on_closed_pipe() {
    // SAFETY: restores the default handler for one signal before any threads start
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

// Signal one process by pid
// pid 0 or anything past i32 would mean "my whole group" or wrap to "everything" — refused here
#[allow(unsafe_code)]
pub fn send_signal(pid: u32, signal: i32) -> std::io::Result<()> {
    let target = one_pid(pid)?;
    // SAFETY: kill takes two integers; `target` is a single positive pid
    let status = unsafe { libc::kill(target, signal) };
    if status == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

// Set a process's nice value (-20 fastest … 19 slowest)
#[allow(unsafe_code)]
pub fn set_nice(pid: u32, nice: i32) -> std::io::Result<()> {
    let target = one_pid(pid)?;
    // SAFETY: setpriority takes integers only; pid 0 (meaning "this process") is refused above
    let status = unsafe { libc::setpriority(libc::PRIO_PROCESS, target as libc::id_t, nice) };
    if status == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

// A pid that names exactly one process: above 0 and inside the kernel's signed range
fn one_pid(pid: u32) -> std::io::Result<i32> {
    i32::try_from(pid).ok().filter(|p| *p > 0).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "pid must be a single process above 0",
        )
    })
}
