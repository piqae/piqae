use futures::TryStreamExt as _;
use spool_agent_core::{AgentError, ContentStore, StoredContent};
use spool_domain::UriAuthentication;
use std::{
    collections::BTreeSet,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    time::Duration,
};
use thiserror::Error;
use tokio_util::io::StreamReader;
use url::{Host, Url};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const DNS_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct UriFetcher {
    allow_private_sources: bool,
}

#[derive(Debug, Error)]
pub enum UriFetchError {
    #[error("content URI is invalid")]
    InvalidUri,
    #[error("only HTTP and HTTPS content URIs are supported")]
    InvalidScheme,
    #[error("credentials embedded in content URIs are not allowed")]
    EmbeddedCredentials,
    #[error("content URI host is blocked by agent policy")]
    BlockedHost,
    #[error("content URI DNS resolution failed")]
    ResolutionFailed,
    #[error("content URI resolved to a blocked destination")]
    BlockedDestination,
    #[error("content URI redirect was refused")]
    RedirectRefused,
    #[error("content URI request could not be completed")]
    Transport,
    #[error("content URI returned HTTP {0}")]
    HttpStatus(u16),
    #[error("content length {actual} exceeds the {limit} byte limit")]
    ContentTooLarge { actual: u64, limit: u64 },
    #[error("Digest URI authentication is not enabled in this build")]
    DigestAuthenticationUnsupported,
    #[error("content storage failed: {0}")]
    Content(#[from] AgentError),
}

impl UriFetcher {
    #[must_use]
    pub const fn new(allow_private_sources: bool) -> Self {
        Self {
            allow_private_sources,
        }
    }

    /// Fetches a URI through a DNS-pinned, redirect-free client and streams it
    /// into the bounded content-addressed store.
    ///
    /// # Errors
    ///
    /// Returns a redacted policy, transport, status, size, authentication, or
    /// content-store error. URI credentials are never retained in errors.
    pub async fn fetch_to_store(
        &self,
        content_store: &ContentStore,
        uri: &str,
        authentication: Option<&UriAuthentication>,
        expected_sha256: Option<&str>,
    ) -> Result<StoredContent, UriFetchError> {
        let response = self.fetch(uri, authentication).await?;
        let stream = response.bytes_stream().map_err(std::io::Error::other);
        if let Some(expected) = expected_sha256 {
            let path = content_store
                .put_verified(expected, StreamReader::new(stream))
                .await?;
            let bytes = tokio::fs::metadata(&path)
                .await
                .map_err(AgentError::from)?
                .len();
            Ok(StoredContent {
                sha256: expected.to_ascii_lowercase(),
                path,
                bytes,
            })
        } else {
            Ok(content_store.put(StreamReader::new(stream)).await?)
        }
    }

    async fn fetch(
        &self,
        uri: &str,
        authentication: Option<&UriAuthentication>,
    ) -> Result<reqwest::Response, UriFetchError> {
        let mut parsed_url = Url::parse(uri).map_err(|_| UriFetchError::InvalidUri)?;
        if !matches!(parsed_url.scheme(), "http" | "https") {
            return Err(UriFetchError::InvalidScheme);
        }
        if !parsed_url.username().is_empty() || parsed_url.password().is_some() {
            return Err(UriFetchError::EmbeddedCredentials);
        }
        let host_name = {
            let host = parsed_url.host().ok_or(UriFetchError::InvalidUri)?;
            normalized_host(&host)
        };
        if is_metadata_host(&host_name) {
            return Err(UriFetchError::BlockedHost);
        }
        parsed_url
            .set_host(Some(&host_name))
            .map_err(|_| UriFetchError::InvalidUri)?;
        let host = parsed_url.host().ok_or(UriFetchError::InvalidUri)?;
        let port = parsed_url
            .port_or_known_default()
            .ok_or(UriFetchError::InvalidUri)?;
        let addresses = self.resolve(&host, &host_name, port).await?;

        let mut builder = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy();
        if matches!(host, Host::Domain(_)) {
            builder = builder.resolve_to_addrs(&host_name, &addresses);
        }
        let client = builder.build().map_err(|_| UriFetchError::Transport)?;
        let mut request = client.get(parsed_url);
        match authentication {
            Some(UriAuthentication::Basic { username, password }) => {
                request = request.basic_auth(username, Some(password));
            }
            Some(UriAuthentication::Digest { .. }) => {
                return Err(UriFetchError::DigestAuthenticationUnsupported);
            }
            None => {}
        }
        let response = request.send().await.map_err(|_| UriFetchError::Transport)?;
        if response.status().is_redirection() {
            return Err(UriFetchError::RedirectRefused);
        }
        if !response.status().is_success() {
            return Err(UriFetchError::HttpStatus(response.status().as_u16()));
        }
        if let Some(actual) = response.content_length() {
            if actual > ContentStore::MAX_CONTENT_BYTES {
                return Err(UriFetchError::ContentTooLarge {
                    actual,
                    limit: ContentStore::MAX_CONTENT_BYTES,
                });
            }
        }
        Ok(response)
    }

