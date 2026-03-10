use skillfile_core::error::SkillfileError;

// ---------------------------------------------------------------------------
// HttpClient trait — abstraction over HTTP GET for testability
// ---------------------------------------------------------------------------

/// Contract for HTTP GET requests used by the fetcher/resolver layer.
///
/// Implementations are responsible for:
/// - Setting standard headers (User-Agent, Authorization)
/// - Connection pooling / agent reuse
/// - Error mapping to [`SkillfileError`]
///
/// The trait has two methods covering the two HTTP patterns in this codebase:
/// - `get_bytes`: raw file downloads (content from `raw.githubusercontent.com`)
/// - `get_json`: GitHub API calls that may return 4xx gracefully
pub trait HttpClient: Send + Sync {
    /// GET a URL and return the response body as raw bytes.
    ///
    /// Returns `Err(SkillfileError::Network)` on HTTP errors (including 404).
    fn get_bytes(&self, url: &str) -> Result<Vec<u8>, SkillfileError>;

    /// GET a URL with `Accept: application/vnd.github.v3+json` header.
    ///
    /// Returns `Ok(None)` on 4xx client errors (used for tentative lookups
    /// like SHA resolution where a missing ref is not fatal).
    /// Returns `Err` on network/server errors.
    fn get_json(&self, url: &str) -> Result<Option<String>, SkillfileError>;
}

// ---------------------------------------------------------------------------
// UreqClient — the production implementation backed by ureq
// ---------------------------------------------------------------------------

/// Production HTTP client backed by `ureq::Agent`.
///
/// Automatically attaches `User-Agent` and GitHub `Authorization` headers
/// to every request. The GitHub token is discovered once from environment
/// variables or the `gh` CLI and cached for the process lifetime.
pub struct UreqClient {
    agent: ureq::Agent,
}

impl UreqClient {
    pub fn new() -> Self {
        Self {
            agent: ureq::Agent::new_with_defaults(),
        }
    }

    /// Build a GET request with standard headers.
    fn build_get(&self, url: &str) -> ureq::RequestBuilder<ureq::typestate::WithoutBody> {
        let mut req = self.agent.get(url).header("User-Agent", "skillfile/1.0");
        if let Some(token) = super::resolver::github_token() {
            req = req.header("Authorization", &format!("Bearer {token}"));
        }
        req
    }
}

impl Default for UreqClient {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpClient for UreqClient {
    fn get_bytes(&self, url: &str) -> Result<Vec<u8>, SkillfileError> {
        let mut response = self.build_get(url).call().map_err(|e| match &e {
            ureq::Error::StatusCode(404) => SkillfileError::Network(format!(
                "HTTP 404: {url} not found — check that the path exists in the upstream repo"
            )),
            ureq::Error::StatusCode(code) => {
                SkillfileError::Network(format!("HTTP {code} fetching {url}"))
            }
            _ => SkillfileError::Network(format!("{e} fetching {url}")),
        })?;
        response.body_mut().read_to_vec().map_err(|e| {
            SkillfileError::Network(format!("failed to read response from {url}: {e}"))
        })
    }

    fn get_json(&self, url: &str) -> Result<Option<String>, SkillfileError> {
        let result = self
            .build_get(url)
            .header("Accept", "application/vnd.github.v3+json")
            .call();

        match result {
            Ok(mut response) => {
                let text = response.body_mut().read_to_string().map_err(|e| {
                    SkillfileError::Network(format!("failed to read response from {url}: {e}"))
                })?;
                Ok(Some(text))
            }
            Err(ureq::Error::StatusCode(code)) if (400..500).contains(&code) => Ok(None),
            Err(e) => Err(SkillfileError::Network(format!("{e} fetching {url}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ureq_client_default_creates_successfully() {
        let _client = UreqClient::default();
    }
}
