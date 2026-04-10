use anyhow::{Context, Result};
use seccompiler::{BpfProgram, SeccompAction, SeccompFilter};
use std::collections::BTreeMap;

/// Apply a named seccomp profile to the current process.
///
/// Must be called after no_new_privs is set (or with CAP_SYS_ADMIN).
/// GardenLiminal sets no_new_privs before calling this function.
pub fn apply_seccomp(profile: &str) -> Result<()> {
    tracing::debug!("Applying seccomp profile: {}", profile);

    let program = match profile {
        "strict"  => build_strict()?,
        "minimal" => build_minimal()?,
        "default" | _ => {
            if profile != "default" {
                tracing::warn!("Unknown seccomp profile '{}', falling back to default", profile);
            }
            build_default()?
        }
    };

    seccompiler::apply_filter(&program)
        .with_context(|| format!("Failed to apply seccomp profile '{}'", profile))?;

    tracing::debug!("Seccomp profile '{}' applied", profile);
    Ok(())
}

// ── profile builders ──────────────────────────────────────────────────────────

/// Strict: only read / write / exit. Suitable for pure data-processing jobs.
fn build_strict() -> Result<BpfProgram> {
    let allowed: &[i64] = &[
        nix::libc::SYS_read,
        nix::libc::SYS_write,
        nix::libc::SYS_exit,
        nix::libc::SYS_exit_group,
        nix::libc::SYS_rt_sigreturn,
        nix::libc::SYS_futex,
        nix::libc::SYS_brk,
        nix::libc::SYS_mmap,
        nix::libc::SYS_mprotect,
        nix::libc::SYS_munmap,
        nix::libc::SYS_close,
    ];
    build_allow_list(allowed, SeccompAction::KillProcess)
}

/// Minimal: covers a typical POSIX process without network or privileged ops.
fn build_minimal() -> Result<BpfProgram> {
    let allowed: &[i64] = &[
        // Basic I/O
        nix::libc::SYS_read,    nix::libc::SYS_write,   nix::libc::SYS_pread64,
        nix::libc::SYS_pwrite64, nix::libc::SYS_readv,  nix::libc::SYS_writev,
        nix::libc::SYS_open,    nix::libc::SYS_openat,  nix::libc::SYS_close,
        nix::libc::SYS_stat,    nix::libc::SYS_fstat,   nix::libc::SYS_lstat,
        nix::libc::SYS_lseek,   nix::libc::SYS_access,  nix::libc::SYS_faccessat,
        // Memory
        nix::libc::SYS_brk,     nix::libc::SYS_mmap,    nix::libc::SYS_mprotect,
        nix::libc::SYS_munmap,  nix::libc::SYS_mremap,  nix::libc::SYS_madvise,
        // Process
        nix::libc::SYS_clone,   nix::libc::SYS_fork,    nix::libc::SYS_vfork,
        nix::libc::SYS_execve,  nix::libc::SYS_execveat,
        nix::libc::SYS_wait4,   nix::libc::SYS_waitid,
        nix::libc::SYS_exit,    nix::libc::SYS_exit_group,
        nix::libc::SYS_getpid,  nix::libc::SYS_getppid, nix::libc::SYS_gettid,
        nix::libc::SYS_getuid,  nix::libc::SYS_getgid,  nix::libc::SYS_geteuid,
        nix::libc::SYS_getegid, nix::libc::SYS_getpgrp, nix::libc::SYS_setsid,
        nix::libc::SYS_setpgid,
        // Signals
        nix::libc::SYS_rt_sigaction, nix::libc::SYS_rt_sigprocmask, nix::libc::SYS_rt_sigreturn,
        nix::libc::SYS_rt_sigsuspend, nix::libc::SYS_kill, nix::libc::SYS_tgkill,
        // Filesystem
        nix::libc::SYS_getcwd,  nix::libc::SYS_chdir,   nix::libc::SYS_mkdir,
        nix::libc::SYS_rmdir,   nix::libc::SYS_unlink,  nix::libc::SYS_unlinkat,
        nix::libc::SYS_rename,  nix::libc::SYS_renameat, nix::libc::SYS_renameat2,
        nix::libc::SYS_link,    nix::libc::SYS_symlink,  nix::libc::SYS_readlink,
        nix::libc::SYS_chmod,   nix::libc::SYS_fchmod,  nix::libc::SYS_chown,
        nix::libc::SYS_fchown,  nix::libc::SYS_lchown,
        nix::libc::SYS_getdents, nix::libc::SYS_getdents64,
        nix::libc::SYS_truncate, nix::libc::SYS_ftruncate,
        nix::libc::SYS_fsync,   nix::libc::SYS_fdatasync,
        nix::libc::SYS_dup,     nix::libc::SYS_dup2,    nix::libc::SYS_dup3,
        nix::libc::SYS_fcntl,   nix::libc::SYS_ioctl,
        nix::libc::SYS_pipe,    nix::libc::SYS_pipe2,
        // Time
        nix::libc::SYS_clock_gettime, nix::libc::SYS_clock_nanosleep,
        nix::libc::SYS_nanosleep, nix::libc::SYS_gettimeofday, nix::libc::SYS_time,
        // Sync / futex
        nix::libc::SYS_futex,   nix::libc::SYS_set_robust_list, nix::libc::SYS_get_robust_list,
        // Misc
        nix::libc::SYS_arch_prctl, nix::libc::SYS_prctl,
        nix::libc::SYS_set_tid_address,
        nix::libc::SYS_uname,   nix::libc::SYS_sysinfo,
        nix::libc::SYS_getrlimit, nix::libc::SYS_setrlimit, nix::libc::SYS_prlimit64,
        nix::libc::SYS_getrusage,
        nix::libc::SYS_umask,
        nix::libc::SYS_poll,    nix::libc::SYS_ppoll,
        nix::libc::SYS_select,  nix::libc::SYS_pselect6,
        nix::libc::SYS_epoll_create, nix::libc::SYS_epoll_create1,
        nix::libc::SYS_epoll_ctl, nix::libc::SYS_epoll_wait, nix::libc::SYS_epoll_pwait,
        nix::libc::SYS_eventfd, nix::libc::SYS_eventfd2,
        nix::libc::SYS_timerfd_create, nix::libc::SYS_timerfd_settime, nix::libc::SYS_timerfd_gettime,
        nix::libc::SYS_signalfd, nix::libc::SYS_signalfd4,
        nix::libc::SYS_inotify_init, nix::libc::SYS_inotify_init1,
        nix::libc::SYS_inotify_add_watch, nix::libc::SYS_inotify_rm_watch,
        nix::libc::SYS_memfd_create,
        nix::libc::SYS_copy_file_range,
    ];
    build_allow_list(allowed, SeccompAction::Errno(nix::libc::EPERM as u32))
}

