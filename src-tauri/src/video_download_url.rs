use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs},
    ops::RangeInclusive,
};

const CGNAT_IPV4_SECOND_OCTET: RangeInclusive<u8> = 64..=127;

#[derive(Debug, PartialEq, Eq)]
pub enum DownloadUrlError {
    Rejected(String),
    Resolution,
}
impl From<String> for DownloadUrlError {
    fn from(value: String) -> Self {
        Self::Rejected(value)
    }
}
impl From<&str> for DownloadUrlError {
    fn from(value: &str) -> Self {
        Self::Rejected(value.into())
    }
}

pub fn validate_video_download_url(
    url: &str,
) -> Result<(reqwest::Url, Vec<SocketAddr>), DownloadUrlError> {
    validate_with_resolver(url, |host, port| {
        (host, port).to_socket_addrs().map(Iterator::collect)
    })
}

fn validate_with_resolver(
    url: &str,
    resolve: impl FnOnce(&str, u16) -> std::io::Result<Vec<SocketAddr>>,
) -> Result<(reqwest::Url, Vec<SocketAddr>), DownloadUrlError> {
    let parsed = reqwest::Url::parse(url).map_err(|_| "video download URL is invalid")?;
    if parsed.scheme() != "https" {
        return Err("video download URL must use https".into());
    }
    let host = parsed
        .host_str()
        .ok_or("video download URL must include a host")?;
    let socket_host = host_without_ipv6_brackets(host);
    if let Ok(ip) = socket_host.parse::<IpAddr>() {
        reject_non_public_ip(ip)?;
    }
    let port = parsed
        .port_or_known_default()
        .ok_or("video download URL must include a resolvable port")?;
    let addrs = resolve(socket_host, port).map_err(|_| DownloadUrlError::Resolution)?;
    if addrs.is_empty() {
        return Err(DownloadUrlError::Resolution);
    }
    for addr in &addrs {
        reject_non_public_ip(addr.ip())?;
    }
    Ok((parsed, addrs))
}

pub fn video_download_client_builder(
    parsed: &reqwest::Url,
    validated_addrs: &[SocketAddr],
) -> Result<reqwest::ClientBuilder, String> {
    let host = parsed
        .host_str()
        .ok_or_else(|| "video download URL must include a host".to_string())?;
    let socket_host = host_without_ipv6_brackets(host);
    if validated_addrs.is_empty() {
        return Err("video download URL host did not resolve".to_string());
    }

    // Provider video download URLs are direct GETs. Redirects fail closed, so
    // no hop can bypass the validation and address pinning above. If a provider
    // later needs CDN redirects, validate, resolve, and pin every hop before
    // following it.
    Ok(reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(socket_host, validated_addrs))
}

fn host_without_ipv6_brackets(host: &str) -> &str {
    host.strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host)
}

fn reject_non_public_ip(ip: IpAddr) -> Result<(), String> {
    if is_non_public_ip(ip) {
        return Err("video download URL host resolved to a non-public address".to_string());
    }
    Ok(())
}

fn is_non_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ipv4) => is_non_public_ipv4(ipv4),
        IpAddr::V6(ipv6) => is_non_public_ipv6(ipv6),
    }
}

fn is_non_public_ipv4(ip: Ipv4Addr) -> bool {
    ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        || is_cgnat_ipv4(ip)
        || is_special_use_ipv4(ip)
}

fn is_cgnat_ipv4(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    octets[0] == 100 && CGNAT_IPV4_SECOND_OCTET.contains(&octets[1])
}

