//! Fetching update bytes over HTTPS.

use crate::error::TransportError;
use crate::urls::ALLOWED_REDIRECT_HOSTS;
use reqwest::{Client, Response, Url};
use std::fs::File;
use std::future::Future;
use std::io::Write;
use std::path::Path;
use std::time::Duration;
use tokio::runtime::{Builder, Handle};

/// Ceiling on a manifest body. The published document is a few kilobytes of
/// JSON; anything approaching this is either a mistake or an attempt to make a
/// client buffer an unbounded response before it can check anything about it.
pub const MANIFEST_MAX_BYTES: u64 = 64 * 1024;

/// How many redirects one fetch may follow. GitHub needs exactly one to reach
/// its asset CDN; five leaves room for that to grow without ever becoming a
/// loop the client walks forever.
pub const MAX_REDIRECTS: usize = 5;

/// Establishing the connection is the part that hangs on a dead network.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

/// Per-read inactivity, deliberately *not* a whole-request timeout: `content`
/// is ~300 MB and any total budget large enough for it on a slow link would be
/// useless as a stall detector.
const READ_TIMEOUT: Duration = Duration::from_secs(60);

const UPDATE_USER_AGENT: &str = concat!("clonk-rs/", env!("CARGO_PKG_VERSION"), " (updater)");

/// Recreates reqwest 0.12's bundled-root policy explicitly. Reqwest 0.13 uses
/// platform verification by default; selecting rustls alone would therefore
/// let a modified system trust store authorize an update.
fn bundled_root_client_builder() -> Result<reqwest::ClientBuilder, reqwest::Error> {
    // `rustls-no-provider` keeps reqwest from pulling aws-lc-rs. Installing
    // ring once per process gives all builders the provider reqwest requires;
    // an embedding application that deliberately installed another provider
    // first keeps its choice.
    let _ = rustls::crypto::ring::default_provider().install_default();
    webpki_root_certs::TLS_SERVER_ROOT_CERTS
        .iter()
        .map(|certificate| reqwest::Certificate::from_der(certificate.as_ref()))
        .collect::<Result<Vec<_>, _>>()
        .map(|roots| {
            reqwest::Client::builder()
                .tls_backend_rustls()
                .tls_certs_only(roots)
        })
}

/// Moving update bytes, expressed so a caller can fake it.
///
/// Synchronous on purpose: every caller — the update dialog and the applying
/// binary — is ordinary blocking code, and an async trait here would push a
/// runtime into both of them for no gain.
pub trait UpdateTransport {
    /// Fetches a manifest document, refusing a body above
    /// [`MANIFEST_MAX_BYTES`].
    fn fetch_manifest(&self, url: &str) -> Result<Vec<u8>, TransportError>;

    /// Streams a component archive to `into`, reporting `(downloaded, total)`
    /// as it goes and returning the number of bytes written.
    ///
    /// `progress` returning `false` cancels: the transfer stops at the next
    /// chunk boundary, the partial file is removed, and
    /// [`TransportError::Cancelled`] is returned. Nothing is left at `into`
    /// after any failure, so a file there is always a complete response body —
    /// though only the SHA-256 `clonk-update` checks afterwards says it is the
    /// *right* body.
    fn download(
        &self,
        url: &str,
        into: &Path,
        progress: &mut dyn FnMut(u64, u64) -> bool,
    ) -> Result<u64, TransportError>;
}

/// The real transport: `reqwest` over `rustls`, presenting a blocking API.
///
/// It holds no runtime of its own. A runtime is built per call, *after* the
/// nesting check below, so one can never be created or dropped inside an async
/// context — dropping a `tokio` runtime there panics inside `tokio` itself,
/// where no error type of ours could intercept it.
#[derive(Debug, Clone)]
pub struct HttpTransport {
    client: Client,
}

impl HttpTransport {
    /// Builds the transport policy this crate wants: no client-side redirect
    /// following (this crate follows them itself, so the guards cannot be
    /// bypassed), no compression (component archives are already deflated, and
    /// a decompressing client makes the declared length a lie), no idle
    /// connection pooling (a pooled connection outlives the per-call runtime
    /// that was driving it, and reuse buys an updater making four requests
    /// nothing), and timeouts sized for a large download rather than a small
    /// request.
    pub fn new() -> Result<Self, TransportError> {
        bundled_root_client_builder()
            .map_err(TransportError::Client)?
            .redirect(reqwest::redirect::Policy::none())
            .no_gzip()
            .user_agent(UPDATE_USER_AGENT)
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(READ_TIMEOUT)
            .pool_max_idle_per_host(0)
            .build()
            .map(Self::with_client)
            .map_err(TransportError::Client)
    }

