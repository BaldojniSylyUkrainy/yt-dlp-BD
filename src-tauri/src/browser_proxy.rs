//! Per-window, public-internet-only SOCKS5 relay. No TLS interception, global
//! proxy settings, credentials, request logging, UDP, or background service.
use std::{
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::watch,
    task::JoinSet,
    time::timeout,
};

const MAX_CONNECTIONS: usize = 64;
const LIFETIME: Duration = Duration::from_secs(15 * 60);
const HANDSHAKE: Duration = Duration::from_secs(10);

pub(crate) struct BrowserProxy {
    pub address: SocketAddr,
    stop: watch::Sender<bool>,
    pub expired: watch::Receiver<bool>,
}

impl BrowserProxy {
    pub async fn start() -> io::Result<Self> {
        Self::with_lifetime(LIFETIME).await
    }

    async fn with_lifetime(lifetime: Duration) -> io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let address = listener.local_addr()?;
        let (stop, mut shutdown) = watch::channel(false);
        let (expired_sender, expired) = watch::channel(false);
        tauri::async_runtime::spawn(async move {
            let mut connections = JoinSet::new();
            let deadline = tokio::time::sleep(lifetime);
            tokio::pin!(deadline);
            loop {
                tokio::select! {
                    biased;
                    _ = shutdown.changed() => break,
                    _ = &mut deadline => { let _ = expired_sender.send(true); break; },
                    _ = connections.join_next(), if !connections.is_empty() => {},
                    incoming = listener.accept() => {
                        let Ok((socket, peer)) = incoming else { break };
                        if !peer.ip().is_loopback() || connections.len() >= MAX_CONNECTIONS {
                            continue;
                        }
                        connections.spawn(async move {
                            let _ = serve(socket).await;
                        });
                    }
                }
            }
            // Dropping JoinSet aborts ALL handshakes, lookups and active tunnels.
            // The browser has no direct-connection fallback when this ends.
        });
        Ok(Self {
            address,
            stop,
            expired,
        })
    }

    pub fn stop(&self) {
        let _ = self.stop.send(true);
    }
}

impl Drop for BrowserProxy {
    fn drop(&mut self) {
        self.stop();
    }
}

fn denied() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "Browser network policy denied destination",
    )
}

pub(crate) fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, c, _] = v4.octets();
            super::is_public_thumbnail_ip(ip) && !(a == 192 && b == 88 && c == 99)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return public_ip(v4.into());
            }
            let s = v6.segments();
            // Accept allocated global unicast only; deny NAT64, compatible-v4,
            // Teredo, 6to4, documentation and other special-use translations.
            (s[0] & 0xe000) == 0x2000
                && !(s[0] == 0x2001 && s[1] < 0x0200)
                && s[0] != 0x2002
                && s[0] != 0x3fff
                && super::is_public_thumbnail_ip(ip)
        }
    }
}

fn public_host(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if let Ok(ip) = host.parse() {
        return public_ip(ip);
    }
    host.len() <= 253
        && host.contains('.')
        && !host.bytes().all(|c| c.is_ascii_digit() || c == b'.')
        && host
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'.')
        && ![
            "localhost",
            "local",
            "internal",
            "home",
            "lan",
            "test",
            "invalid",
            "onion",
            "arpa",
        ]
        .iter()
        .any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")))
        && host
            .split('.')
            .all(|label| !label.is_empty() && label.len() <= 63)
}

fn validate_addresses(addresses: &[SocketAddr]) -> io::Result<()> {
    if addresses.is_empty()
        || addresses.len() > 32
        || addresses
            .iter()
            .any(|a| a.port() == 0 || !public_ip(a.ip()))
    {
        return Err(denied());
    }
    Ok(())
}

