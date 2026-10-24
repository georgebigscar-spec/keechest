//! Process-level protections for the secrets in memory.

/// Keeps decrypted secrets out of core dumps and, on Linux, stops other
/// processes of the same user from attaching a debugger to read memory.
pub fn harden_process() {
    #[cfg(unix)]
    // SAFETY: plain syscalls with valid arguments; failure is harmless.
    unsafe {
        let none = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        libc::setrlimit(libc::RLIMIT_CORE, &none);
        #[cfg(any(target_os = "linux", target_os = "android"))]
        libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0);
    }
}
