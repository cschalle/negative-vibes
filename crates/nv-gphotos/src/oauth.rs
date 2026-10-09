//! Sign-in: authorisation code flow with PKCE on a loopback redirect.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::{Endpoints, Error, Result, SCOPE, agent};

/// The application's identity with Google (a "Desktop app" OAuth client).
/// The secret of an installed app is not secret; Google asks for it all the same.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OAuthConfig {
    pub client_id: String,
    pub client_secret: String,
}

impl OAuthConfig {
    pub fn is_set(&self) -> bool {
        !self.client_id.trim().is_empty()
    }
}

/// What a sign-in leaves behind. Only the refresh token is worth keeping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tokens {
    pub refresh_token: String,
    pub access_token: Option<String>,
    pub expires_at: Option<Instant>,
}

#[derive(Deserialize)]
pub(crate) struct TokenReply {
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub expires_in: Option<u64>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

fn random_string(bytes: usize) -> String {
    let mut b = vec![0u8; bytes];
    getrandom::fill(&mut b).expect("the system has no random numbers");
    URL_SAFE_NO_PAD.encode(b)
}

/// `S256` challenge for a verifier (RFC 7636).
fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

pub(crate) fn pct_encode(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => o.push(b as char),
            _ => o.push_str(&format!("%{b:02X}")),
        }
    }
    o
}

pub(crate) fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() && b[i + 1].is_ascii_hexdigit() && b[i + 2].is_ascii_hexdigit() => {
                out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap_or(b'?'));
                i += 2;
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The page to send the person to.
pub fn authorization_url(endpoints: &Endpoints, cfg: &OAuthConfig, redirect: &str, challenge: &str, state: &str) -> String {
    format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&code_challenge={}&code_challenge_method=S256&access_type=offline&prompt=consent",
        endpoints.auth,
        pct_encode(&cfg.client_id),
        pct_encode(redirect),
        pct_encode(SCOPE),
        pct_encode(state),
        pct_encode(challenge),
    )
}

const PAGE: &str =
    "<!doctype html><meta charset=utf-8><title>Negative Vibes</title><body style=\"font:16px sans-serif;max-width:30em;margin:4em auto\"><h1>%TITLE%</h1><p>%BODY%</p>";

fn reply(stream: &mut TcpStream, status: &str, title: &str, body: &str) {
    let page = PAGE.replace("%TITLE%", title).replace("%BODY%", body);
    let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}", page.len());
}

/// Wait for the browser to come back, and return the query of the request it makes.
fn wait_for_redirect(listener: &TcpListener, cancel: &AtomicBool, timeout: Duration) -> Result<String> {
    listener.set_nonblocking(true)?;
    let start = Instant::now();
    loop {
        if cancel.load(Ordering::Relaxed) || start.elapsed() > timeout {
            return Err(Error::Cancelled);
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                stream.set_nonblocking(false)?;
                stream.set_read_timeout(Some(Duration::from_secs(5)))?;
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).into_owned();
                let target = head.lines().next().and_then(|l| l.split_whitespace().nth(1)).unwrap_or("");
                // Browsers also ask for /favicon.ico; only the redirect carries a query.
                let Some((path, query)) = target.split_once('?') else {
                    reply(&mut stream, "404 Not Found", "Not found", "");
                    continue;
                };
                if path != "/" {
                    reply(&mut stream, "404 Not Found", "Not found", "");
                    continue;
                }
                let denied = query.split('&').any(|p| p.starts_with("error="));
                if denied {
                    reply(&mut stream, "200 OK", "Sign-in cancelled", "Nothing was shared with Negative Vibes. You can close this tab.");
                } else {
                    reply(&mut stream, "200 OK", "You are signed in", "You can close this tab and go back to Negative Vibes.");
                }
                return Ok(query.to_owned());
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(e.into()),
        }
    }
}

pub(crate) fn token_request(endpoints: &Endpoints, form: &[(&str, &str)]) -> Result<TokenReply> {
    let mut resp = agent().post(&endpoints.token).send_form(form.iter().copied())?;
    let status = resp.status().as_u16();
    let text = resp.body_mut().read_to_string()?;
    let reply: TokenReply = serde_json::from_str(&text).map_err(|_| Error::Http { status, message: text.chars().take(300).collect() })?;
    if let Some(code) = &reply.error {
        let detail = reply.error_description.clone().unwrap_or_else(|| code.clone());
        return Err(if code == "invalid_grant" { Error::NotSignedIn } else { Error::Refused(detail) });
    }
    if !(200..300).contains(&status) {
        return Err(Error::Http { status, message: text.chars().take(300).collect() });
    }
    Ok(reply)
}

/// Run the whole sign-in. `open` is given the address to show the person (the
/// caller opens the browser); it must return quickly. Blocks until the
/// browser comes back, `cancel` is set, or `timeout` passes.
pub fn sign_in(endpoints: &Endpoints, cfg: &OAuthConfig, open: &dyn Fn(&str), cancel: &AtomicBool, timeout: Duration) -> Result<Tokens> {
    if !cfg.is_set() {
        return Err(Error::Refused("no Google client ID is set".into()));
    }
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let redirect = format!("http://127.0.0.1:{}", listener.local_addr()?.port());
    let verifier = random_string(48);
    let state = random_string(16);
    open(&authorization_url(endpoints, cfg, &redirect, &challenge(&verifier), &state));

    let query = wait_for_redirect(&listener, cancel, timeout)?;
    let get = |key: &str| query.split('&').find_map(|p| p.strip_prefix(key).and_then(|v| v.strip_prefix('=')).map(pct_decode));
    if let Some(e) = get("error") {
        return Err(if e == "access_denied" { Error::Cancelled } else { Error::Refused(e) });
    }
    if get("state").as_deref() != Some(state.as_str()) {
        return Err(Error::Refused("the sign-in answer did not match the request".into()));
    }
    let code = get("code").ok_or_else(|| Error::Refused("Google sent no sign-in code".into()))?;

    let reply = token_request(
        endpoints,
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", &redirect),
            ("client_id", &cfg.client_id),
            ("client_secret", &cfg.client_secret),
            ("code_verifier", &verifier),
        ],
    )?;
    let refresh_token = reply.refresh_token.ok_or_else(|| Error::Refused("Google gave no long-lived sign-in; try again".into()))?;
    Ok(Tokens { refresh_token, access_token: reply.access_token, expires_at: reply.expires_in.map(|s| Instant::now() + Duration::from_secs(s)) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pkce_example_from_rfc_7636_matches() {
        assert_eq!(challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn percent_encoding_round_trips() {
        for s in ["plain", "a b&c=d/é", "http://127.0.0.1:4000", "100%"] {
            assert_eq!(pct_decode(&pct_encode(s)), s);
        }
        assert_eq!(pct_decode("a+b%20c"), "a b c");
        assert_eq!(pct_decode("bad%zz%"), "bad%zz%");
    }

    #[test]
    fn the_url_asks_for_the_add_only_scope_and_a_challenge() {
        let u = authorization_url(&Endpoints::google(), &OAuthConfig { client_id: "id".into(), client_secret: "s".into() }, "http://127.0.0.1:1", "CH", "ST");
        assert!(u.starts_with("https://accounts.google.com/o/oauth2/v2/auth?"));
        assert!(u.contains("scope=https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fphotoslibrary.appendonly"));
        assert!(u.contains("code_challenge=CH&code_challenge_method=S256"));
        assert!(u.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A1"));
        assert!(!u.contains("client_secret"));
    }
}
