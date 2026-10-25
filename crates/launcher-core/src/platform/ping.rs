//! Ping: one ICMP echo. Windows uses iphlpapi (`icmp.rs`, from master); macOS asks `/sbin/ping`,
//! which may send ICMP without privileges.

#[derive(Clone, Debug, PartialEq)]
pub struct Reply {
    pub address: String,
    pub ms: u32,
}

#[cfg(windows)]
pub fn ping(host: &str, timeout_ms: u64) -> Result<Reply, String> {
    super::icmp::ping(host, timeout_ms.min(u32::MAX as u64) as u32).map(|r| Reply {
        address: r.address.to_string(),
        ms: r.ms,
    })
}

#[cfg(not(windows))]
pub fn ping(host: &str, timeout_ms: u64) -> Result<Reply, String> {
    let host = host.trim();
    if host.is_empty() {
        return Err("no host to ping".into());
    }
    let out = std::process::Command::new("/sbin/ping")
        .args(["-n", "-c", "1", "-W", &timeout_ms.max(1).to_string(), host])
        .output()
        .map_err(|e| format!("could not run ping: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    if !out.status.success() {
        return Err(format!("{host} does not answer"));
    }
    // "64 bytes from 10.0.0.1: icmp_seq=0 ttl=64 time=1.234 ms"
    let line = text.lines().find(|l| l.contains("time=")).unwrap_or("");
    let address = line
        .split("from ")
        .nth(1)
        .and_then(|r| r.split(':').next())
        .unwrap_or(host)
        .to_string();
    let ms = line
        .split("time=")
        .nth(1)
        .and_then(|r| r.split_whitespace().next())
        .and_then(|t| t.parse::<f64>().ok())
        .map(|t| t.round() as u32)
        .unwrap_or(0);
    Ok(Reply { address, ms })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn localhost_answers() {
        let r = ping("127.0.0.1", 2000).unwrap();
        assert_eq!(r.address, "127.0.0.1");
    }
}
