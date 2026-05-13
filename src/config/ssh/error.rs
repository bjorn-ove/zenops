//! `config::ssh`-scoped error type.
//!
//! Wraps the failure modes specific to fetching and parsing SSH keys
//! for GitHub-typed entries in the SSH config block. Exposed to the
//! rest of the crate as `crate::Error::Ssh` via `#[error(transparent)]`
//! + `#[from]`.

use smol_str::SmolStr;

/// Failure modes for the GitHub-key-fetching paths in this module.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// `curl` isn't on `PATH` and is needed to fetch a user's GitHub keys.
    #[error(
        "curl is required to fetch GitHub SSH keys; install curl or switch the entry to type = \"manual\""
    )]
    CurlNotFound,
    /// `curl https://api.github.com/users/<u>/ssh_signing_keys` failed.
    #[error("Failed to fetch SSH keys for GitHub user {username}: {source}")]
    GithubKeyFetchFailed {
        /// GitHub username queried.
        username: SmolStr,
        /// Underlying xshell/curl failure.
        #[source]
        source: xshell::Error,
    },
    /// GitHub returned a body that didn't match the expected SSH-signing-key JSON shape.
    #[error("Failed to parse SSH signing keys response for GitHub user {username}: {source}")]
    GithubKeyParseFailed {
        /// GitHub username queried.
        username: SmolStr,
        /// Underlying serde_json failure.
        #[source]
        source: serde_json::Error,
    },
}

impl PartialEq for Error {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::CurlNotFound, Self::CurlNotFound) => true,
            (
                Self::GithubKeyFetchFailed {
                    username: l_user,
                    source: l_src,
                },
                Self::GithubKeyFetchFailed {
                    username: r_user,
                    source: r_src,
                },
            ) => l_user == r_user && l_src.to_string() == r_src.to_string(),
            (
                Self::GithubKeyParseFailed {
                    username: l_user,
                    source: l_src,
                },
                Self::GithubKeyParseFailed {
                    username: r_user,
                    source: r_src,
                },
            ) => l_user == r_user && l_src.to_string() == r_src.to_string(),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use similar_asserts::assert_eq;
    use smol_str::SmolStr;
    use xshell::{Shell, cmd};

    use super::*;

    fn xshell_err() -> xshell::Error {
        let sh = Shell::new().unwrap();
        cmd!(sh, "false").quiet().run().unwrap_err()
    }

    fn json_err() -> serde_json::Error {
        serde_json::from_str::<serde_json::Value>("{").unwrap_err()
    }

    #[test]
    fn curl_not_found_eq() {
        assert_eq!(Error::CurlNotFound, Error::CurlNotFound);
    }

    #[test]
    fn github_key_fetch_failed_eq_and_ne() {
        let a = Error::GithubKeyFetchFailed {
            username: SmolStr::new_static("u"),
            source: xshell_err(),
        };
        let b = Error::GithubKeyFetchFailed {
            username: SmolStr::new_static("u"),
            source: xshell_err(),
        };
        let c = Error::GithubKeyFetchFailed {
            username: SmolStr::new_static("v"),
            source: xshell_err(),
        };
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn github_key_parse_failed_eq_and_ne() {
        let a = Error::GithubKeyParseFailed {
            username: SmolStr::new_static("u"),
            source: json_err(),
        };
        let b = Error::GithubKeyParseFailed {
            username: SmolStr::new_static("u"),
            source: json_err(),
        };
        let c = Error::GithubKeyParseFailed {
            username: SmolStr::new_static("v"),
            source: json_err(),
        };
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
