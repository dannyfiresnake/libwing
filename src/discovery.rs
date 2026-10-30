use std::collections::HashSet;
use std::io;
use std::net::{SocketAddrV4, UdpSocket};
use std::time::{Duration, Instant};

use if_addrs::{IfAddr, Interface};

use crate::console::DiscoveryInfo;
use crate::Result;

const SCAN_TIMEOUT: Duration = Duration::from_secs(5);

fn probe(interface: &Interface) -> io::Result<Option<UdpSocket>> {
    if !interface.is_oper_up() || interface.is_loopback() || interface.is_p2p() {
        return Ok(None);
    }
    let addr = match &interface.addr {
        IfAddr::V4(addr) if addr.broadcast.is_some() && !addr.ip.is_unspecified() => addr,
        _ => return Ok(None),
    };
    let socket = UdpSocket::bind(SocketAddrV4::new(addr.ip, 0))?;
    socket.set_broadcast(true)?;
    socket.set_nonblocking(true)?;

    // Binding a source address alone does not override a VPN's broadcast route
    // on Apple platforms. IP_BOUND_IF also selects the outgoing interface.
    #[cfg(target_vendor = "apple")]
    {
        let index = interface
            .index
            .and_then(std::num::NonZeroU32::new)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "interface has no index"))?;
        socket2::SockRef::from(&socket).bind_device_by_index_v4(Some(index))?;
    }

    socket.send_to(b"WING?", SocketAddrV4::new(addr.broadcast.unwrap(), 2222))?;
    Ok(Some(socket))
}

fn parse_response(bytes: &[u8]) -> Option<DiscoveryInfo> {
    let response = std::str::from_utf8(bytes).ok()?;
    let tokens: Vec<_> = response
        .trim_end_matches(['\0', '\r', '\n'])
        .split(',')
        .collect();
    if tokens.len() < 6 || tokens[0] != "WING" {
        return None;
    }
    tokens[1].parse::<std::net::Ipv4Addr>().ok()?;
    Some(DiscoveryInfo {
        ip: tokens[1].to_owned(),
        name: tokens[2].to_owned(),
        model: tokens[3].to_owned(),
        serial: tokens[4].to_owned(),
        firmware: tokens[5].to_owned(),
    })
}

pub(crate) fn scan(stop_on_first: bool) -> Result<Vec<DiscoveryInfo>> {
    let interfaces = if_addrs::get_if_addrs()?;
    let mut sockets = Vec::new();
    let mut last_error = None;
    let deadline = Instant::now() + SCAN_TIMEOUT;
    for interface in interfaces {
        match probe(&interface) {
            Ok(Some(socket)) => sockets.push(socket),
            Ok(None) => {}
            Err(error) => last_error = Some(error),
        }
    }
    if sockets.is_empty() {
        return match last_error {
            Some(error) => Err(error.into()),
            None => Ok(Vec::new()),
        };
    }

    Ok(receive(sockets, stop_on_first, deadline))
}

fn receive(
    mut sockets: Vec<UdpSocket>,
    stop_on_first: bool,
    deadline: Instant,
) -> Vec<DiscoveryInfo> {
    let mut results = Vec::new();
    let mut seen = HashSet::new();
    let mut buf = [0u8; 1024];
    while Instant::now() < deadline && !sockets.is_empty() {
        sockets.retain(|socket| {
            if stop_on_first && !results.is_empty() {
                return true;
            }
            // One receive per interface per pass keeps busy interfaces fair
            // and ensures unrelated traffic cannot extend the scan deadline.
            match socket.recv_from(&mut buf) {
                Ok((received, _)) => {
                    if let Some(info) = parse_response(&buf[..received]) {
                        if seen.insert((info.ip.clone(), info.serial.clone())) {
                            results.push(info);
                        }
                    }
                    true
                }
                Err(error) => matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ),
            }
        });
        if stop_on_first && !results.is_empty() {
            break;
        }
        std::thread::sleep(
            Duration::from_millis(10).min(deadline.saturating_duration_since(Instant::now())),
        );
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deduplicates_replies_across_interfaces_and_keeps_distinct_mixers() {
        let a = UdpSocket::bind("127.0.0.1:0").unwrap();
        let b = UdpSocket::bind("127.0.0.1:0").unwrap();
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        for socket in [&a, &b] {
            socket.set_nonblocking(true).unwrap();
            sender
                .send_to(
                    b"WING,192.168.1.19,Studio,wing-compact,serial,3.1.1",
                    socket.local_addr().unwrap(),
                )
                .unwrap();
        }
        sender
            .send_to(
                b"WING,192.168.1.20,Other,wing,other-serial,3.1.1",
                b.local_addr().unwrap(),
            )
            .unwrap();
        let results = receive(
            vec![a, b],
            false,
            Instant::now() + Duration::from_millis(100),
        );
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].ip, "192.168.1.19");
        assert_eq!(results[1].ip, "192.168.1.20");
    }

    #[test]
    fn returns_early_on_first_reply_and_honors_deadline_without_replies() {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.set_nonblocking(true).unwrap();
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        sender
            .send_to(
                b"WING,192.168.1.19,Studio,wing-compact,serial,3.1.1",
                socket.local_addr().unwrap(),
            )
            .unwrap();
        let start = Instant::now();
        assert_eq!(
            receive(vec![socket], true, start + Duration::from_secs(5)).len(),
            1
        );
        assert!(start.elapsed() < Duration::from_secs(1));

        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.set_nonblocking(true).unwrap();
        let start = Instant::now();
        assert!(receive(vec![socket], false, start + Duration::from_millis(100)).is_empty());
        assert!(start.elapsed() >= Duration::from_millis(100));
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn parses_discovery_response_and_ignores_invalid_packets() {
        let info =
            parse_response(b"WING,192.168.1.19,Studio,wing-compact,serial,3.1.1\r\n").unwrap();
        assert_eq!(info.ip, "192.168.1.19");
        assert_eq!(info.name, "Studio");
        assert_eq!(info.firmware, "3.1.1");
        for packet in [
            b"WING?".as_slice(),
            b"OTHER,192.168.1.19,n,m,s,f",
            b"WING,invalid,n,m,s,f",
            b"\xff",
        ] {
            assert!(parse_response(packet).is_none());
        }
    }
}
