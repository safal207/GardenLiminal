use anyhow::{Context, Result};
use gl::isolate::{mount, ns, seccomp};
use nix::mount::{mount as nix_mount, MsFlags};
use nix::sched::{unshare, CloneFlags};
use std::path::PathBuf;

fn main() -> Result<()> {
    let mode = std::env::args()
        .nth(1)
        .context("usage: gl-isolation-evidence <pivot-root|seccomp>")?;

    match mode.as_str() {
        "pivot-root" => pivot_root_probe(),
        "seccomp" => seccomp_probe(),
        other => anyhow::bail!("unknown evidence probe mode: {other}"),
    }
}

fn pivot_root_probe() -> Result<()> {
    let host_mnt_before = namespace_id("mnt")?;

    // Isolate the destructive mount transition to this short-lived process.
    unshare(CloneFlags::CLONE_NEWNS).context("unshare mount namespace")?;
    let probe_mnt = namespace_id("mnt")?;
    if probe_mnt == host_mnt_before {
        anyhow::bail!("mount namespace id did not change after unshare");
    }

    // Match the runtime precondition: no mount propagation from this namespace
    // may leak back to the host namespace.
    nix_mount(
        None::<&str>,
        "/",
        None::<&str>,
        MsFlags::MS_REC | MsFlags::MS_PRIVATE,
        None::<&str>,
    )
    .context("make evidence mount namespace private")?;

    let base = PathBuf::from(format!("/tmp/gl-isolation-evidence-{}", std::process::id()));
    let rootfs = base.join("rootfs");
    let host_only = base.join("host-only-sentinel");
    std::fs::create_dir_all(&rootfs).context("create evidence rootfs")?;
    std::fs::write(&host_only, b"host-only").context("create host sentinel")?;
    std::fs::write(rootfs.join("inside-root"), b"inside").context("create root sentinel")?;

    mount::pivot_root_to(&rootfs).context("pivot_root evidence transition")?;

    let root_sentinel_visible = std::path::Path::new("/inside-root").exists();
    let old_root_visible = std::path::Path::new("/.old_root").exists();
    let host_sentinel_visible = host_only.exists();

    if !root_sentinel_visible {
        anyhow::bail!("new root sentinel is not visible after pivot_root");
    }
    if old_root_visible {
        anyhow::bail!("old root remains visible after pivot_root");
    }
    if host_sentinel_visible {
        anyhow::bail!("host-only sentinel remains reachable after pivot_root");
    }

    println!(
        "pivot_root_postcondition=PASS host_mnt_before={} probe_mnt={} new_root_visible={} old_root_visible={} host_sentinel_visible={}",
        host_mnt_before,
        probe_mnt,
        root_sentinel_visible,
        old_root_visible,
        host_sentinel_visible
    );
    Ok(())
}

fn seccomp_probe() -> Result<()> {
    let nnp_before = prctl_get_no_new_privs()?;
    ns::set_no_new_privs().context("set no_new_privs")?;
    let nnp_after = prctl_get_no_new_privs()?;
    if nnp_before != 0 || nnp_after != 1 {
        anyhow::bail!(
            "unexpected no_new_privs transition: before={} after={}",
            nnp_before,
            nnp_after
        );
    }

    seccomp::apply_seccomp("minimal").context("install minimal seccomp filter")?;
    let seccomp_mode = prctl_get_seccomp()?;
    if seccomp_mode != 2 {
        anyhow::bail!("expected seccomp filter mode 2, got {}", seccomp_mode);
    }

    // The minimal profile intentionally excludes socket(2). A real kernel
    // denial proves the filter is installed rather than merely compiled.
    let fd = unsafe { nix::libc::socket(nix::libc::AF_INET, nix::libc::SOCK_STREAM, 0) };
    if fd >= 0 {
        unsafe { nix::libc::close(fd) };
        anyhow::bail!("socket unexpectedly succeeded under minimal seccomp profile");
    }
    let errno = std::io::Error::last_os_error().raw_os_error();
    if errno != Some(nix::libc::EPERM) {
        anyhow::bail!("socket denial returned {:?}, expected EPERM", errno);
    }

    println!(
        "seccomp_postcondition=PASS no_new_privs_before={} no_new_privs_after={} seccomp_mode={} denied_syscall=socket errno=EPERM",
        nnp_before, nnp_after, seccomp_mode
    );
    Ok(())
}

fn namespace_id(kind: &str) -> Result<String> {
    std::fs::read_link(format!("/proc/thread-self/ns/{kind}"))
        .with_context(|| format!("read {kind} namespace id"))
        .map(|path| path.to_string_lossy().into_owned())
}

fn prctl_get_no_new_privs() -> Result<i32> {
    let ret = unsafe {
        nix::libc::prctl(
            nix::libc::PR_GET_NO_NEW_PRIVS,
            0 as nix::libc::c_ulong,
            0 as nix::libc::c_ulong,
            0 as nix::libc::c_ulong,
            0 as nix::libc::c_ulong,
        )
    };
    if ret < 0 {
        anyhow::bail!("PR_GET_NO_NEW_PRIVS failed: {}", std::io::Error::last_os_error());
    }
    Ok(ret)
}

fn prctl_get_seccomp() -> Result<i32> {
    let ret = unsafe {
        nix::libc::prctl(
            nix::libc::PR_GET_SECCOMP,
            0 as nix::libc::c_ulong,
            0 as nix::libc::c_ulong,
            0 as nix::libc::c_ulong,
            0 as nix::libc::c_ulong,
        )
    };
    if ret < 0 {
        anyhow::bail!("PR_GET_SECCOMP failed: {}", std::io::Error::last_os_error());
    }
    Ok(ret)
}