fn is_special_use_ipv4(ip: Ipv4Addr) -> bool {
    // IANA IPv4 Special-Purpose Address Registry blocks that are not covered by
    // the standard library predicates above:
    // - 0.0.0.0/8 ("This network")
    // - 192.0.0.0/24 (IETF protocol assignments)
    // - 192.0.2.0/24, 198.51.100.0/24, 203.0.113.0/24 (documentation)
    // - 192.88.99.0/24 (deprecated 6to4 relay anycast)
    // - 198.18.0.0/15 (benchmarking)
    // - 240.0.0.0/4 (reserved)
    matches!(
        ip.octets(),
        [0, _, _, _]
            | [192, 0, 0 | 2, _]
            | [192, 88, 99, _]
            | [198, 18..=19, _, _]
            | [198, 51, 100, _]
            | [203, 0, 113, _]
            | [240..=255, _, _, _]
    )
}

fn is_non_public_ipv6(ip: Ipv6Addr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return true;
    }

    // IANA IPv6 Special-Purpose Address Registry: ::ffff:0:0/96
    // (IPv4-mapped). Classify the address the socket will actually reach.
    if let Some(ipv4) = ip.to_ipv4_mapped() {
        return is_non_public_ipv4(ipv4);
    }

    let segments = ip.segments();
    // Deprecated IPv4-compatible IPv6 addresses occupy ::/96. Unlike mapped
    // addresses, these must not be accepted even when the trailing IPv4 is
    // public.
    if segments[..6].iter().all(|segment| *segment == 0) {
        return true;
    }
    if is_local_nat64_ipv6(segments) {
        return true;
    }
    if is_well_known_nat64_ipv6(segments) {
        return is_non_public_ipv4(ipv4_from_segments(segments[6], segments[7]));
    }
    if is_6to4_ipv6(segments) {
        return is_non_public_ipv4(ipv4_from_segments(segments[1], segments[2]));
    }

    let first = segments[0];
    if (first & 0xfe00) == 0xfc00
        || (first & 0xffc0) == 0xfe80
        || is_benchmarking_ipv6(segments)
        || is_ietf_protocol_assignment_ipv6(segments)
        || is_documentation_ipv6(segments)
        || is_srv6_sid_ipv6(segments)
        || is_discard_only_ipv6(segments)
    {
        return true;
    }

    // Globally routable IPv6 unicast space is currently allocated from
    // 2000::/3. The translation prefixes above are handled before this gate.
    if (first & 0xe000) != 0x2000 {
        return true;
    }

    false
}

fn is_local_nat64_ipv6(segments: [u16; 8]) -> bool {
    // IANA IPv6 Special-Purpose Address Registry: 64:ff9b:1::/48,
    // IPv4-IPv6 translation for local use, is not globally reachable.
    segments[0] == 0x0064 && segments[1] == 0xff9b && segments[2] == 0x0001
}

fn is_well_known_nat64_ipv6(segments: [u16; 8]) -> bool {
    // IANA IPv6 Special-Purpose Address Registry: 64:ff9b::/96,
    // the well-known IPv4-IPv6 translation prefix. Its embedded IPv4 address
    // determines whether the destination is public.
    segments[0] == 0x0064
        && segments[1] == 0xff9b
        && segments[2] == 0
        && segments[3] == 0
        && segments[4] == 0
        && segments[5] == 0
}

fn is_6to4_ipv6(segments: [u16; 8]) -> bool {
    // IANA IPv6 Special-Purpose Address Registry: 2002::/16 (6to4).
    // The next 32 bits carry the IPv4 destination to classify.
    segments[0] == 0x2002
}

fn is_ietf_protocol_assignment_ipv6(segments: [u16; 8]) -> bool {
    // IANA IPv6 Special-Purpose Address Registry: 2001::/23. This includes
    // 2001::/32 (Teredo) and 2001:2::/48 (benchmarking).
    segments[0] == 0x2001 && (segments[1] & 0xfe00) == 0
}

fn is_documentation_ipv6(segments: [u16; 8]) -> bool {
    // IANA IPv6 Special-Purpose Address Registry documentation blocks:
    // 2001:db8::/32 and 3fff::/20.
    (segments[0] == 0x2001 && segments[1] == 0x0db8)
        || (segments[0] == 0x3fff && (segments[1] & 0xf000) == 0)
}