    /// Uses a caller-supplied client, the seam for proxy or TLS policy this
    /// crate does not own — and for tests.
    ///
    /// The redirect guards do not depend on the supplied policy: wherever the
    /// client ends up, that URL is checked before its body is read.
    pub fn with_client(client: Client) -> Self {
        Self { client }
    }

    /// Runs one request to completion on a runtime built for it, refusing to
    /// nest inside a runtime that is already running — `Runtime::block_on`
    /// aborts the process there.
    fn block_on<F: Future>(&self, future: F) -> Result<F::Output, TransportError> {
        Handle::try_current()
            .is_err()
            .then_some(())
            .ok_or(TransportError::NestedRuntime)
            .and_then(|()| {
                Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(TransportError::RuntimeUnavailable)
            })
            .map(|runtime| runtime.block_on(future))
    }

    /// GETs `url`, following redirects under [`check_hop`].
    async fn get(&self, url: &str) -> Result<Response, TransportError> {
        let origin = parse_url(url)?;
        check_origin(&origin)?;
        let mut current = origin.clone();
        for hop in 0..=MAX_REDIRECTS {
            let response = self
                .client
                .get(current.clone())
                .header(reqwest::header::ACCEPT_ENCODING, "identity")
                .send()
                .await
                .map_err(|source| TransportError::Request {
                    url: current.to_string(),
                    source,
                })?;
            // Wherever the client actually landed — which is not necessarily
            // where we sent it, since `with_client` may follow redirects on its
            // own — is checked before a single body byte is read.
            check_hop(&origin, response.url())?;
            if !response.status().is_redirection() {
                return succeeded(response);
            }
            if hop == MAX_REDIRECTS {
                break;
            }
            let next = redirect_target(&current, &response)?;
            check_hop(&origin, &next)?;
            tracing::debug!(from = %current, to = %next, hop, "update fetch follows a redirect");
            current = next;
        }
        Err(TransportError::TooManyRedirects {
            url: origin.to_string(),
            limit: MAX_REDIRECTS,
        })
    }

    /// The body of [`UpdateTransport::download`], less the cleanup.
    async fn stream_to_file(
        &self,
        url: &str,
        into: &Path,
        progress: &mut dyn FnMut(u64, u64) -> bool,
    ) -> Result<u64, TransportError> {
        let mut response = self.get(url).await?;
        let declared = response
            .content_length()
            .ok_or_else(|| TransportError::UndeclaredSize {
                url: response.url().to_string(),
            })?;
        // Give the caller the server's ceiling before creating the destination
        // or accepting a body chunk. The update planner compares this with the
        // published manifest size and can reject a mismatched asset without
        // spending disk space on it first.
        progress(0, declared)
            .then_some(())
            .ok_or(TransportError::Cancelled)?;
        let mut file = File::create(into).map_err(|source| TransportError::Io {
            path: into.to_path_buf(),
            source,
        })?;
        let mut written = 0u64;
        while let Some(chunk) =
            response
                .chunk()
                .await
                .map_err(|source| TransportError::Request {
                    url: url.to_owned(),
                    source,
                })?
        {
            written += chunk.len() as u64;
            within_declared(written, declared)?;
            file.write_all(&chunk)
                .map_err(|source| TransportError::Io {
                    path: into.to_path_buf(),
                    source,
                })?;
            // Between chunks, never mid-write: a cancelled transfer leaves a
            // whole number of chunks on disk right up until they are deleted.
            progress(written, declared)
                .then_some(())
                .ok_or(TransportError::Cancelled)?;
        }
        file.flush().map_err(|source| TransportError::Io {
            path: into.to_path_buf(),
            source,
        })?;
        Ok(written)
    }
}

