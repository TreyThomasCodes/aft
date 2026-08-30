use std::fmt;

use url::Url;

/// One of the two GitHub resource kinds supported by the read scheme.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum GithubResourceKind {
    Issue,
    PullRequest,
}

impl GithubResourceKind {
    pub const fn scheme(self) -> &'static str {
        match self {
            Self::Issue => "issue",
            Self::PullRequest => "pr",
        }
    }

    pub const fn command(self) -> &'static str {
        match self {
            Self::Issue => "issue",
            Self::PullRequest => "pr",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Issue => "Issue",
            Self::PullRequest => "Pull request",
        }
    }
}

/// A validated `issue://` or `pr://` resource.
///
/// Short resources deliberately retain no inferred repository. `gh` resolves
/// those resources from the request's working directory, while explicit
/// resources carry the exact `OWNER/REPO` argument to pass to `gh -R`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GithubResource {
    pub kind: GithubResourceKind,
    pub number: u64,
    pub repository: Option<String>,
}

impl GithubResource {
    pub fn is_explicit(&self) -> bool {
        self.repository.is_some()
    }
}

/// A typed error produced before a GitHub resource can enter the fetch path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvalidGithubResource {
    resource: String,
    reason: &'static str,
}

impl InvalidGithubResource {
    fn new(resource: &str, reason: &'static str) -> Self {
        Self {
            resource: resource.to_string(),
            reason,
        }
    }

    pub fn code(&self) -> &'static str {
        "invalid_resource"
    }
}

impl fmt::Display for InvalidGithubResource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid GitHub resource '{}': {}. Use issue://NUMBER, pr://NUMBER, issue://OWNER/REPO/NUMBER, or pr://OWNER/REPO/NUMBER",
            self.resource, self.reason
        )
    }
}

impl std::error::Error for InvalidGithubResource {}

/// Parse only the GitHub resource forms that the read command exposes.
///
/// This rejects query strings, fragments, ports, credentials, empty path
/// components, and alternate URL shapes rather than silently treating them as
/// filesystem paths or a different remote resource.
pub fn parse_resource(resource: &str) -> Result<GithubResource, InvalidGithubResource> {
    let parsed = Url::parse(resource)
        .map_err(|_| InvalidGithubResource::new(resource, "the URL is malformed"))?;
    let kind = match parsed.scheme() {
        "issue" => GithubResourceKind::Issue,
        "pr" => GithubResourceKind::PullRequest,
        _ => return Err(InvalidGithubResource::new(resource, "unsupported scheme")),
    };

    if parsed.query().is_some()
        || parsed.fragment().is_some()
        || parsed.port().is_some()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err(InvalidGithubResource::new(
            resource,
            "unsupported URL authority or suffix",
        ));
    }

    let authority = parsed
        .host_str()
        .ok_or_else(|| InvalidGithubResource::new(resource, "missing authority"))?;
    let path_segments: Vec<_> = parsed
        .path_segments()
        .map(|segments| segments.filter(|segment| !segment.is_empty()).collect())
        .unwrap_or_default();

    // `issue://373` uses the URL authority for the number. It has no path.
    if parsed.path().is_empty() {
        return Ok(GithubResource {
            kind,
            number: parse_number(resource, authority)?,
            repository: None,
        });
    }

    // `issue://OWNER/REPO/373` uses the authority for OWNER and exactly two
    // non-empty path components for REPO and NUMBER.
    if path_segments.len() != 2
        || !valid_repository_component(authority)
        || !valid_repository_component(path_segments[0])
    {
        return Err(InvalidGithubResource::new(
            resource,
            "unsupported authority or malformed repository path",
        ));
    }

    Ok(GithubResource {
        kind,
        number: parse_number(resource, path_segments[1])?,
        repository: Some(format!("{authority}/{}", path_segments[0])),
    })
}

fn parse_number(resource: &str, value: &str) -> Result<u64, InvalidGithubResource> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(InvalidGithubResource::new(
            resource,
            "resource number must be numeric",
        ));
    }
    let number = value
        .parse::<u64>()
        .map_err(|_| InvalidGithubResource::new(resource, "resource number is out of range"))?;
    if number == 0 {
        return Err(InvalidGithubResource::new(
            resource,
            "resource number must be greater than zero",
        ));
    }
    Ok(number)
}

fn valid_repository_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_the_four_resource_forms() {
        assert_eq!(
            parse_resource("issue://373").unwrap(),
            GithubResource {
                kind: GithubResourceKind::Issue,
                number: 373,
                repository: None,
            }
        );
        assert_eq!(
            parse_resource("pr://Owner/repo-name/45").unwrap(),
            GithubResource {
                kind: GithubResourceKind::PullRequest,
                number: 45,
                repository: Some("Owner/repo-name".to_string()),
            }
        );

        for value in [
            "issue:///373",
            "issue://373/",
            "issue://owner/repo/not-a-number",
            "issue://owner/repo/0",
            "issue://owner/repo/1/extra",
            "issue://owner/repo/1?view=full",
            "issue://owner:secret@repo/1",
            "issue://github.com/owner/repo/1",
            "https://github.com/owner/repo/issues/1",
        ] {
            assert!(parse_resource(value).is_err(), "{value}");
        }
    }
}