async fn connect_public(host: &str, port: u16) -> io::Result<TcpStream> {
    if port == 0 || !public_host(host) {
        return Err(denied());
    }
    let addresses: Vec<_> = timeout(
        Duration::from_secs(5),
        tokio::net::lookup_host((host, port)),
    )
    .await??
    .take(33)
    .collect();
    validate_addresses(&addresses)?;
    // Connect to checked numeric addresses, NEVER resolve the hostname again.
    // A later DNS answer/redirect/new tunnel must pass a new validation.
    timeout(
        Duration::from_secs(5),
        TcpStream::connect(addresses.as_slice()),
    )
    .await?
}

async fn handshake(client: &mut TcpStream) -> io::Result<TcpStream> {
    let mut header = [0; 2];
    client.read_exact(&mut header).await?;
    if header[0] != 5 || header[1] == 0 {
        return Err(denied());
    }
    let mut methods = vec![0; header[1] as usize];
    client.read_exact(&mut methods).await?;
    if !methods.contains(&0) {
        client.write_all(&[5, 255]).await?;
        return Err(denied());
    }
    client.write_all(&[5, 0]).await?;
    let mut request = [0; 4];
    client.read_exact(&mut request).await?;
    if request[0] != 5 || request[2] != 0 {
        return Err(denied());
    }
    let host = match request[3] {
        1 => {
            let mut bytes = [0; 4];
            client.read_exact(&mut bytes).await?;
            Ipv4Addr::from(bytes).to_string()
        }
        4 => {
            let mut bytes = [0; 16];
            client.read_exact(&mut bytes).await?;
            Ipv6Addr::from(bytes).to_string()
        }
        3 => {
            let length = client.read_u8().await? as usize;
            let mut bytes = vec![0; length];
            client.read_exact(&mut bytes).await?;
            String::from_utf8(bytes).map_err(|_| denied())?
        }
        _ => return Err(denied()),
    };
    let port = client.read_u16().await?;
    // Consume the bounded destination before rejecting BIND/UDP. Closing with
    // unread request bytes can reset the socket on Windows and discard REP=2.
    // No destination resolution or connection occurs for unsupported commands.
    if request[1] != 1 {
        return Err(denied());
    }
    let remote = connect_public(&host, port).await?;
    client.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
    Ok(remote)
}