impl UpdateTransport for HttpTransport {
    fn fetch_manifest(&self, url: &str) -> Result<Vec<u8>, TransportError> {
        tracing::debug!(url, "fetching the update manifest");
        self.block_on(async {
            let response = self.get(url).await?;
            // A truthful server is refused on its own header, before a body
            // arrives; a silent one is caught chunk by chunk below.
            within_manifest_cap(response.content_length())?;
            read_capped(response, MANIFEST_MAX_BYTES).await
        })?
    }

    fn download(
        &self,
        url: &str,
        into: &Path,
        progress: &mut dyn FnMut(u64, u64) -> bool,
    ) -> Result<u64, TransportError> {
        tracing::debug!(url, into = %into.display(), "downloading an update component");
        self.block_on(self.stream_to_file(url, into, progress))?
            .inspect_err(|_| {
                // Whatever went wrong, an incomplete body at the destination
                // is worse than no body: the next run would hash it, or worse,
                // unpack it. Removing a path that was never created is fine.
                let _ = std::fs::remove_file(into);
            })
    }
}

fn parse_url(url: &str) -> Result<Url, TransportError> {
    Url::parse(url).map_err(|_| TransportError::InvalidUrl {
        url: url.to_owned(),
    })
}

/// TLS authenticates the unsigned manifest that authorizes executable
/// replacement. Plain HTTP is retained only for an in-process loopback fixture,
/// where no network peer can answer in the server's place.
fn check_origin(origin: &Url) -> Result<(), TransportError> {
    check_origin_with_loopback_fixtures(origin, cfg!(test))
}

fn check_origin_with_loopback_fixtures(
    origin: &Url,
    allow_loopback_http: bool,
) -> Result<(), TransportError> {
    let loopback_http = allow_loopback_http
        && origin.scheme() == "http"
        && origin.host_str().is_some_and(|host| {
            host.eq_ignore_ascii_case("localhost")
                || host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|address| address.is_loopback())
        });
    (origin.scheme() == "https" || loopback_http)
        .then_some(())
        .ok_or_else(|| TransportError::InsecureOrigin {
            url: origin.to_string(),
        })
}

/// Rejects a response whose status is not a success, keeping the code so the
/// caller can tell "not published yet" from "server is broken".
fn succeeded(response: Response) -> Result<Response, TransportError> {
    let status = response.status();
    status
        .is_success()
        .then_some(())
        .ok_or_else(|| TransportError::Status {
            url: response.url().to_string(),
            status: status.as_u16(),
        })
        .map(|()| response)
}

/// Resolves a redirect's `Location` against the URL it came from, which is what
/// makes a relative `Location` — entirely legal — work.
fn redirect_target(current: &Url, response: &Response) -> Result<Url, TransportError> {
    response
        .headers()
        .get(reqwest::header::LOCATION)
        .ok_or_else(|| TransportError::RedirectWithoutLocation {
            url: current.to_string(),
        })
        .and_then(|location| {
            location
                .to_str()
                .ok()
                .and_then(|location| current.join(location).ok())
                .ok_or_else(|| TransportError::InvalidUrl {
                    url: String::from_utf8_lossy(location.as_bytes()).into_owned(),
                })
        })
}

/// Decides whether a fetch may move to `target`.
///
/// A server may redirect within its own origin — that is the shape of a mirror,
/// and of every local test — but leaving it is only allowed towards a known
/// release host over TLS. Without a manifest signature, an unchecked redirect
/// is the whole attack: it would let anyone who can answer one plaintext
/// request choose which bytes get installed.
fn check_hop(origin: &Url, target: &Url) -> Result<(), TransportError> {
    if same_origin(origin, target) {
        return Ok(());
    }
    (target.scheme() == "https")
        .then_some(())
        .ok_or_else(|| TransportError::InsecureRedirect {
            url: target.to_string(),
        })?;
    let host = target.host_str().unwrap_or_default();
    ALLOWED_REDIRECT_HOSTS
        .contains(&host)
        .then_some(())
        .ok_or_else(|| TransportError::RedirectHostNotAllowed {
            host: host.to_owned(),
            url: target.to_string(),
        })
}

fn same_origin(left: &Url, right: &Url) -> bool {
    left.scheme() == right.scheme()
        && left.host_str() == right.host_str()
        && left.port_or_known_default() == right.port_or_known_default()
}

