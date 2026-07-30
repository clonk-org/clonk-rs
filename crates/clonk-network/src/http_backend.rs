//! `Network.UseCurl` selects between C++'s two HTTP client implementations.
//!
//! `C4Network2HTTPClient` picks one at construction
//! (`C4Network2Reference.cpp:410-413`):
//!
//! ```cpp
//! : impl{Config.Network.UseCurl
//!        ? IMPL(C4Network2HTTPClientImplCurl)
//!        : IMPL(C4Network2HTTPClientImplNetIO)}
//! ```
//!
//! The two are not the same client with different plumbing — they differ on the
//! wire, and those differences are what this module reproduces:
//!
//! | | curl (`C4HTTPClient.cpp:189-198`) | NetIO (`C4Network2Reference.cpp:404,825-856`) |
//! |---|---|---|
//! | redirects | `CURLOPT_FOLLOWLOCATION 1` — followed | **never followed**; there is no `Location` handling at all |
//! | cookies | `CURLOPT_COOKIEFILE ""` — in-memory jar | none |
//! | connections | `CURLOPT_SHARE` — reused across handles | `Connection: Close` on every request |
//! | protocol | whatever libcurl negotiates | `HTTP/1.0` |
//! | timeout | `CURLOPT_CONNECTTIMEOUT` plus a low-speed abort | one 20 s query timeout |
//! | `Accept-Encoding` | `gzip` | `gzip` |
//! | `User-Agent` | `C4ENGINENAME "/" C4VERSION` | the same |
//!
//! **Deliberate divergence.** This selects a differently-configured `reqwest`
//! client rather than hand-writing a second HTTP stack. The acceptance
//! criterion is observable request semantics ("preserve existing headers,
//! timeouts, redirects, and response decoding"), and a hand-rolled header and
//! gzip parser reading straight off the reference and league paths would be new
//! attack surface on network input — which this repo's own rules forbid
//! panicking on — duplicating what `reqwest` already does. One residual stays:
//! `reqwest` cannot emit an `HTTP/1.0` request line, so a server that
//! distinguishes the version still sees `HTTP/1.1` with `Connection: close`.
//! [`NETIO_HAPPY_EYEBALLS_TIMEOUT`] is exported for the same reason: this
//! `reqwest` has no builder for it, and its connector's own default already
//! matches C++'s 300 ms, so the constant documents the value rather than
//! setting it.

use std::time::Duration;

/// `C4Network2HTTPQueryTimeout` (`C4Network2Reference.cpp:405`).
pub const NETIO_QUERY_TIMEOUT: Duration = Duration::from_secs(20);

/// `C4Network2HTTPHappyEyeballsTimeout` (`C4Network2Reference.cpp:404`).
pub const NETIO_HAPPY_EYEBALLS_TIMEOUT: Duration = Duration::from_millis(300);

/// Which `C4Network2HTTPClient::Impl` the configuration asked for.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum HttpBackend {
    /// `C4Network2HTTPClientImplCurl`. `Network.UseCurl` defaults to `true`
    /// (`C4Config.cpp:561`), so this is the shipped behaviour.
    #[default]
    Curl,
    /// `C4Network2HTTPClientImplNetIO`.
    NetIo,
}

impl HttpBackend {
    /// Reads the `Network.UseCurl` key.
    pub fn from_use_curl(use_curl: bool) -> Self {
        if use_curl {
            Self::Curl
        } else {
            Self::NetIo
        }
    }

    /// Whether this backend follows `3xx` redirects.
    pub fn follows_redirects(self) -> bool {
        matches!(self, Self::Curl)
    }

    /// Whether this backend keeps a cookie jar.
    pub fn keeps_cookies(self) -> bool {
        matches!(self, Self::Curl)
    }

    /// Whether connections are reused. NetIO sends `Connection: Close`.
    pub fn reuses_connections(self) -> bool {
        matches!(self, Self::Curl)
    }

    /// The whole-query timeout, when the backend imposes one. curl bounds the
    /// connect phase and a stalled transfer instead, which `reqwest` expresses
    /// separately, so only NetIO has a single query deadline.
    pub fn query_timeout(self) -> Option<Duration> {
        match self {
            Self::Curl => None,
            Self::NetIo => Some(NETIO_QUERY_TIMEOUT),
        }
    }

    /// Applies the backend's policy to a client builder. Both backends decode
    /// gzip and send the engine user-agent; everything else differs.
    pub fn apply(self, builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
        let builder = builder.gzip(true);
        match self {
            Self::Curl => builder
                .cookie_store(true)
                .redirect(reqwest::redirect::Policy::custom(|attempt| {
                    attempt.follow()
                })),
            Self::NetIo => builder
                .cookie_store(false)
                .redirect(reqwest::redirect::Policy::none())
                // `Connection: Close` on every NetIO request means no pool.
                .pool_max_idle_per_host(0)
                .timeout(NETIO_QUERY_TIMEOUT),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // C4Network2Reference.cpp:410-413 selects the impl; :404-405,825-856 and
    // C4HTTPClient.cpp:189-198 are what the two impls actually do differently.
    #[test]
    fn use_curl_false_selects_netio_compatible_http_transport() {
        // The key's C++ default is true (C4Config.cpp:561), so the shipped
        // behaviour is the curl policy.
        assert_eq!(HttpBackend::default(), HttpBackend::Curl);
        assert_eq!(HttpBackend::from_use_curl(true), HttpBackend::Curl);
        assert_eq!(HttpBackend::from_use_curl(false), HttpBackend::NetIo);

        // curl follows Location, keeps a jar and shares connections.
        assert!(HttpBackend::Curl.follows_redirects());
        assert!(HttpBackend::Curl.keeps_cookies());
        assert!(HttpBackend::Curl.reuses_connections());
        // Its timeouts bound the connect phase and stalled transfers rather
        // than the whole query.
        assert_eq!(HttpBackend::Curl.query_timeout(), None);

        // NetIO does none of those: there is no Location handling in
        // C4Network2HTTPClientImplNetIO at all, no cookie state, and every
        // request carries `Connection: Close`.
        assert!(!HttpBackend::NetIo.follows_redirects());
        assert!(!HttpBackend::NetIo.keeps_cookies());
        assert!(!HttpBackend::NetIo.reuses_connections());
        assert_eq!(
            HttpBackend::NetIo.query_timeout(),
            Some(Duration::from_secs(20)),
            "C4Network2HTTPQueryTimeout"
        );
        assert_eq!(NETIO_HAPPY_EYEBALLS_TIMEOUT, Duration::from_millis(300));

        // Both policies build, and both decode gzip — `Accept-Encoding: gzip`
        // is sent by the curl handle and written into the NetIO header block.
        for backend in [HttpBackend::Curl, HttpBackend::NetIo] {
            backend
                .apply(reqwest::Client::builder())
                .build()
                .unwrap_or_else(|error| panic!("{backend:?} policy builds: {error}"));
        }
    }
}
