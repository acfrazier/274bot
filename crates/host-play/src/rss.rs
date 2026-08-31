//! Process RSS / CPU sample for live harnesses and the panel resource card.

pub fn rss_bytes_from_ru_maxrss(raw: i64) -> u64 {
    if raw < 0 {
        return 0;
    }
    let n = raw as u64;
    #[cfg(target_os = "linux")]
    {
        n.saturating_mul(1024)
    }
    #[cfg(target_os = "macos")]
    {
        n
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = n;
        0
    }
}

pub fn sample_process() -> (u64, f64) {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
        let rc = unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
        if rc != 0 {
            return (0, 0.0);
        }
        let rss = rss_bytes_from_ru_maxrss(usage.ru_maxrss);
        let cpu = usage.ru_utime.tv_sec as f64
            + usage.ru_utime.tv_usec as f64 / 1e6
            + usage.ru_stime.tv_sec as f64
            + usage.ru_stime.tv_usec as f64 / 1e6;
        (rss, cpu)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        (0, 0.0)
    }
}

/// Count unique ESTABLISHED TCP names in `lsof -nP -iTCP` stdout.
/// One socket can appear twice (reader + writer fd); the NAME column
/// (`host:port->host:port`) is the connection.
pub fn parse_lsof_established(stdout: &str) -> usize {
    use std::collections::HashSet;
    let mut names = HashSet::new();
    for line in stdout.lines() {
        if !line.contains("ESTABLISHED") {
            continue;
        }
        let Some(tcp) = line.find("TCP ") else {
            continue;
        };
        let rest = &line[tcp + 4..];
        if let Some(name) = rest.split_whitespace().next() {
            names.insert(name);
        }
    }
    names.len()
}

/// Established TCP from this pid to `host:port`, via `lsof`. `None` if
/// `lsof` is missing. Empty match is `Some(0)` (`lsof` exits 1).
pub fn count_tcp_to(host: &str, port: u16) -> Option<usize> {
    let pid = std::process::id().to_string();
    let spec = format!("-iTCP@{host}:{port}");
    let output = std::process::Command::new("lsof")
        .args(["-nP", "-a", "-p", &pid, &spec])
        .output()
        .ok()?;
    Some(parse_lsof_established(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_process_rss_is_nonzero_on_this_host() {
        let (rss, cpu) = sample_process();
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            assert!(rss > 0, "rss={rss} cpu={cpu}");
            assert!(cpu >= 0.0);
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            assert_eq!((rss, cpu), (0, 0.0));
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn darwin_ru_maxrss_is_bytes() {
        assert_eq!(rss_bytes_from_ru_maxrss(4096), 4096);
        assert_eq!(rss_bytes_from_ru_maxrss(-1), 0);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_ru_maxrss_is_kilobytes() {
        assert_eq!(rss_bytes_from_ru_maxrss(2), 2048);
        assert_eq!(rss_bytes_from_ru_maxrss(-1), 0);
    }

    #[test]
    fn lsof_established_counts_unique_sockets_not_fds() {
        let two_fds_one_socket = "\
COMMAND   PID USER   FD   TYPE DEVICE SIZE/OFF NODE NAME
rss_ladder 1 acf   10u  IPv4 0x1      0t0  TCP 127.0.0.1:50000->127.0.0.1:43594 (ESTABLISHED)
rss_ladder 1 acf   11u  IPv4 0x1      0t0  TCP 127.0.0.1:50000->127.0.0.1:43594 (ESTABLISHED)
rss_ladder 1 acf   12u  IPv4 0x2      0t0  TCP 127.0.0.1:50001->127.0.0.1:43594 (ESTABLISHED)
rss_ladder 1 acf   13u  IPv4 0x3      0t0  TCP 127.0.0.1:50002->127.0.0.1:43594 (CLOSE_WAIT)
";
        assert_eq!(parse_lsof_established(two_fds_one_socket), 2);
        assert_eq!(parse_lsof_established(""), 0);
        assert_eq!(parse_lsof_established("COMMAND PID\n"), 0);
    }
}
