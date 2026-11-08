//! Allow-listed, bounded HTTP fetching for track archives (FR-DA-01).
//!
//! Every archive request is checked against an HTTPS host allow-list before it
//! leaves the process. Responses are capped, non-success statuses are bounded
//! by a retry count, and the production transport applies a timeout and refuses
//! redirects. [`HttpTransport`] is the seam that lets tests replay recorded
//! payloads without touching the network (SDD section 9.1).

use std::fmt;
use std::io::Read;
use std::time::Duration;

use crate::error::AdsbError;

/// One authenticated GET request.
#[derive(Clone, PartialEq, Eq)]
pub struct HttpRequest {
    url: String,
    bearer_token: Option<String>,
}

impl HttpRequest {
    /// Build a GET request. [`HttpPolicy::fetch`] validates the URL.
    pub fn new(url: impl Into<String>, bearer_token: Option<String>) -> Self {
        Self {
            url: url.into(),
            bearer_token,
        }
    }

    /// Request URL.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Bearer token, when the archive requires authentication.
    ///
    /// The token is never included in [`fmt::Debug`] output.
    pub fn bearer_token(&self) -> Option<&str> {
        self.bearer_token.as_deref()
    }
}

impl fmt::Debug for HttpRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HttpRequest")
            .field("url", &self.url)
            .field("authenticated", &self.bearer_token.is_some())
            .finish()
    }
}

/// A fully read HTTP response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    status: u16,
    body: Vec<u8>,
}

impl HttpResponse {
    /// Build a response from a status code and body.
    pub fn new(status: u16, body: Vec<u8>) -> Self {
        Self { status, body }
    }

    /// HTTP status code.
    pub fn status(&self) -> u16 {
        self.status
    }

    /// Response body bytes.
    pub fn body(&self) -> &[u8] {
        &self.body
    }
}

/// A synchronous HTTP transport.
///
/// Production code uses [`ReqwestTransport`]. Tests implement this trait with
/// recorded responses, so no test opens a socket.
pub trait HttpTransport: Send + Sync {
    /// Perform one GET request.
    fn get(&self, request: &HttpRequest) -> Result<HttpResponse, AdsbError>;
}

/// Host allow-list, timeout, size cap and retry bounds for archive requests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpPolicy {
    allowed_hosts: Vec<String>,
    timeout: Duration,
    max_response_bytes: usize,
    max_attempts: usize,
    retry_backoff: Duration,
}

impl HttpPolicy {
    /// Build a policy from its bounds.
    ///
    /// `max_attempts` counts the first request, so a value of three allows two
    /// retries. `retry_backoff` is the pause between attempts and may be zero
    /// in tests.
    ///
    /// # Errors
    ///
    /// Returns [`AdsbError::InvalidRequest`] when the host list is empty, a
    /// host is blank, or the timeout, size cap or attempt count is zero.
    pub fn new(
        allowed_hosts: Vec<String>,
        timeout: Duration,
        max_response_bytes: usize,
        max_attempts: usize,
        retry_backoff: Duration,
    ) -> Result<Self, AdsbError> {
        if timeout.is_zero() {
            return Err(AdsbError::InvalidRequest(
                "HTTP timeout must be non-zero".to_owned(),
            ));
        }
        if max_response_bytes == 0 {
            return Err(AdsbError::InvalidRequest(
                "response size cap must be non-zero".to_owned(),
            ));
        }
        if max_attempts == 0 {
            return Err(AdsbError::InvalidRequest(
                "attempt count must be at least one".to_owned(),
            ));
        }
        if allowed_hosts.is_empty() {
            return Err(AdsbError::InvalidRequest(
                "host allow-list must not be empty".to_owned(),
            ));
        }
        let mut normalized = Vec::with_capacity(allowed_hosts.len());
        for host in allowed_hosts {
            let host = host.trim().to_ascii_lowercase();
            if host.is_empty() {
                return Err(AdsbError::InvalidRequest(
                    "host allow-list contains a blank host".to_owned(),
                ));
            }
            normalized.push(host);
        }
        Ok(Self {
            allowed_hosts: normalized,
            timeout,
            max_response_bytes,
            max_attempts,
            retry_backoff,
        })
    }

    /// Hosts this policy permits, lowercased.
    pub fn allowed_hosts(&self) -> &[String] {
        &self.allowed_hosts
    }

    /// Request timeout applied by the production transport.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Maximum response body size in bytes.
    pub fn max_response_bytes(&self) -> usize {
        self.max_response_bytes
    }

    /// Maximum number of attempts, including the first request.
    pub fn max_attempts(&self) -> usize {
        self.max_attempts
    }

    /// Pause between retry attempts.
    pub fn retry_backoff(&self) -> Duration {
        self.retry_backoff
    }

    /// Build the production transport with this policy's timeout and size cap.
    ///
    /// # Errors
    ///
    /// Returns [`AdsbError::Io`] if the TLS client cannot be constructed.
    pub fn reqwest_transport(&self) -> Result<ReqwestTransport, AdsbError> {
        ReqwestTransport::new(self.timeout, self.max_response_bytes)
    }

