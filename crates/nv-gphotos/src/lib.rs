//! Upload pictures to Google Photos.
//!
//! Sign-in is OAuth 2.0 for installed applications: the system browser shows
//! Google's consent page, and Google sends the browser back to a port this
//! program listens on for a moment on the loopback address (RFC 8252), with a
//! PKCE challenge so a stolen code is useless. Only the add-only scope is asked
//! for: the program can put pictures in the library and make albums for them,
//! and can read or change nothing else.
//!
//! Everything is blocking and meant to run on a worker thread. The endpoints
//! can be replaced, which is how the tests run against a local server.

mod oauth;
mod photos;

pub use oauth::{OAuthConfig, Tokens, authorization_url, sign_in};
pub use photos::{Client, UploadedItem, mime_for};

use std::time::Duration;

/// The scope that lets an app add to the library, and nothing more.
pub const SCOPE: &str = "https://www.googleapis.com/auth/photoslibrary.appendonly";

/// Where to talk to. [`Endpoints::google`] is the real service.
#[derive(Debug, Clone)]
pub struct Endpoints {
    pub auth: String,
    pub token: String,
    pub revoke: String,
    pub upload: String,
    pub batch_create: String,
    pub albums: String,
}

impl Endpoints {
    pub fn google() -> Self {
        Self {
            auth: "https://accounts.google.com/o/oauth2/v2/auth".into(),
            token: "https://oauth2.googleapis.com/token".into(),
            revoke: "https://oauth2.googleapis.com/revoke".into(),
            upload: "https://photoslibrary.googleapis.com/v1/uploads".into(),
            batch_create: "https://photoslibrary.googleapis.com/v1/mediaItems:batchCreate".into(),
            albums: "https://photoslibrary.googleapis.com/v1/albums".into(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The person has to sign in (again): no token, or Google no longer accepts it.
    #[error("not signed in to Google Photos")]
    NotSignedIn,
    /// The sign-in was stopped, or nobody finished it in time.
    #[error("the sign-in was cancelled")]
    Cancelled,
    /// Google refused the sign-in or the request, with its explanation.
    #[error("{0}")]
    Refused(String),
    /// Google answered with an error status.
    #[error("Google Photos answered {status}: {message}")]
    Http { status: u16, message: String },
    #[error("could not reach Google: {0}")]
    Network(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl From<ureq::Error> for Error {
    fn from(e: ureq::Error) -> Self {
        Error::Network(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn agent() -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(600)))
        .timeout_connect(Some(Duration::from_secs(20)))
        .user_agent(concat!("NegativeVibes/", env!("CARGO_PKG_VERSION")))
        .build();
    ureq::Agent::new_with_config(config)
}