fn is_srv6_sid_ipv6(segments: [u16; 8]) -> bool {
    // IANA IPv6 Special-Purpose Address Registry: 5f00::/16 (SRv6 SIDs).
    segments[0] == 0x5f00
}

fn is_discard_only_ipv6(segments: [u16; 8]) -> bool {
    // IANA IPv6 Special-Purpose Address Registry: 100::/64 (discard-only).
    segments[0] == 0x0100 && segments[1] == 0 && segments[2] == 0 && segments[3] == 0
}

fn is_benchmarking_ipv6(segments: [u16; 8]) -> bool {
    // IANA IPv6 Special-Purpose Address Registry: 2001:2::/48 (benchmarking).
    segments[0] == 0x2001 && segments[1] == 0x0002 && segments[2] == 0
}

fn ipv4_from_segments(high: u16, low: u16) -> Ipv4Addr {
    let [a, b] = high.to_be_bytes();
    let [c, d] = low.to_be_bytes();
    Ipv4Addr::new(a, b, c, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        time::{Duration, Instant},
    };

    #[test]
    fn public_ipv4_and_ipv6_are_allowed() {
        assert!(!is_non_public_ip(IpAddr::V4(Ipv4Addr::new(
            93, 184, 216, 34
        ))));
        assert!(!is_non_public_ip(IpAddr::V6(
            "2606:2800:220:1:248:1893:25c8:1946".parse().unwrap()
        )));
    }

    #[test]
    fn public_embedded_ipv4_destinations_are_allowed() {
        for ip in ["::ffff:5db8:d822", "64:ff9b::5db8:d822", "2002:5db8:d822::"] {
            assert!(!is_non_public_ip(IpAddr::V6(ip.parse().unwrap())), "{ip}");
        }
    }

    #[test]
    fn rejects_non_https_urls() {
        assert!(validate_video_download_url("http://example.com/video.mp4").is_err());
        assert!(validate_video_download_url("file:///tmp/video.mp4").is_err());
        assert!(validate_video_download_url("data:text/plain,video").is_err());
    }

    #[test]
    fn rejects_non_public_ip_literals() {
        for url in [
            "https://127.0.0.1/video.mp4",
            "https://10.0.0.5/video.mp4",
            "https://192.168.1.1/video.mp4",
            "https://169.254.169.254/latest/meta-data",
            "https://100.64.0.1/video.mp4",
            "https://100.127.255.255/video.mp4",
            "https://[::1]/video.mp4",
            "https://[fd00::1]/video.mp4",
            "https://[fe80::1]/video.mp4",
            "https://[::ffff:127.0.0.1]/video.mp4",
            "https://[::ffff:10.0.0.5]/video.mp4",
        ] {
            assert!(validate_video_download_url(url).is_err(), "{url}");
        }
    }

    #[test]
    fn rejects_this_network_ipv4_range() {
        assert_non_public_urls(&[
            "https://0.0.0.1/video.mp4",
            "https://0.255.255.255/video.mp4",
        ]);
    }

    #[test]
    fn rejects_ietf_protocol_assignments_ipv4_range() {
        assert_non_public_urls(&[
            "https://192.0.0.1/video.mp4",
            "https://192.0.0.255/video.mp4",
        ]);
    }

    #[test]
    fn rejects_test_net_1_ipv4_range() {
        assert_non_public_urls(&[
            "https://192.0.2.1/video.mp4",
            "https://192.0.2.255/video.mp4",
        ]);
    }

    #[test]
    fn rejects_deprecated_6to4_relay_anycast_ipv4_range() {
        assert_non_public_urls(&[
            "https://192.88.99.1/video.mp4",
            "https://192.88.99.255/video.mp4",
        ]);
    }

    #[test]
    fn rejects_benchmarking_ipv4_range() {
        assert_non_public_urls(&[
            "https://198.18.0.1/video.mp4",
            "https://198.19.255.255/video.mp4",
        ]);
    }

    #[test]
    fn rejects_test_net_2_ipv4_range() {
        assert_non_public_urls(&[
            "https://198.51.100.1/video.mp4",
            "https://198.51.100.255/video.mp4",
        ]);
    }

    #[test]
    fn rejects_test_net_3_ipv4_range() {
        assert_non_public_urls(&[
            "https://203.0.113.1/video.mp4",
            "https://203.0.113.255/video.mp4",
        ]);
    }

    #[test]
    fn rejects_reserved_ipv4_range() {
        assert_non_public_urls(&[
            "https://240.0.0.1/video.mp4",
            "https://255.255.255.254/video.mp4",
        ]);
    }

    #[test]
    fn rejects_ipv4_mapped_ipv6_with_non_public_destination() {
        assert_non_public_urls(&[
            "https://[::ffff:a00:1]/video.mp4",
            "https://[::ffff:c000:201]/video.mp4",
        ]);
    }

    #[test]
    fn rejects_ipv4_compatible_ipv6_range() {
        assert_non_public_urls(&["https://[::5db8:d822]/video.mp4"]);
    }

    #[test]
    fn rejects_well_known_nat64_with_non_public_destination() {
        assert_non_public_urls(&[
            "https://[64:ff9b::a00:1]/video.mp4",
            "https://[64:ff9b::c000:201]/video.mp4",
        ]);
    }

    #[test]
    fn rejects_local_use_nat64_ipv6_range() {
        assert_non_public_urls(&[
            "https://[64:ff9b:1::1]/video.mp4",
            "https://[64:ff9b:1:ffff:ffff:ffff:ffff:ffff]/video.mp4",
        ]);
    }

    #[test]
    fn rejects_discard_only_ipv6_range() {
        assert_non_public_urls(&[
            "https://[100::1]/video.mp4",
            "https://[100::ffff:ffff:ffff:ffff]/video.mp4",
        ]);
    }

    #[test]
    fn rejects_ietf_protocol_assignments_ipv6_range() {
        assert_non_public_urls(&[
            "https://[2001::1]/video.mp4",
            "https://[2001:1ff:ffff:ffff:ffff:ffff:ffff:ffff]/video.mp4",
        ]);
    }

    #[test]
    fn rejects_teredo_ipv6_range() {
        assert_non_public_urls(&[
            "https://[2001:0:1::1]/video.mp4",
            "https://[2001:0:ffff:ffff:ffff:ffff:ffff:ffff]/video.mp4",
        ]);
    }

    #[test]
    fn rejects_benchmarking_ipv6_range() {
        assert_non_public_urls(&[
            "https://[2001:2::1]/video.mp4",
            "https://[2001:2:0:ffff:ffff:ffff:ffff:ffff]/video.mp4",
        ]);
    }

    #[test]
    fn rejects_documentation_2001_db8_ipv6_range() {
        assert_non_public_urls(&[
            "https://[2001:db8::1]/video.mp4",
            "https://[2001:db8:ffff:ffff:ffff:ffff:ffff:ffff]/video.mp4",
        ]);
    }

    #[test]
    fn rejects_6to4_with_non_public_destination() {
        assert_non_public_urls(&[
            "https://[2002:a00:1::]/video.mp4",
            "https://[2002:c000:201::]/video.mp4",
        ]);
    }

    #[test]
    fn rejects_documentation_3fff_ipv6_range() {
        assert_non_public_urls(&[
            "https://[3fff::1]/video.mp4",
            "https://[3fff:fff:ffff:ffff:ffff:ffff:ffff:ffff]/video.mp4",
        ]);
    }

    #[test]
    fn rejects_srv6_sid_ipv6_range() {
        assert_non_public_urls(&[
            "https://[5f00::1]/video.mp4",
            "https://[5f00:ffff:ffff:ffff:ffff:ffff:ffff:ffff]/video.mp4",
        ]);
    }

    #[test]
    fn rejects_unallocated_global_ipv6_space() {
        assert_non_public_urls(&["https://[4000::1]/video.mp4"]);
    }

    #[test]
    fn allows_public_ip_literal() {
        let (_parsed, addrs) =
            validate_video_download_url("https://93.184.216.34/video.mp4").unwrap();
        assert_eq!(addrs, vec![SocketAddr::from(([93, 184, 216, 34], 443))]);

        let (_parsed, addrs) =
            validate_video_download_url("https://[2606:2800:220:1:248:1893:25c8:1946]/video.mp4")
                .unwrap();
        assert_eq!(
            addrs,
            vec!["[2606:2800:220:1:248:1893:25c8:1946]:443".parse().unwrap()]
        );
    }

    #[tokio::test]
    async fn pinned_video_download_client_does_not_follow_redirect_to_loopback() {
        let target_hit = Arc::new(AtomicBool::new(false));
        let (target_addr, target_thread) =
            spawn_one_response_server("HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nvideo", {
                let target_hit = Arc::clone(&target_hit);
                move || target_hit.store(true, Ordering::SeqCst)
            });
        let redirect_response = format!(
            "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{}/video.mp4\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            target_addr.port()
        );
        let (redirect_addr, redirect_thread) = spawn_one_response_server(&redirect_response, || {});
        let url = format!("http://example.com:{}/video.mp4", redirect_addr.port());
        let parsed = reqwest::Url::parse(&url).unwrap();
        let client = video_download_client_builder(&parsed, &[redirect_addr])
            .unwrap()
            .build()
            .unwrap();

        let response = client.get(parsed).send().await.unwrap();

        assert_eq!(response.status(), reqwest::StatusCode::FOUND);
        redirect_thread.join().unwrap();
        assert!(!target_hit.load(Ordering::SeqCst));
        target_thread.join().unwrap();
    }

    #[test]
    fn resolution_failure_is_distinct_from_a_rejected_address() {
        let url = "https://media.fal.media/result.mp4";
        assert_eq!(
            validate_with_resolver(url, |_, _| Err(std::io::ErrorKind::TimedOut.into()))
                .unwrap_err(),
            DownloadUrlError::Resolution
        );
        assert_eq!(
            validate_with_resolver(url, |_, _| Ok(vec![])).unwrap_err(),
            DownloadUrlError::Resolution
        );
        assert!(matches!(
            validate_with_resolver(url, |_, _| Ok(vec!["127.0.0.1:443".parse().unwrap()]))
                .unwrap_err(),
            DownloadUrlError::Rejected(_)
        ));
        assert!(
            validate_with_resolver(url, |_, _| Ok(vec!["93.184.216.34:443".parse().unwrap()]))
                .is_ok()
        );
    }

    fn assert_non_public_urls(urls: &[&str]) {
        for url in urls {
            assert_eq!(
                validate_video_download_url(url).unwrap_err(),
                DownloadUrlError::Rejected(
                    "video download URL host resolved to a non-public address".into()
                ),
                "{url}"
            );
        }
    }

    fn spawn_one_response_server(
        response: &str,
        on_request: impl FnOnce() + Send + 'static,
    ) -> (SocketAddr, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let response = response.to_string();
        let handle = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(2);
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(accepted) => break accepted,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if Instant::now() >= deadline {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("test server accept failed: {error}"),
                }
            };
            // Accepted sockets inherit O_NONBLOCK from the listener on macOS.
            // Restore blocking reads so a newly accepted request cannot race the
            // client write and fail this deterministic redirect test with EAGAIN.
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 1024];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let read = stream.read(&mut buffer).unwrap();
                assert!(
                    read > 0,
                    "test client closed before sending request headers"
                );
                request.extend_from_slice(&buffer[..read]);
            }
            on_request();
            stream.write_all(response.as_bytes()).unwrap();
            stream.flush().unwrap();
        });
        (addr, handle)
    }
}