async fn serve(mut client: TcpStream) -> io::Result<()> {
    match timeout(HANDSHAKE, handshake(&mut client)).await {
        Ok(Ok(mut remote)) => {
            tokio::io::copy_bidirectional(&mut client, &mut remote).await?;
            Ok(())
        }
        _ => {
            let _ = timeout(
                Duration::from_secs(1),
                client.write_all(&[5, 2, 0, 1, 0, 0, 0, 0, 0, 0]),
            )
            .await;
            Err(denied())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn expiry_notifies_and_closes_the_listener() {
        let proxy = BrowserProxy::with_lifetime(Duration::from_millis(20))
            .await
            .unwrap();
        let mut expired = proxy.expired.clone();
        timeout(Duration::from_secs(2), expired.changed())
            .await
            .unwrap()
            .unwrap();
        assert!(*expired.borrow());
        tokio::task::yield_now().await;
        assert!(TcpStream::connect(proxy.address).await.is_err());
    }

    #[test]
    fn special_addresses_and_dns_rebinding_are_denied() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "169.254.169.254",
            "172.16.1.1",
            "192.168.1.1",
            "100.64.0.1",
            "0.0.0.0",
            "224.0.0.1",
            "192.88.99.1",
            "::1",
            "::ffff:127.0.0.1",
            "::127.0.0.1",
            "64:ff9b::7f00:1",
            "2002:7f00:1::1",
            "2001::1",
            "2001:db8::1",
            "fc00::1",
            "fe80::1",
        ] {
            assert!(!public_ip(ip.parse().unwrap()), "{ip}");
        }
        assert!(public_ip("1.1.1.1".parse().unwrap()));
        assert!(public_ip("2606:4700:4700::1111".parse().unwrap()));
        let mixed = [
            "1.1.1.1:443".parse().unwrap(),
            "127.0.0.1:443".parse().unwrap(),
        ];
        assert!(validate_addresses(&mixed).is_err());
        assert!(validate_addresses(&[]).is_err());
        for host in [
            "localhost",
            "LOCALHOST.",
            "x.local",
            "router",
            "a.home.arpa",
            "user@site.com",
            "site.com\0.local",
            "127.1",
            "0x7f000001",
            "2130706433",
            "example.com:80",
        ] {
            assert!(!public_host(host), "{host}");
        }
    }

    async fn request(proxy: &BrowserProxy, command: u8, host: &str, port: u16) -> TcpStream {
        let mut client = TcpStream::connect(proxy.address).await.unwrap();
        client.write_all(&[5, 1, 0]).await.unwrap();
        let mut reply = [0; 2];
        client.read_exact(&mut reply).await.unwrap();
        assert_eq!(reply, [5, 0]);
        let mut request = vec![5, command, 0, 3, host.len() as u8];
        request.extend(host.as_bytes());
        request.extend(port.to_be_bytes());
        client.write_all(&request).await.unwrap();
        client
    }

    #[tokio::test]
    async fn actual_socks_connection_cannot_reach_loopback_or_udp() {
        let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy = BrowserProxy::start().await.unwrap();
        for (cmd, host) in [
            (1, "127.0.0.1"),
            (1, "localhost"),
            (1, "::ffff:127.0.0.1"),
            (3, "1.1.1.1"),
            (2, "1.1.1.1"),
        ] {
            let mut client = request(&proxy, cmd, host, target.local_addr().unwrap().port()).await;
            let mut response = [0; 10];
            timeout(Duration::from_secs(2), client.read_exact(&mut response))
                .await
                .unwrap()
                .unwrap();
            assert_ne!(response[1], 0);
        }
        assert!(timeout(Duration::from_millis(50), target.accept())
            .await
            .is_err());
        let mut pending = TcpStream::connect(proxy.address).await.unwrap();
        proxy.stop();
        let mut byte = [0];
        let result = timeout(Duration::from_secs(2), pending.read(&mut byte))
            .await
            .unwrap();
        assert!(matches!(result, Ok(0) | Err(_)));
    }

    #[tokio::test]
    async fn binary_addresses_and_malformed_requests_fail_closed() {
        let proxy = BrowserProxy::start().await.unwrap();
        let mut cases = vec![
            vec![5, 1, 0, 1, 127, 0, 0, 1, 0, 80],
            vec![5, 1, 0, 1, 169, 254, 169, 254, 0, 80],
            vec![5, 1, 0, 1, 1, 1, 1, 1, 0, 0],
            vec![5, 1, 0, 3, 0, 0, 80],
            vec![5, 1, 0, 99],
            vec![4, 1, 0, 1],
        ];
        for ip in ["::1", "::ffff:127.0.0.1", "fc00::1"] {
            let mut request = vec![5, 1, 0, 4];
            request.extend(ip.parse::<Ipv6Addr>().unwrap().octets());
            request.extend(80u16.to_be_bytes());
            cases.push(request);
        }
        for request in cases {
            let mut client = TcpStream::connect(proxy.address).await.unwrap();
            client.write_all(&[5, 1, 0]).await.unwrap();
            let mut greeting = [0; 2];
            client.read_exact(&mut greeting).await.unwrap();
            assert_eq!(greeting, [5, 0]);
            client.write_all(&request).await.unwrap();
            let mut response = [0; 10];
            timeout(Duration::from_secs(2), client.read_exact(&mut response))
                .await
                .unwrap()
                .unwrap();
            assert_ne!(response[1], 0, "{request:?}");
        }
        let mut client = TcpStream::connect(proxy.address).await.unwrap();
        client.write_all(&[5, 1, 2]).await.unwrap();
        let mut response = [0; 2];
        client.read_exact(&mut response).await.unwrap();
        assert_eq!(response, [5, 255]);
    }
}