/// Default: minimal + networking. Suitable for web services.
fn build_default() -> Result<BpfProgram> {
    let mut allowed: Vec<i64> = minimal_syscall_list();

    // Networking
    allowed.extend_from_slice(&[
        nix::libc::SYS_socket,      nix::libc::SYS_connect,    nix::libc::SYS_accept,
        nix::libc::SYS_accept4,     nix::libc::SYS_bind,       nix::libc::SYS_listen,
        nix::libc::SYS_getsockname, nix::libc::SYS_getpeername,nix::libc::SYS_socketpair,
        nix::libc::SYS_setsockopt,  nix::libc::SYS_getsockopt,
        nix::libc::SYS_sendto,      nix::libc::SYS_recvfrom,
        nix::libc::SYS_sendmsg,     nix::libc::SYS_recvmsg,
        nix::libc::SYS_sendfile,    nix::libc::SYS_shutdown,
        nix::libc::SYS_sendmmsg,    nix::libc::SYS_recvmmsg,
    ]);

    build_allow_list(&allowed, SeccompAction::Errno(nix::libc::EPERM as u32))
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Return the syscall list used by the minimal profile (reused by default).
fn minimal_syscall_list() -> Vec<i64> {
    vec![
        nix::libc::SYS_read,    nix::libc::SYS_write,   nix::libc::SYS_pread64,
        nix::libc::SYS_pwrite64, nix::libc::SYS_readv,  nix::libc::SYS_writev,
        nix::libc::SYS_open,    nix::libc::SYS_openat,  nix::libc::SYS_close,
        nix::libc::SYS_stat,    nix::libc::SYS_fstat,   nix::libc::SYS_lstat,
        nix::libc::SYS_lseek,   nix::libc::SYS_access,  nix::libc::SYS_faccessat,
        nix::libc::SYS_brk,     nix::libc::SYS_mmap,    nix::libc::SYS_mprotect,
        nix::libc::SYS_munmap,  nix::libc::SYS_mremap,  nix::libc::SYS_madvise,
        nix::libc::SYS_clone,   nix::libc::SYS_fork,    nix::libc::SYS_vfork,
        nix::libc::SYS_execve,  nix::libc::SYS_execveat,
        nix::libc::SYS_wait4,   nix::libc::SYS_waitid,
        nix::libc::SYS_exit,    nix::libc::SYS_exit_group,
        nix::libc::SYS_getpid,  nix::libc::SYS_getppid, nix::libc::SYS_gettid,
        nix::libc::SYS_getuid,  nix::libc::SYS_getgid,  nix::libc::SYS_geteuid,
        nix::libc::SYS_getegid, nix::libc::SYS_getpgrp, nix::libc::SYS_setsid,
        nix::libc::SYS_setpgid,
        nix::libc::SYS_rt_sigaction, nix::libc::SYS_rt_sigprocmask, nix::libc::SYS_rt_sigreturn,
        nix::libc::SYS_rt_sigsuspend, nix::libc::SYS_kill, nix::libc::SYS_tgkill,
        nix::libc::SYS_getcwd,  nix::libc::SYS_chdir,   nix::libc::SYS_mkdir,
        nix::libc::SYS_rmdir,   nix::libc::SYS_unlink,  nix::libc::SYS_unlinkat,
        nix::libc::SYS_rename,  nix::libc::SYS_renameat, nix::libc::SYS_renameat2,
        nix::libc::SYS_link,    nix::libc::SYS_symlink,  nix::libc::SYS_readlink,
        nix::libc::SYS_chmod,   nix::libc::SYS_fchmod,  nix::libc::SYS_chown,
        nix::libc::SYS_fchown,  nix::libc::SYS_lchown,
        nix::libc::SYS_getdents, nix::libc::SYS_getdents64,
        nix::libc::SYS_truncate, nix::libc::SYS_ftruncate,
        nix::libc::SYS_fsync,   nix::libc::SYS_fdatasync,
        nix::libc::SYS_dup,     nix::libc::SYS_dup2,    nix::libc::SYS_dup3,
        nix::libc::SYS_fcntl,   nix::libc::SYS_ioctl,
        nix::libc::SYS_pipe,    nix::libc::SYS_pipe2,
        nix::libc::SYS_clock_gettime, nix::libc::SYS_clock_nanosleep,
        nix::libc::SYS_nanosleep, nix::libc::SYS_gettimeofday, nix::libc::SYS_time,
        nix::libc::SYS_futex,   nix::libc::SYS_set_robust_list, nix::libc::SYS_get_robust_list,
        nix::libc::SYS_arch_prctl, nix::libc::SYS_prctl,
        nix::libc::SYS_set_tid_address,
        nix::libc::SYS_uname,   nix::libc::SYS_sysinfo,
        nix::libc::SYS_getrlimit, nix::libc::SYS_setrlimit, nix::libc::SYS_prlimit64,
        nix::libc::SYS_getrusage,
        nix::libc::SYS_umask,
        nix::libc::SYS_poll,    nix::libc::SYS_ppoll,
        nix::libc::SYS_select,  nix::libc::SYS_pselect6,
        nix::libc::SYS_epoll_create, nix::libc::SYS_epoll_create1,
        nix::libc::SYS_epoll_ctl, nix::libc::SYS_epoll_wait, nix::libc::SYS_epoll_pwait,
        nix::libc::SYS_eventfd, nix::libc::SYS_eventfd2,
        nix::libc::SYS_timerfd_create, nix::libc::SYS_timerfd_settime, nix::libc::SYS_timerfd_gettime,
        nix::libc::SYS_signalfd, nix::libc::SYS_signalfd4,
        nix::libc::SYS_inotify_init, nix::libc::SYS_inotify_init1,
        nix::libc::SYS_inotify_add_watch, nix::libc::SYS_inotify_rm_watch,
        nix::libc::SYS_memfd_create,
        nix::libc::SYS_copy_file_range,
    ]
}

/// Build a BPF allow-list filter from a slice of syscall numbers.
/// Any syscall not in the list triggers `default_action`.
fn build_allow_list(syscalls: &[i64], default_action: SeccompAction) -> Result<BpfProgram> {
    let rules: BTreeMap<i64, Vec<seccompiler::SeccompRule>> = syscalls
        .iter()
        .map(|&nr| (nr, vec![]))
        .collect();

    let filter = SeccompFilter::new(
        rules,
        default_action,
        SeccompAction::Allow,
        std::env::consts::ARCH.try_into().context("Unsupported architecture for seccomp")?,
    )
    .context("Failed to build seccomp filter")?;

    filter.try_into().context("Failed to compile seccomp BPF program")
}