    /// Validate the URL and fetch it with bounded retries.
    ///
    /// Only HTTPS requests to an allow-listed host are sent. Statuses 408, 425,
    /// 429, 500, 502, 503 and 504, and transport errors other than a size-cap
    /// breach, are retried until `max_attempts`. Any other non-success status
    /// is returned immediately.
    ///
    /// # Errors
    ///
    /// Returns [`AdsbError::DisallowedHost`] or [`AdsbError::InvalidRequest`]
    /// for a rejected URL, [`AdsbError::ResponseTooLarge`] when the body
    /// exceeds the cap, [`AdsbError::HttpStatus`] for a non-success status, or
    /// the transport's own error.
    pub fn fetch<T: HttpTransport>(
        &self,
        transport: &T,
        request: &HttpRequest,
    ) -> Result<HttpResponse, AdsbError> {
        self.check_url(request.url())?;
        let mut last_error = AdsbError::Io("HTTP request made no attempt".to_owned());

        for attempt in 0..self.max_attempts {
            match transport.get(request) {
                Ok(response) => {
                    if response.body.len() > self.max_response_bytes {
                        return Err(AdsbError::ResponseTooLarge {
                            limit_bytes: self.max_response_bytes,
                        });
                    }
                    if (200..300).contains(&response.status) {
                        return Ok(response);
                    }
                    let error = AdsbError::HttpStatus {
                        status: response.status,
                    };
                    if !retryable_status(response.status) || attempt + 1 == self.max_attempts {
                        return Err(error);
                    }
                    last_error = error;
                }
                Err(error) => {
                    if !retryable_error(&error) || attempt + 1 == self.max_attempts {
                        return Err(error);
                    }
                    last_error = error;
                }
            }
            if !self.retry_backoff.is_zero() {
                std::thread::sleep(self.retry_backoff);
            }
        }

        Err(last_error)
    }

    /// Reject anything that is not HTTPS on an allow-listed host.
    fn check_url(&self, raw: &str) -> Result<(), AdsbError> {
        let url = reqwest::Url::parse(raw)
            .map_err(|error| AdsbError::InvalidRequest(format!("invalid URL: {error}")))?;
        if url.scheme() != "https" {
            return Err(AdsbError::InvalidRequest(
                "archive URLs must use HTTPS".to_owned(),
            ));
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(AdsbError::InvalidRequest(
                "archive URLs must not carry credentials".to_owned(),
            ));
        }
        if url.fragment().is_some() {
            return Err(AdsbError::InvalidRequest(
                "archive URLs must not carry a fragment".to_owned(),
            ));
        }
        if url.port_or_known_default() != Some(443) {
            return Err(AdsbError::InvalidRequest(
                "archive URLs must use the default HTTPS port".to_owned(),
            ));
        }
        let host = url
            .host_str()
            .ok_or_else(|| AdsbError::InvalidRequest("archive URL has no host".to_owned()))?
            .to_ascii_lowercase();
        if !self.allowed_hosts.contains(&host) {
            return Err(AdsbError::DisallowedHost(host));
        }
        Ok(())
    }
}

/// Whether a non-success status is worth retrying.
fn retryable_status(status: u16) -> bool {
    matches!(status, 408 | 425 | 429 | 500 | 502 | 503 | 504)
}

/// Whether a transport error is worth retrying.
fn retryable_error(error: &AdsbError) -> bool {
    !matches!(
        error,
        AdsbError::ResponseTooLarge { .. }
            | AdsbError::InvalidRequest(_)
            | AdsbError::DisallowedHost(_)
    )
}

/// Production transport built on `reqwest`'s blocking client.
#[derive(Debug, Clone)]
pub struct ReqwestTransport {
    client: reqwest::blocking::Client,
    max_response_bytes: usize,
}

impl ReqwestTransport {
    /// Build a client with a timeout, no redirects and a response size cap.
    ///
    /// Redirects are refused so an allow-listed host cannot forward the
    /// request to an unlisted one.
    ///
    /// # Errors
    ///
    /// Returns [`AdsbError::InvalidRequest`] for a zero timeout or cap, or
    /// [`AdsbError::Io`] if the TLS client cannot be constructed.
    pub fn new(timeout: Duration, max_response_bytes: usize) -> Result<Self, AdsbError> {
        if timeout.is_zero() {
            return Err(AdsbError::InvalidRequest(
                "HTTP timeout must be non-zero".to_owned(),
            ));
        }
        if max_response_bytes == 0 {
            return Err(AdsbError::InvalidRequest(
                "response size cap must be non-zero".to_owned(),
            ));
        }
        let client = reqwest::blocking::Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("doppler-finder/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| AdsbError::Io(format!("could not build HTTP client: {error}")))?;
        Ok(Self {
            client,
            max_response_bytes,
        })
    }
}

impl HttpTransport for ReqwestTransport {
    fn get(&self, request: &HttpRequest) -> Result<HttpResponse, AdsbError> {
        let mut builder = self.client.get(request.url());
        if let Some(token) = request.bearer_token() {
            builder = builder.bearer_auth(token);
        }
        let response = builder
            .send()
            .map_err(|error| AdsbError::Io(format!("HTTP request failed: {error}")))?;
        let status = response.status().as_u16();
        let limit = u64::try_from(self.max_response_bytes)
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        let mut body = Vec::new();
        response
            .take(limit)
            .read_to_end(&mut body)
            .map_err(|error| AdsbError::Io(format!("could not read response body: {error}")))?;
        if body.len() > self.max_response_bytes {
            return Err(AdsbError::ResponseTooLarge {
                limit_bytes: self.max_response_bytes,
            });
        }
        Ok(HttpResponse::new(status, body))
    }
}