fn within_manifest_cap(declared: Option<u64>) -> Result<(), TransportError> {
    declared
        .filter(|size| *size > MANIFEST_MAX_BYTES)
        .map_or(Ok(()), |size| {
            Err(TransportError::ManifestTooLarge {
                declared: Some(size),
                limit: MANIFEST_MAX_BYTES,
            })
        })
}

/// Refuses to account for a byte past what the response declared.
fn within_declared(written: u64, declared: u64) -> Result<(), TransportError> {
    (written <= declared)
        .then_some(())
        .ok_or(TransportError::BodyLongerThanDeclared { declared, written })
}

/// Buffers a body, giving up the moment it passes `limit` rather than after.
async fn read_capped(mut response: Response, limit: u64) -> Result<Vec<u8>, TransportError> {
    let url = response.url().to_string();
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|source| TransportError::Request {
            url: url.clone(),
            source,
        })?
    {
        (body.len() as u64 + chunk.len() as u64 <= limit)
            .then_some(())
            .ok_or(TransportError::ManifestTooLarge {
                declared: None,
                limit,
            })?;
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    #[test]
    fn a_manifest_body_is_returned_verbatim() {
        let fixture = serve(vec![Reply::ok(b"{\"schema\":1}")]);
        let transport = fixture.transport();

        assert_eq!(
            transport
                .fetch_manifest(&fixture.url("/manifest.json"))
                .expect("fixture serves the manifest"),
            b"{\"schema\":1}"
        );
        assert!(fixture.requests()[0].starts_with("GET /manifest.json HTTP/1."));
    }

    #[test]
    fn production_transport_never_decodes_server_sent_gzip() {
        // `clonk-app` also links `clonk-network`, whose reqwest `gzip` feature
        // is unified into this crate. The updater must still treat response
        // bytes and Content-Length as the server sent them, even when a server
        // ignores `Accept-Encoding: identity`.
        const GZIP_MANIFEST: &[u8] = &[
            31, 139, 8, 0, 0, 0, 0, 0, 2, 19, 171, 86, 42, 78, 206, 72, 205, 77, 84, 178, 50, 172,
            5, 0, 140, 193, 251, 137, 12, 0, 0, 0,
        ];
        let fixture = serve(vec![Reply::encoded("gzip", GZIP_MANIFEST)]);
        let transport = fixture.transport();

        assert_eq!(
            transport
                .fetch_manifest(&fixture.url("/manifest.json"))
                .expect("fixture serves the encoded manifest"),
            GZIP_MANIFEST
        );
    }

    #[test]
    fn a_missing_manifest_is_reported_with_its_status() {
        // The updater UI distinguishes "nothing published yet" from a broken
        // connection, so the status has to survive as a number.
        let fixture = serve(vec![Reply::status("404 Not Found")]);
        let transport = fixture.transport();

        assert!(matches!(
            transport.fetch_manifest(&fixture.url("/manifest.json")),
            Err(TransportError::Status { status: 404, .. })
        ));
    }

    #[test]
    fn a_redirect_within_the_same_origin_is_followed() {
        let fixture = serve(vec![
            Reply::redirect("/releases/download/v0.4.0/manifest.json"),
            Reply::ok(b"{\"schema\":1}"),
        ]);
        let transport = fixture.transport();

        assert_eq!(
            transport
                .fetch_manifest(&fixture.url("/latest/download/manifest.json"))
                .expect("redirect is followed"),
            b"{\"schema\":1}"
        );
        assert_eq!(fixture.requests().len(), 2);
        assert!(fixture.requests()[1].starts_with("GET /releases/download/v0.4.0/manifest.json"));
    }

    #[test]
    fn a_redirect_to_a_host_outside_the_allowlist_is_refused() {
        // Refused before the request is made, so an attacker-chosen host is
        // never even resolved.
        let fixture = serve(vec![Reply::redirect(
            "https://evil.example.com/manifest.json",
        )]);
        let transport = fixture.transport();

        assert!(matches!(
            transport.fetch_manifest(&fixture.url("/manifest.json")),
            Err(TransportError::RedirectHostNotAllowed { ref host, .. }) if host == "evil.example.com"
        ));
        assert_eq!(fixture.requests().len(), 1);
    }

    #[test]
    fn a_plaintext_redirect_off_the_origin_is_refused() {
        // The host is allowlisted; the scheme is not. TLS is the entire trust
        // story for an unsigned manifest, so a downgrade ends the fetch.
        let fixture = serve(vec![Reply::redirect(
            "http://objects.githubusercontent.com/manifest.json",
        )]);
        let transport = fixture.transport();

        assert!(matches!(
            transport.fetch_manifest(&fixture.url("/manifest.json")),
            Err(TransportError::InsecureRedirect { .. })
        ));
    }

    #[test]
    fn a_plaintext_non_loopback_origin_is_refused_before_any_request() {
        let origin = Url::parse("http://updates.example/manifest.json").expect("origin parses");

        assert!(matches!(
            check_origin(&origin),
            Err(TransportError::InsecureOrigin { .. })
        ));
    }

    #[test]
    fn production_policy_refuses_plaintext_loopback_origins() {
        for url in [
            "http://127.0.0.1/manifest.json",
            "http://localhost/manifest.json",
        ] {
            let origin = Url::parse(url).expect("origin parses");
            assert!(matches!(
                check_origin_with_loopback_fixtures(&origin, false),
                Err(TransportError::InsecureOrigin { .. })
            ));
        }
    }

    #[test]
    fn a_redirect_to_a_non_http_scheme_is_refused() {
        let fixture = serve(vec![Reply::redirect("file:///etc/passwd")]);
        let transport = fixture.transport();

        assert!(matches!(
            transport.fetch_manifest(&fixture.url("/manifest.json")),
            Err(TransportError::InsecureRedirect { .. })
        ));
    }

    #[test]
    fn a_redirect_chain_longer_than_the_limit_is_refused() {
        let fixture = serve(
            (0..MAX_REDIRECTS + 2)
                .map(|hop| Reply::redirect(&format!("/hop/{hop}")))
                .collect(),
        );
        let transport = fixture.transport();

        assert!(matches!(
            transport.fetch_manifest(&fixture.url("/manifest.json")),
            Err(TransportError::TooManyRedirects {
                limit: MAX_REDIRECTS,
                ..
            })
        ));
        // The first request plus MAX_REDIRECTS follow-ups, and no more.
        assert_eq!(fixture.requests().len(), MAX_REDIRECTS + 1);
    }

    #[test]
    fn a_redirect_without_a_location_is_refused() {
        let fixture = serve(vec![Reply::status("302 Found")]);
        let transport = fixture.transport();

        assert!(matches!(
            transport.fetch_manifest(&fixture.url("/manifest.json")),
            Err(TransportError::RedirectWithoutLocation { .. })
        ));
    }

    #[test]
    fn a_manifest_declaring_an_oversized_body_is_refused_before_it_is_read() {
        let fixture = serve(vec![Reply::mismatched(MANIFEST_MAX_BYTES + 1, &[b'x'; 16])]);
        let transport = fixture.transport();

        assert!(matches!(
            transport.fetch_manifest(&fixture.url("/manifest.json")),
            Err(TransportError::ManifestTooLarge {
                limit: MANIFEST_MAX_BYTES,
                ..
            })
        ));
    }

    #[test]
    fn a_manifest_that_streams_past_the_cap_without_declaring_a_size_is_refused() {
        // A server that declares nothing cannot be caught by the header check,
        // so the cap is also enforced chunk by chunk.
        let oversized = vec![b'x'; (MANIFEST_MAX_BYTES + 1) as usize];
        let fixture = serve(vec![Reply::undeclared(&oversized)]);
        let transport = fixture.transport();

        assert!(matches!(
            transport.fetch_manifest(&fixture.url("/manifest.json")),
            Err(TransportError::ManifestTooLarge {
                limit: MANIFEST_MAX_BYTES,
                ..
            })
        ));
    }

    #[test]
    fn a_manifest_exactly_at_the_cap_is_accepted() {
        let exact = vec![b'x'; MANIFEST_MAX_BYTES as usize];
        let fixture = serve(vec![Reply::ok(&exact)]);
        let transport = fixture.transport();

        assert_eq!(
            transport
                .fetch_manifest(&fixture.url("/manifest.json"))
                .expect("the cap is inclusive")
                .len(),
            exact.len()
        );
    }

    #[test]
    fn a_client_that_follows_redirects_itself_still_lands_on_a_checked_url() {
        // `with_client` hands redirect policy to the caller, so the guard
        // cannot live only in the follow loop: whatever URL the response
        // actually came from is checked before its body is read.
        let elsewhere = serve(vec![Reply::ok(b"{}")]);
        let fixture = serve(vec![Reply::redirect(&elsewhere.url("/manifest.json"))]);
        let transport = HttpTransport::with_client(
            bundled_root_client_builder()
                .expect("bundled roots parse")
                .redirect(reqwest::redirect::Policy::limited(10))
                .build()
                .expect("build a redirect-following client"),
        );

        assert!(matches!(
            transport.fetch_manifest(&fixture.url("/manifest.json")),
            Err(TransportError::InsecureRedirect { .. })
        ));
    }

    #[test]
    fn an_allowlisted_https_hop_is_accepted() {
        let origin = reqwest::Url::parse("https://github.com/clonk-org/clonk-rs/releases/latest")
            .expect("origin parses");
        for target in [
            "https://objects.githubusercontent.com/x?token=1",
            "https://release-assets.githubusercontent.com/x",
            "https://github.com/clonk-org/clonk-rs/releases/download/v0.4.0/planet.zip",
        ] {
            let target = reqwest::Url::parse(target).expect("target parses");
            assert!(
                check_hop(&origin, &target).is_ok(),
                "hop to {target} must be accepted"
            );
        }
    }

    #[test]
    fn a_component_is_written_to_disk_and_reports_its_byte_progress() {
        let body = vec![b'z'; 40_000];
        let fixture = serve(vec![Reply::ok(&body)]);
        let transport = fixture.transport();
        let directory = tempfile::tempdir().expect("temporary install cache");
        let into = directory.path().join("content.zip");

        let mut seen = Vec::new();
        let written = {
            let mut progress = |done, total| {
                seen.push((done, total));
                true
            };
            transport
                .download(&fixture.url("/content.zip"), &into, &mut progress)
                .expect("fixture serves the component")
        };

        assert_eq!(written, body.len() as u64);
        assert_eq!(std::fs::read(&into).expect("component is on disk"), body);
        assert_eq!(
            seen.last().copied(),
            Some((body.len() as u64, body.len() as u64))
        );
        assert!(
            seen.iter()
                .all(|(done, total)| *total == body.len() as u64 && *done <= body.len() as u64),
            "progress never overstates the transfer: {seen:?}"
        );
    }

    #[test]
    fn a_component_reports_its_declared_size_before_writing_a_body_chunk() {
        let body = vec![b'z'; 40_000];
        let fixture = serve(vec![Reply::ok(&body)]);
        let transport = fixture.transport();
        let directory = tempfile::tempdir().expect("temporary install cache");
        let into = directory.path().join("content.zip");
        let mut seen = Vec::new();

        let outcome =
            transport.download(&fixture.url("/content.zip"), &into, &mut |done, total| {
                seen.push((done, total));
                false
            });

        assert!(matches!(outcome, Err(TransportError::Cancelled)));
        assert_eq!(seen, [(0, body.len() as u64)]);
        assert!(!into.exists(), "cancellation precedes the first body write");
    }

    #[test]
    fn cancelling_from_the_progress_callback_deletes_the_partial_file() {
        // A cancelled download must not leave bytes behind that a later run
        // could mistake for a complete component.
        let fixture = serve(vec![Reply::ok(&vec![b'z'; 200_000])]);
        let transport = fixture.transport();
        let directory = tempfile::tempdir().expect("temporary install cache");
        let into = directory.path().join("content.zip");

        let mut saw_body = false;
        let outcome =
            transport.download(&fixture.url("/content.zip"), &into, &mut |downloaded, _| {
                if downloaded == 0 {
                    true
                } else {
                    saw_body = true;
                    false
                }
            });

        assert!(matches!(outcome, Err(TransportError::Cancelled)));
        assert!(saw_body, "cancellation follows at least one body write");
        assert!(!into.exists(), "the partial download is removed");
    }

    #[test]
    fn a_component_without_a_declared_size_is_refused() {
        // Without a length there is no ceiling on what a server can make the
        // client write to disk, and no honest progress to report.
        let fixture = serve(vec![Reply::undeclared(b"partial component")]);
        let transport = fixture.transport();
        let directory = tempfile::tempdir().expect("temporary install cache");
        let into = directory.path().join("content.zip");

        assert!(matches!(
            transport.download(&fixture.url("/content.zip"), &into, &mut |_, _| true),
            Err(TransportError::UndeclaredSize { .. })
        ));
        assert!(!into.exists(), "nothing is left behind");
    }

    #[test]
    fn a_body_longer_than_its_declared_size_never_reaches_the_disk() {
        // The client itself stops at `Content-Length`, and the transport
        // refuses to write past it regardless; between them the file can never
        // exceed what the response declared.
        let fixture = serve(vec![Reply::mismatched(4, b"123456789")]);
        let transport = fixture.transport();
        let directory = tempfile::tempdir().expect("temporary install cache");
        let into = directory.path().join("content.zip");

        let outcome = transport.download(&fixture.url("/content.zip"), &into, &mut |_, _| true);

        let written = outcome.unwrap_or(u64::MAX);
        assert!(written <= 4, "wrote {written} bytes against a declared 4");
    }

    #[test]
    fn writing_past_the_declared_size_is_refused() {
        // Directly, because a conforming HTTP client truncates at
        // `Content-Length` before this guard can fire. It exists for the case
        // that stops being true — a transfer-decoding client makes the
        // declared length a lie, which is why `Accept-Encoding: identity` is
        // sent — and an unenforced limit is not a limit.
        assert!(within_declared(4, 4).is_ok());
        assert!(matches!(
            within_declared(5, 4),
            Err(TransportError::BodyLongerThanDeclared { declared: 4, .. })
        ));
    }

    #[test]
    fn a_failed_component_request_leaves_no_file_behind() {
        let fixture = serve(vec![Reply::status("404 Not Found")]);
        let transport = fixture.transport();
        let directory = tempfile::tempdir().expect("temporary install cache");
        let into = directory.path().join("content.zip");

        assert!(matches!(
            transport.download(&fixture.url("/content.zip"), &into, &mut |_, _| true),
            Err(TransportError::Status { status: 404, .. })
        ));
        assert!(!into.exists(), "nothing is left behind");
    }

    #[test]
    fn a_component_download_is_bound_by_the_same_redirect_guards() {
        let fixture = serve(vec![Reply::redirect(
            "https://evil.example.com/content.zip",
        )]);
        let transport = fixture.transport();
        let directory = tempfile::tempdir().expect("temporary install cache");
        let into = directory.path().join("content.zip");

        assert!(matches!(
            transport.download(&fixture.url("/content.zip"), &into, &mut |_, _| true),
            Err(TransportError::RedirectHostNotAllowed { .. })
        ));
    }

    #[test]
    fn the_trait_can_be_faked_and_used_as_an_object() {
        // The point of the trait: callers that own decision and apply logic
        // depend on `&dyn UpdateTransport`, so their tests script faults —
        // truncation, cancellation, a 404 — without a socket in sight. If this
        // stops compiling, every one of those callers is forced onto the real
        // network.
        struct Faulty;

        impl UpdateTransport for Faulty {
            fn fetch_manifest(&self, _url: &str) -> Result<Vec<u8>, TransportError> {
                Ok(b"{\"schema\":1}".to_vec())
            }

            fn download(
                &self,
                _url: &str,
                _into: &Path,
                progress: &mut dyn FnMut(u64, u64) -> bool,
            ) -> Result<u64, TransportError> {
                progress(1, 2).then_some(1).ok_or(TransportError::Cancelled)
            }
        }

        let transport: &dyn UpdateTransport = &Faulty;
        assert_eq!(
            transport
                .fetch_manifest("https://example.invalid/manifest.json")
                .expect("the fake answers"),
            b"{\"schema\":1}"
        );
        assert!(matches!(
            transport.download(
                "https://example.invalid/c.zip",
                Path::new("c.zip"),
                &mut |_, _| { false }
            ),
            Err(TransportError::Cancelled)
        ));
    }

    #[tokio::test]
    async fn blocking_inside_an_async_runtime_is_refused_rather_than_panicking() {
        // `Runtime::block_on` panics when it is nested. The updater is called
        // from the app's synchronous UI thread, but a caller inside a runtime
        // gets an error it can handle rather than an abort.
        let transport = HttpTransport::new().expect("transport starts");
        assert!(matches!(
            transport.fetch_manifest("https://github.com/never-requested"),
            Err(TransportError::NestedRuntime)
        ));
    }

    /// A scripted local HTTP server. Each reply is served on its own
    /// connection, in order; once the script runs out every further request is
    /// answered `500` so an over-eager client fails instead of hanging.
    struct Fixture {
        base: String,
        requests: Arc<Mutex<Vec<String>>>,
    }

    impl Fixture {
        fn url(&self, path: &str) -> String {
            format!("{}{path}", self.base)
        }

        fn requests(&self) -> Vec<String> {
            self.requests.lock().expect("fixture request log").clone()
        }

        /// A transport with the crate's own redirect policy, pointed at no host
        /// in particular — the fixture URL is supplied per call.
        fn transport(&self) -> HttpTransport {
            HttpTransport::new().expect("transport starts")
        }
    }

    enum Reply {
        /// Status line and body, with the body's true length declared.
        Declared(String, Vec<u8>),
        /// A declared body carrying an HTTP content encoding.
        Encoded(String, Vec<u8>),
        /// A `Location` header, relative or absolute.
        Redirect(String),
        /// A body delimited only by end-of-stream.
        Undeclared(Vec<u8>),
        /// A `Content-Length` that disagrees with the body that follows.
        Mismatched(u64, Vec<u8>),
    }

    impl Reply {
        fn ok(body: &[u8]) -> Self {
            Self::Declared("200 OK".to_owned(), body.to_vec())
        }

        fn status(status: &str) -> Self {
            Self::Declared(status.to_owned(), Vec::new())
        }

        fn encoded(encoding: &str, body: &[u8]) -> Self {
            Self::Encoded(encoding.to_owned(), body.to_vec())
        }

        fn redirect(location: &str) -> Self {
            Self::Redirect(location.to_owned())
        }

        fn undeclared(body: &[u8]) -> Self {
            Self::Undeclared(body.to_vec())
        }

        fn mismatched(declared: u64, body: &[u8]) -> Self {
            Self::Mismatched(declared, body.to_vec())
        }
    }

    fn serve(replies: Vec<Reply>) -> Fixture {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind local update fixture");
        let address = listener.local_addr().expect("fixture address");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&requests);
        thread::spawn(move || {
            let mut replies = replies.into_iter();
            while let Ok((mut stream, _)) = listener.accept() {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let request = read_request_head(&mut stream);
                recorded.lock().expect("fixture request log").push(request);
                let reply = replies
                    .next()
                    .unwrap_or_else(|| Reply::status("500 Fixture Script Exhausted"));
                write_reply(&mut stream, &reply);
            }
        });
        Fixture {
            base: format!("http://{address}"),
            requests,
        }
    }

    fn read_request_head(stream: &mut TcpStream) -> String {
        let mut request = Vec::new();
        let mut buffer = [0u8; 1024];
        while !request.windows(4).any(|part| part == b"\r\n\r\n") {
            match stream.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(count) => request.extend_from_slice(&buffer[..count]),
            }
        }
        String::from_utf8_lossy(&request).into_owned()
    }

    fn write_reply(stream: &mut TcpStream, reply: &Reply) {
        let _ = match reply {
            Reply::Declared(status, body) => stream
                .write_all(
                    format!(
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .as_bytes(),
                )
                .and_then(|()| stream.write_all(body)),
            Reply::Encoded(encoding, body) => stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Encoding: {encoding}\r\n\
                         Content-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .as_bytes(),
                )
                .and_then(|()| stream.write_all(body)),
            Reply::Redirect(location) => stream.write_all(
                format!(
                    "HTTP/1.1 302 Found\r\nLocation: {location}\r\n\
                     Content-Length: 0\r\nConnection: close\r\n\r\n"
                )
                .as_bytes(),
            ),
            Reply::Undeclared(body) => stream
                .write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n")
                .and_then(|()| stream.write_all(body)),
            Reply::Mismatched(declared, body) => stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {declared}\r\n\
                         Connection: close\r\n\r\n"
                    )
                    .as_bytes(),
                )
                .and_then(|()| stream.write_all(body)),
        };
        let _ = stream.flush();
    }
}
