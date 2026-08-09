use gl::isolate::{caps, ns};
use std::process::Command;

const CAP_SETPCAP: u32 = 8;
const CAP_NET_RAW: u32 = 13;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProcStatus {
    cap_inh: u64,
    cap_prm: u64,
    cap_eff: u64,
    cap_bnd: u64,
    cap_amb: u64,
    no_new_privs: u32,
}

impl ProcStatus {
    fn assert_cap_absent(&self, cap: u32) {
        let mask = 1u64 << cap;
        for (name, value) in [
            ("CapInh", self.cap_inh),
            ("CapPrm", self.cap_prm),
            ("CapEff", self.cap_eff),
            ("CapBnd", self.cap_bnd),
            ("CapAmb", self.cap_amb),
        ] {
            assert_eq!(
                value & mask,
                0,
                "{} still contains capability {} in {:?}",
                name,
                cap,
                self
            );
        }
    }
}

// Linux capability sets and no_new_privs are task/thread credentials. Rust's
// test harness executes tests on worker threads, so /proc/self/status can show
// the thread-group leader rather than the calling test thread. thread-self
// always resolves to the task performing this read and therefore measures the
// same kernel credential state changed by prctl/capset below.
fn status_text() -> String {
    std::fs::read_to_string("/proc/thread-self/status")
        .expect("read /proc/thread-self/status")
}

fn parse_status(status: &str) -> ProcStatus {
    let mut cap_inh = None;
    let mut cap_prm = None;
    let mut cap_eff = None;
    let mut cap_bnd = None;
    let mut cap_amb = None;
    let mut no_new_privs = None;

    for line in status.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key {
            "CapInh" => cap_inh = Some(u64::from_str_radix(value, 16).expect("invalid CapInh")),
            "CapPrm" => cap_prm = Some(u64::from_str_radix(value, 16).expect("invalid CapPrm")),
            "CapEff" => cap_eff = Some(u64::from_str_radix(value, 16).expect("invalid CapEff")),
            "CapBnd" => cap_bnd = Some(u64::from_str_radix(value, 16).expect("invalid CapBnd")),
            "CapAmb" => cap_amb = Some(u64::from_str_radix(value, 16).expect("invalid CapAmb")),
            "NoNewPrivs" => no_new_privs = Some(value.parse().expect("invalid NoNewPrivs")),
            _ => {}
        }
    }

    ProcStatus {
        cap_inh: cap_inh.expect("missing CapInh"),
        cap_prm: cap_prm.expect("missing CapPrm"),
        cap_eff: cap_eff.expect("missing CapEff"),
        cap_bnd: cap_bnd.expect("missing CapBnd"),
        cap_amb: cap_amb.expect("missing CapAmb"),
        no_new_privs: no_new_privs.expect("missing NoNewPrivs"),
    }
}

/// Privileged Linux evidence test.
///
/// This permanently lowers CAP_NET_RAW in the calling test thread, so normal
/// test runs keep it ignored. CI executes this integration test in its own sudo
/// test process after all ordinary build/test work has completed.
#[test]
#[ignore = "requires isolated privileged Linux capability environment"]
fn privileged_drop_survives_execve() {
    let before_text = status_text();
    let before = parse_status(&before_text);
    println!("--- capability state before ---\n{}", before_text);

    let setpcap_mask = 1u64 << CAP_SETPCAP;
    let net_raw_mask = 1u64 << CAP_NET_RAW;
    assert_ne!(
        before.cap_eff & setpcap_mask,
        0,
        "fixture requires CAP_SETPCAP effective"
    );
    assert_ne!(
        before.cap_eff & net_raw_mask,
        0,
        "fixture requires CAP_NET_RAW effective"
    );
    assert_ne!(
        before.cap_prm & net_raw_mask,
        0,
        "fixture requires CAP_NET_RAW permitted"
    );
    assert_ne!(
        before.cap_bnd & net_raw_mask,
        0,
        "fixture requires CAP_NET_RAW bounded"
    );

    ns::set_no_new_privs().expect("set no_new_privs");
    caps::drop_capabilities(&["CAP_NET_RAW".to_string()])
        .expect("enforce CAP_NET_RAW drop");

    let after_text = status_text();
    let after = parse_status(&after_text);
    println!("--- capability state after drop ---\n{}", after_text);
    assert_eq!(after.no_new_privs, 1);
    after.assert_cap_absent(CAP_NET_RAW);

    // Command::output() creates a child and execs /bin/sh. Reading the child
    // shell's calling-task status proves the dropped capability did not
    // reappear across the supported no_new_privs + execve transition.
    let output = Command::new("/bin/sh")
        .args([
            "-c",
            "grep -E '^(CapInh|CapPrm|CapEff|CapBnd|CapAmb|NoNewPrivs):' /proc/thread-self/status",
        ])
        .output()
        .expect("exec child status probe");
    assert!(output.status.success(), "child status probe failed");

    let child_text = String::from_utf8(output.stdout).expect("child status UTF-8");
    let child = parse_status(&child_text);
    println!("--- capability state after child execve ---\n{}", child_text);
    assert_eq!(child.no_new_privs, 1);
    child.assert_cap_absent(CAP_NET_RAW);
}
