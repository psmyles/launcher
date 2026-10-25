//! ICMP echo (ping) through iphlpapi, IPv4 and IPv6. Synchronous with a timeout; call it from a
//! worker thread.

use std::net::{IpAddr, ToSocketAddrs};

use windows::Win32::Foundation::HANDLE;
use windows::Win32::NetworkManagement::IpHelper::{
    Icmp6CreateFile, Icmp6SendEcho2, IcmpCloseHandle, IcmpCreateFile, IcmpSendEcho,
    ICMPV6_ECHO_REPLY_LH, ICMP_ECHO_REPLY,
};
use windows::Win32::Networking::WinSock::{AF_INET6, SOCKADDR_IN6};

#[derive(Clone, Debug)]
pub struct PingReply {
    pub address: IpAddr,
    pub ms: u32,
}

/// `IP_SUCCESS`.
const IP_SUCCESS: u32 = 0;

/// Resolve `host` (preferring IPv4, as .NET's Ping does for names) and send one echo.
pub fn ping(host: &str, timeout_ms: u32) -> Result<PingReply, String> {
    let host = host.trim();
    if host.is_empty() {
        return Err("no host to ping".into());
    }
    let addr: IpAddr = match host.parse::<IpAddr>() {
        Ok(a) => a,
        Err(_) => {
            let addrs: Vec<IpAddr> = (host, 0)
                .to_socket_addrs()
                .map_err(|e| format!("cannot resolve {host}: {e}"))?
                .map(|a| a.ip())
                .collect();
            *addrs
                .iter()
                .find(|a| a.is_ipv4())
                .or_else(|| addrs.first())
                .ok_or_else(|| format!("cannot resolve {host}"))?
        }
    };
    let timeout = timeout_ms.max(1);
    match addr {
        IpAddr::V4(v4) => ping4(u32::from_ne_bytes(v4.octets()), timeout)
            .map(|ms| PingReply { address: addr, ms }),
        IpAddr::V6(v6) => ping6(v6.octets(), timeout).map(|ms| PingReply { address: addr, ms }),
    }
}

struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: a handle from IcmpCreateFile/Icmp6CreateFile, closed once.
        let _ = unsafe { IcmpCloseHandle(self.0) };
    }
}

fn ping4(dest: u32, timeout: u32) -> Result<u32, String> {
    // SAFETY: plain Win32 calls; the buffers outlive them and their sizes are passed.
    unsafe {
        let h = Handle(IcmpCreateFile().map_err(|e| format!("ICMP unavailable: {e}"))?);
        let payload = [0x61u8; 32];
        let mut reply = vec![0u8; std::mem::size_of::<ICMP_ECHO_REPLY>() + payload.len() + 8 + 64];
        let n = IcmpSendEcho(
            h.0,
            dest,
            payload.as_ptr().cast(),
            payload.len() as u16,
            None,
            reply.as_mut_ptr().cast(),
            reply.len() as u32,
            timeout,
        );
        if n == 0 {
            return Err(format!("no reply within {timeout} ms"));
        }
        let r = &*(reply.as_ptr() as *const ICMP_ECHO_REPLY);
        if r.Status != IP_SUCCESS {
            return Err(format!("no reply (ICMP status {})", r.Status));
        }
        Ok(r.RoundTripTime)
    }
}

fn ping6(dest: [u8; 16], timeout: u32) -> Result<u32, String> {
    // SAFETY: as ping4; the sockaddr structs are fully initialized.
    unsafe {
        let h = Handle(Icmp6CreateFile().map_err(|e| format!("ICMPv6 unavailable: {e}"))?);
        let mut src: SOCKADDR_IN6 = std::mem::zeroed();
        src.sin6_family = AF_INET6;
        let mut dst: SOCKADDR_IN6 = std::mem::zeroed();
        dst.sin6_family = AF_INET6;
        dst.sin6_addr.u.Byte = dest;
        let payload = [0x61u8; 32];
        let mut reply =
            vec![0u8; std::mem::size_of::<ICMPV6_ECHO_REPLY_LH>() + payload.len() + 8 + 64];
        let n = Icmp6SendEcho2(
            h.0,
            None,
            None,
            None,
            &src,
            &dst,
            payload.as_ptr().cast(),
            payload.len() as u16,
            None,
            reply.as_mut_ptr().cast(),
            reply.len() as u32,
            timeout,
        );
        if n == 0 {
            return Err(format!("no reply within {timeout} ms"));
        }
        let r = &*(reply.as_ptr() as *const ICMPV6_ECHO_REPLY_LH);
        if r.Status != IP_SUCCESS {
            return Err(format!("no reply (ICMP status {})", r.Status));
        }
        Ok(r.RoundTripTime)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn loopback_answers() {
        let r = super::ping("127.0.0.1", 1000).expect("loopback ping");
        assert!(r.ms < 1000);
    }
}