    async fn resolve(
        &self,
        host: &Host<&str>,
        host_name: &str,
        port: u16,
    ) -> Result<Vec<SocketAddr>, UriFetchError> {
        let addresses = match host {
            Host::Ipv4(address) => vec![SocketAddr::new(IpAddr::V4(*address), port)],
            Host::Ipv6(address) => vec![SocketAddr::new(IpAddr::V6(*address), port)],
            Host::Domain(_) => {
                let resolved =
                    tokio::time::timeout(DNS_TIMEOUT, tokio::net::lookup_host((host_name, port)))
                        .await
                        .map_err(|_| UriFetchError::ResolutionFailed)?
                        .map_err(|_| UriFetchError::ResolutionFailed)?;
                resolved.collect()
            }
        };
        let addresses: Vec<_> = addresses
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if addresses.is_empty() {
            return Err(UriFetchError::ResolutionFailed);
        }
        if addresses
            .iter()
            .any(|address| !self.destination_allowed(address.ip()))
        {
            return Err(UriFetchError::BlockedDestination);
        }
        Ok(addresses)
    }

    fn destination_allowed(&self, address: IpAddr) -> bool {
        match address {
            IpAddr::V4(address) => ipv4_allowed(address, self.allow_private_sources),
            IpAddr::V6(address) => ipv6_allowed(address, self.allow_private_sources),
        }
    }
}

fn normalized_host(host: &Host<&str>) -> String {
    match host {
        Host::Domain(domain) => domain.trim_end_matches('.').to_ascii_lowercase(),
        Host::Ipv4(address) => address.to_string(),
        Host::Ipv6(address) => address.to_string(),
    }
}

fn is_metadata_host(host: &str) -> bool {
    matches!(
        host,
        "metadata.google.internal"
            | "metadata.google.com"
            | "instance-data.ec2.internal"
            | "metadata.azure.com"
            | "metadata.azure.internal"
    ) || host.ends_with(".metadata.google.internal")
}

fn ipv4_allowed(address: Ipv4Addr, allow_private: bool) -> bool {
    if address.is_unspecified() || address.is_multicast() || address == Ipv4Addr::BROADCAST {
        return false;
    }
    if is_cloud_metadata_v4(address) {
        return false;
    }
    allow_private || !is_non_public_v4(address)
}

const fn is_cloud_metadata_v4(address: Ipv4Addr) -> bool {
    matches!(
        address.octets(),
        [169, 254, 169, 254] | [169, 254, 170, 2] | [100, 100, 100, 200]
    )
}

fn is_non_public_v4(address: Ipv4Addr) -> bool {
    let [a, b, c, _] = address.octets();
    address.is_private()
        || address.is_loopback()
        || address.is_link_local()
        || a == 0
        || (a == 100 && (64..=127).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 192 && b == 0 && c == 2)
        || (a == 192 && b == 88 && c == 99)
        || (a == 198 && matches!(b, 18 | 19))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
        || a >= 240
}

fn ipv6_allowed(address: Ipv6Addr, allow_private: bool) -> bool {
    if address.is_unspecified() || address.is_multicast() {
        return false;
    }
    if is_cloud_metadata_v6(address) {
        return false;
    }
    if let Some(mapped) = address.to_ipv4_mapped() {
        return ipv4_allowed(mapped, allow_private);
    }
    if address.segments()[..6] == [0x0064, 0xff9b, 0, 0, 0, 0] {
        let segments = address.segments();
        let [a, b] = segments[6].to_be_bytes();
        let [c, d] = segments[7].to_be_bytes();
        let embedded = Ipv4Addr::new(a, b, c, d);
        return ipv4_allowed(embedded, allow_private);
    }
    allow_private || !is_non_public_v6(address)
}

const fn is_cloud_metadata_v6(address: Ipv6Addr) -> bool {
    matches!(
        address.segments(),
        [0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x0254] | [0xfd20, 0x00ce, 0, 0, 0, 0, 0, 0x0254]
    )
}

const fn is_non_public_v6(address: Ipv6Addr) -> bool {
    let segments = address.segments();
    address.is_loopback()
        || (segments[0] & 0xfe00) == 0xfc00
        || (segments[0] & 0xffc0) == 0xfe80
        || (segments[0] & 0xffc0) == 0xfec0
        || (segments[0] == 0x2001 && segments[1] == 0x0db8)
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use std::fmt::Write as _;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    #[tokio::test]
    async fn private_destinations_are_denied_by_default() {
        let directory = tempfile::tempdir().expect("tempdir");
        let store = ContentStore::open(directory.path()).await.expect("store");
        let error = UriFetcher::new(false)
            .fetch_to_store(&store, "http://127.0.0.1/document", None, None)
            .await
            .expect_err("loopback must be denied");
        assert!(matches!(error, UriFetchError::BlockedDestination));
        assert!(!ipv4_allowed(Ipv4Addr::new(10, 0, 0, 1), false));
        assert!(!ipv6_allowed(Ipv6Addr::LOCALHOST, false));
        assert!(!ipv6_allowed(
            "::ffff:10.0.0.1".parse().expect("mapped address"),
            false
        ));
    }

    #[tokio::test]
    async fn explicit_opt_in_can_reach_loopback() {
        let (url, server) = one_response(
            "200 OK",
            &[("Content-Type", "application/pdf")],
            b"print me",
        )
        .await;
        let directory = tempfile::tempdir().expect("tempdir");
        let store = ContentStore::open(directory.path()).await.expect("store");
        let content = UriFetcher::new(true)
            .fetch_to_store(&store, &url, None, None)
            .await
            .expect("trusted loopback fetch");
        server.await.expect("server");
        assert_eq!(
            tokio::fs::read(content.path).await.expect("content"),
            b"print me"
        );
    }

    #[tokio::test]
    async fn redirects_are_never_followed() {
        let (target, target_server) = one_response("200 OK", &[], b"secret").await;
        let (redirect, redirect_server) =
            one_response("302 Found", &[("Location", target.as_str())], b"").await;
        let directory = tempfile::tempdir().expect("tempdir");
        let store = ContentStore::open(directory.path()).await.expect("store");
        let error = UriFetcher::new(true)
            .fetch_to_store(&store, &redirect, None, None)
            .await
            .expect_err("redirect must be refused");
        redirect_server.await.expect("redirect server");
        target_server.abort();
        assert!(matches!(error, UriFetchError::RedirectRefused));
    }

    #[tokio::test]
    async fn declared_oversized_content_is_rejected_without_leaking_credentials() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let address = listener.local_addr().expect("address");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut request = [0_u8; 4096];
            let read = stream.read(&mut request).await.expect("read");
            let request = String::from_utf8_lossy(&request[..read]);
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("authorization: basic ")
            );
            let length = ContentStore::MAX_CONTENT_BYTES + 1;
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n"
                    )
                    .as_bytes(),
                )
                .await
                .expect("response");
        });
        let secret = "never-log-this-password";
        let authentication = UriAuthentication::Basic {
            username: "spool".into(),
            password: secret.into(),
        };
        let directory = tempfile::tempdir().expect("tempdir");
        let store = ContentStore::open(directory.path()).await.expect("store");
        let error = UriFetcher::new(true)
            .fetch_to_store(
                &store,
                &format!("http://{address}/oversized"),
                Some(&authentication),
                None,
            )
            .await
            .expect_err("oversized response");
        server.await.expect("server");
        assert!(matches!(error, UriFetchError::ContentTooLarge { .. }));
        assert!(!format!("{error:?}").contains(secret));
    }

    async fn one_response(
        status: &'static str,
        headers: &[(&str, &str)],
        body: &'static [u8],
    ) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let address = listener.local_addr().expect("address");
        let headers = headers
            .iter()
            .fold(String::new(), |mut output, (name, value)| {
                let _ = write!(output, "{name}: {value}\r\n");
                output
            });
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request).await.expect("read");
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .expect("headers");
            stream.write_all(body).await.expect("body");
        });
        (format!("http://{address}/document"), server)
    }
}
