use gl::isolate::{caps, ns};
use std::process::Command;

const CAP_SETPCAP: u32 = 8;
const CAP_NET_RAW: u32 = 13;

fn status_text() -> String {
    std::fs::read_to_string("/proc/self/status").expect("read /proc/self/status")
}

fn hex_field(status: &str, key: &str) -> u64 {
    let line = status
        .lines()
        .find(|line| line.starts_with(key))
        .unwrap_or_else(|| panic!("missing {} in /proc status", key));
    let value = line
        .split_whitespace()
        .nth(1)
        .unwrap_or_else(|| panic!("missing value for {}", key));
    u64::from_str_radix(value, 16).unwrap_or_else(|_| panic!("invalid hex for {}", key))
}

fn no_new_privs(status: &str) -> u32 {
    let line = status
        .lines()
        .find(|line| line.starts_with("NoNewPrivs:"))
        .expect("missing NoNewPrivs in /proc status");
    line.split_whitespace()
        .nth(1)
        .expect("missing NoNewPrivs value")
        .parse()
        .expect("invalid NoNewPrivs value")
}

fn assert_cap_absent(status: &str, cap: u32) {
    let mask = 1u64 << cap;
    for key in ["CapInh:", "CapPrm:", "CapEff:", "CapBnd:", "CapAmb:"] {
        assert_eq!(
            hex_field(status, key) & mask,
            0,
            "{} still contains capability {}\n{}",
            key,
            cap,
            status
        );
    }
}

/// Privileged Linux evidence test.
///
/// This permanently lowers CAP_NET_RAW in the test process, so normal test
/// runs keep it ignored. CI executes this integration test in its own sudo
/// process after all ordinary build/test work has completed.
#[test]
#[ignore = "requires isolated privileged Linux capability environment"]
fn privileged_drop_survives_execve() {
    let before = status_text();
    println!("--- capability state before ---\n{}", before);

    let setpcap_mask = 1u64 << CAP_SETPCAP;
    let net_raw_mask = 1u64 << CAP_NET_RAW;
    assert_ne!(hex_field(&before, "CapEff:") & setpcap_mask, 0, "fixture requires CAP_SETPCAP effective");
    assert_ne!(hex_field(&before, "CapEff:") & net_raw_mask, 0, "fixture requires CAP_NET_RAW effective");
    assert_ne!(hex_field(&before, "CapPrm:") & net_raw_mask, 0, "fixture requires CAP_NET_RAW permitted");
    assert_ne!(hex_field(&before, "CapBnd:") & net_raw_mask, 0, "fixture requires CAP_NET_RAW bounded");

    ns::set_no_new_privs().expect("set no_new_privs");
    caps::drop_capabilities(&["CAP_NET_RAW".to_string()])
        .expect("enforce CAP_NET_RAW drop");

    let after = status_text();
    println!("--- capability state after drop ---\n{}", after);
    assert_eq!(no_new_privs(&after), 1);
    assert_cap_absent(&after, CAP_NET_RAW);

    // Command::output() creates a child and execs /bin/sh. Reading status in
    // that process proves the dropped capability did not reappear across the
    // supported no_new_privs + execve transition.
    let output = Command::new("/bin/sh")
        .args([
            "-c",
            "grep -E '^(CapInh|CapPrm|CapEff|CapBnd|CapAmb|NoNewPrivs):' /proc/self/status",
        ])
        .output()
        .expect("exec child status probe");
    assert!(output.status.success(), "child status probe failed");

    let child = String::from_utf8(output.stdout).expect("child status UTF-8");
    println!("--- capability state after child execve ---\n{}", child);
    assert_eq!(no_new_privs(&child), 1);
    assert_cap_absent(&child, CAP_NET_RAW);
}
