//! The add-only Photos Library API: upload bytes, then make a library item
//! from the upload token, optionally inside an album the app made.

use std::path::Path;
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::oauth::{Tokens, token_request};
use crate::{Endpoints, Error, OAuthConfig, Result, agent};

/// A picture now in the library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadedItem {
    pub id: String,
    /// Where the person can open it in Google Photos, when Google says.
    pub product_url: Option<String>,
}

/// The media type Google needs to be told for a file we write.
pub fn mime_for(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("tif" | "tiff") => "image/tiff",
        _ => "application/octet-stream",
    }
}

/// How many times a failed request is tried before giving up, and the pause before the second try.
const ATTEMPTS: u32 = 4;
const FIRST_PAUSE: Duration = Duration::from_secs(1);

pub struct Client {
    endpoints: Endpoints,
    cfg: OAuthConfig,
    refresh_token: String,
    access: Option<(String, Instant)>,
    /// Pause before a retry; short in tests.
    pause: Duration,
}

#[derive(Deserialize)]
struct BatchReply {
    #[serde(rename = "newMediaItemResults", default)]
    results: Vec<BatchResult>,
}

#[derive(Deserialize)]
struct BatchResult {
    status: Option<BatchStatus>,
    #[serde(rename = "mediaItem")]
    media_item: Option<MediaItem>,
}

#[derive(Deserialize)]
struct BatchStatus {
    code: Option<i64>,
    message: Option<String>,
}

#[derive(Deserialize)]
struct MediaItem {
    id: String,
    #[serde(rename = "productUrl")]
    product_url: Option<String>,
}

fn clip(s: &str) -> String {
    let one_line: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    one_line.chars().take(300).collect()
}

impl Client {
    pub fn new(endpoints: Endpoints, cfg: OAuthConfig, tokens: Tokens) -> Self {
        let access = tokens.access_token.zip(tokens.expires_at);
        Self { endpoints, cfg, refresh_token: tokens.refresh_token, access, pause: FIRST_PAUSE }
    }

    #[cfg(test)]
    fn quick(mut self) -> Self {
        self.pause = Duration::from_millis(5);
        self
    }

    /// A token Google accepts now, fetching a new one when the last has (almost) run out.
    fn access_token(&mut self) -> Result<String> {
        if let Some((t, until)) = &self.access
            && Instant::now() + Duration::from_secs(60) < *until
        {
            return Ok(t.clone());
        }
        let r = token_request(
            &self.endpoints,
            &[("grant_type", "refresh_token"), ("refresh_token", &self.refresh_token), ("client_id", &self.cfg.client_id), ("client_secret", &self.cfg.client_secret)],
        )?;
        let token = r.access_token.ok_or_else(|| Error::Refused("Google sent no access token".into()))?;
        self.access = Some((token.clone(), Instant::now() + Duration::from_secs(r.expires_in.unwrap_or(3000))));
        Ok(token)
    }

    /// Send a request built by `make` (given a fresh bearer token), trying again
    /// on rate limits and server trouble, and once more with a new token if Google says
    /// the old one is no good.
    fn call<T>(&mut self, mut make: impl FnMut(&str) -> Result<(u16, T)>) -> Result<T>
    where
        T: AsRef<str>,
    {
        let mut pause = self.pause;
        let mut renewed = false;
        for attempt in 1..=ATTEMPTS {
            let token = self.access_token()?;
            let (status, body) = make(&token)?;
            match status {
                200..=299 => return Ok(body),
                401 if !renewed => {
                    renewed = true;
                    self.access = None;
                }
                401 | 403 if body.as_ref().contains("insufficient") || status == 401 => return Err(Error::NotSignedIn),
                429 | 500..=599 if attempt < ATTEMPTS => {
                    std::thread::sleep(pause);
                    pause *= 2;
                }
                _ => return Err(Error::Http { status, message: clip(body.as_ref()) }),
            }
        }
        Err(Error::Network("too many failed tries".into()))
    }

    /// Make an album with this title and return its id. Albums made by the app are the
    /// only ones it may add to.
    pub fn create_album(&mut self, title: &str) -> Result<String> {
        let url = self.endpoints.albums.clone();
        let body = serde_json::json!({ "album": { "title": title.chars().take(500).collect::<String>() } });
        let text = self.call(|token| {
            let mut r = agent().post(&url).header("Authorization", &format!("Bearer {token}")).send_json(&body)?;
            Ok((r.status().as_u16(), r.body_mut().read_to_string()?))
        })?;
        let v: serde_json::Value = serde_json::from_str(&text).map_err(|_| Error::Refused("Google's answer was not understood".into()))?;
        v["id"].as_str().map(str::to_owned).ok_or_else(|| Error::Refused("Google made no album".into()))
    }

    /// Upload one file and add it to the library (and to `album`, if given). The
    /// description is cut to the 1000 characters Google allows.
    pub fn upload_file(&mut self, path: &Path, description: &str, album: Option<&str>) -> Result<UploadedItem> {
        let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "picture".into());
        let mime = mime_for(path);
        let size = std::fs::metadata(path)?.len();
        let upload_url = self.endpoints.upload.clone();

        // Step 1: the bytes. The file is opened afresh for each try and streamed, never held in memory.
        let upload_token = self.call(|token| {
            let file = std::fs::File::open(path)?;
            let mut r = agent()
                .post(&upload_url)
                .header("Authorization", &format!("Bearer {token}"))
                .header("Content-Type", "application/octet-stream")
                .header("X-Goog-Upload-Content-Type", mime)
                .header("X-Goog-Upload-Protocol", "raw")
                .header("X-Goog-Upload-File-Name", &pct_ascii(&file_name))
                .header("Content-Length", &size.to_string())
                .send(file)?;
            Ok((r.status().as_u16(), r.body_mut().read_to_string()?))
        })?;
        let upload_token = upload_token.trim().to_owned();
        if upload_token.is_empty() {
            return Err(Error::Refused("Google accepted the file but gave no upload token".into()));
        }

        // Step 2: make the library item.
        let url = self.endpoints.batch_create.clone();
        let mut request = serde_json::json!({
            "newMediaItems": [{
                "description": description.chars().take(1000).collect::<String>(),
                "simpleMediaItem": { "uploadToken": upload_token, "fileName": file_name },
            }]
        });
        if let Some(a) = album {
            request["albumId"] = a.into();
        }
        let text = self.call(|token| {
            let mut r = agent().post(&url).header("Authorization", &format!("Bearer {token}")).send_json(&request)?;
            Ok((r.status().as_u16(), r.body_mut().read_to_string()?))
        })?;
        let reply: BatchReply = serde_json::from_str(&text).map_err(|_| Error::Refused("Google's answer was not understood".into()))?;
        let first = reply.results.into_iter().next().ok_or_else(|| Error::Refused("Google's answer held no result".into()))?;
        if let Some(s) = &first.status
            && s.code.unwrap_or(0) != 0
        {
            return Err(Error::Refused(s.message.clone().unwrap_or_else(|| "Google could not add the picture".into())));
        }
        let item = first.media_item.ok_or_else(|| Error::Refused("Google added no picture".into()))?;
        Ok(UploadedItem { id: item.id, product_url: item.product_url })
    }

    /// Tell Google the app no longer needs access. Best effort: a network failure is not an error.
    pub fn revoke(&mut self) {
        let _ = agent().post(&self.endpoints.revoke).send_form([("token", self.refresh_token.as_str())]);
        self.access = None;
    }

    /// The long-lived token, which the caller keeps safe.
    pub fn refresh_token(&self) -> &str {
        &self.refresh_token
    }
}

/// HTTP header values are ASCII; keep letters, digits and a few marks, and replace the rest.
fn pct_ascii(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() || " ._-()".contains(c) { c } else { '_' }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sign_in;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};

    /// What the mock saw.
    #[derive(Debug, Clone)]
    struct Seen {
        method: String,
        path: String,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    }

    impl Seen {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
        }
        fn text(&self) -> String {
            String::from_utf8_lossy(&self.body).into_owned()
        }
    }

    /// A server that answers each request with the next scripted `(status, body)` and records the requests.
    fn mock(script: Vec<(u16, &'static str)>) -> (String, Arc<Mutex<Vec<Seen>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for (status, reply) in script {
                let Ok((mut s, _)) = listener.accept() else { return };
                let mut data = Vec::new();
                let mut buf = [0u8; 8192];
                let (head_end, content_length) = loop {
                    let n = s.read(&mut buf).unwrap_or(0);
                    if n == 0 {
                        break (data.len(), 0);
                    }
                    data.extend_from_slice(&buf[..n]);
                    if let Some(p) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&data[..p]).to_ascii_lowercase();
                        let cl = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse().ok()).unwrap_or(0usize);
                        break (p + 4, cl);
                    }
                };
                while data.len() < head_end + content_length {
                    let n = s.read(&mut buf).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    data.extend_from_slice(&buf[..n]);
                }
                let head = String::from_utf8_lossy(&data[..head_end.saturating_sub(4)]).into_owned();
                let mut lines = head.lines();
                let mut first = lines.next().unwrap_or("").split_whitespace();
                let (method, path) = (first.next().unwrap_or("").to_owned(), first.next().unwrap_or("").to_owned());
                let headers = lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned())).collect();
                log.lock().unwrap().push(Seen { method, path, headers, body: data[head_end..].to_vec() });
                let _ = write!(s, "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len());
            }
        });
        (base, seen)
    }

    fn endpoints(base: &str) -> Endpoints {
        Endpoints {
            auth: format!("{base}/auth"),
            token: format!("{base}/token"),
            revoke: format!("{base}/revoke"),
            upload: format!("{base}/v1/uploads"),
            batch_create: format!("{base}/v1/mediaItems:batchCreate"),
            albums: format!("{base}/v1/albums"),
        }
    }

    fn cfg() -> OAuthConfig {
        OAuthConfig { client_id: "client-1".into(), client_secret: "shh".into() }
    }

    fn signed_in(base: &str) -> Client {
        let t = Tokens { refresh_token: "refresh-1".into(), access_token: Some("access-1".into()), expires_at: Some(Instant::now() + Duration::from_secs(3000)) };
        Client::new(endpoints(base), cfg(), t).quick()
    }

    fn scratch_file(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("nv-gphotos-{}-{name}", std::process::id()));
        std::fs::write(&p, bytes).unwrap();
        p
    }

    #[test]
    fn a_file_is_uploaded_then_added_to_the_album() {
        let (base, seen) = mock(vec![
            (200, "UPLOAD-TOKEN-1"),
            (200, r#"{"newMediaItemResults":[{"uploadToken":"UPLOAD-TOKEN-1","status":{"message":"Success"},"mediaItem":{"id":"M1","productUrl":"https://photos.example/M1"}}]}"#),
        ]);
        let file = scratch_file("a.jpg", b"JPEGBYTES");
        let item = signed_in(&base).upload_file(&file, "Portra 400, frame 7", Some("ALBUM-1")).unwrap();
        assert_eq!((item.id.as_str(), item.product_url.as_deref()), ("M1", Some("https://photos.example/M1")));

        let seen = seen.lock().unwrap();
        let up = &seen[0];
        assert_eq!((up.method.as_str(), up.path.as_str()), ("POST", "/v1/uploads"));
        assert_eq!(up.header("authorization"), Some("Bearer access-1"));
        assert_eq!(up.header("x-goog-upload-content-type"), Some("image/jpeg"));
        assert_eq!(up.header("x-goog-upload-protocol"), Some("raw"));
        assert!(up.header("x-goog-upload-file-name").unwrap().ends_with("a.jpg"));
        assert_eq!(up.body, b"JPEGBYTES");

        let create: serde_json::Value = serde_json::from_slice(&seen[1].body).unwrap();
        assert_eq!(create["albumId"], "ALBUM-1");
        assert_eq!(create["newMediaItems"][0]["description"], "Portra 400, frame 7");
        assert_eq!(create["newMediaItems"][0]["simpleMediaItem"]["uploadToken"], "UPLOAD-TOKEN-1");
    }

    #[test]
    fn no_album_means_no_album_id_and_long_descriptions_are_cut() {
        let (base, seen) = mock(vec![(200, "T"), (200, r#"{"newMediaItemResults":[{"status":{"message":"Success"},"mediaItem":{"id":"M2"}}]}"#)]);
        let file = scratch_file("b.png", b"PNG");
        signed_in(&base).upload_file(&file, &"x".repeat(1500), None).unwrap();
        let create: serde_json::Value = serde_json::from_slice(&seen.lock().unwrap()[1].body).unwrap();
        assert!(create.get("albumId").is_none());
        assert_eq!(create["newMediaItems"][0]["description"].as_str().unwrap().len(), 1000);
    }

    #[test]
    fn google_refusing_the_item_is_an_error_with_its_reason() {
        let (base, _) =
            mock(vec![(200, "T"), (200, r#"{"newMediaItemResults":[{"status":{"code":3,"message":"Failed: There was an error while trying to create this media item."}}]}"#)]);
        let file = scratch_file("c.jpg", b"J");
        let e = signed_in(&base).upload_file(&file, "", None).unwrap_err();
        assert!(matches!(&e, Error::Refused(m) if m.contains("error while trying")), "{e:?}");
    }

    #[test]
    fn server_trouble_is_retried_and_the_whole_file_is_sent_again() {
        let (base, seen) =
            mock(vec![(503, "busy"), (429, "slow down"), (200, "TOK"), (200, r#"{"newMediaItemResults":[{"status":{"message":"Success"},"mediaItem":{"id":"M3"}}]}"#)]);
        let file = scratch_file("d.jpg", b"BYTES");
        assert_eq!(signed_in(&base).upload_file(&file, "", None).unwrap().id, "M3");
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 4);
        assert!(seen[..3].iter().all(|s| s.path == "/v1/uploads" && s.body == b"BYTES"));
    }

    #[test]
    fn a_client_error_is_not_retried() {
        let (base, seen) = mock(vec![(400, r#"{"error":{"message":"bad request"}}"#)]);
        let file = scratch_file("e.jpg", b"J");
        let e = signed_in(&base).upload_file(&file, "", None).unwrap_err();
        assert!(matches!(e, Error::Http { status: 400, .. }), "{e:?}");
        assert_eq!(seen.lock().unwrap().len(), 1);
    }

    #[test]
    fn an_expired_token_is_renewed_from_the_refresh_token() {
        let (base, seen) = mock(vec![(200, r#"{"access_token":"access-2","expires_in":3599}"#), (200, r#"{"id":"ALB"}"#)]);
        let t = Tokens { refresh_token: "refresh-1".into(), access_token: Some("old".into()), expires_at: Some(Instant::now()) };
        let id = Client::new(endpoints(&base), cfg(), t).quick().create_album("Roll 12").unwrap();
        assert_eq!(id, "ALB");
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0].path, "/token");
        let form = seen[0].text();
        assert!(form.contains("grant_type=refresh_token") && form.contains("refresh_token=refresh-1") && form.contains("client_id=client-1"), "{form}");
        assert_eq!(seen[1].header("authorization"), Some("Bearer access-2"));
        assert!(seen[1].text().contains("Roll 12"));
    }

    #[test]
    fn a_rejected_token_is_renewed_once_then_means_sign_in_again() {
        let (base, _) = mock(vec![(401, "no"), (200, r#"{"access_token":"access-2","expires_in":3599}"#), (401, "still no")]);
        let e = signed_in(&base).create_album("x").unwrap_err();
        assert!(matches!(e, Error::NotSignedIn), "{e:?}");
    }

    #[test]
    fn a_revoked_refresh_token_means_sign_in_again() {
        let (base, _) = mock(vec![(400, r#"{"error":"invalid_grant","error_description":"Token has been expired or revoked."}"#)]);
        let t = Tokens { refresh_token: "gone".into(), access_token: None, expires_at: None };
        let e = Client::new(endpoints(&base), cfg(), t).quick().create_album("x").unwrap_err();
        assert!(matches!(e, Error::NotSignedIn), "{e:?}");
    }

    #[test]
    fn media_types_follow_the_extension() {
        assert_eq!(mime_for(Path::new("a.JPG")), "image/jpeg");
        assert_eq!(mime_for(Path::new("a.png")), "image/png");
        assert_eq!(mime_for(Path::new("a.tif")), "image/tiff");
        assert_eq!(mime_for(Path::new("a.dng")), "application/octet-stream");
    }

    #[test]
    fn sign_in_follows_the_browser_back_and_trades_the_code() {
        let (base, seen) = mock(vec![(200, r#"{"access_token":"A","refresh_token":"R","expires_in":3599}"#)]);
        let open = |url: &str| {
            // Play the browser: read the request Google would get, then come back with a code.
            let q = url.split_once('?').unwrap().1;
            let get = |k: &str| q.split('&').find_map(|p| p.strip_prefix(&format!("{k}="))).map(crate::oauth::pct_decode).unwrap();
            assert_eq!(get("client_id"), "client-1");
            assert_eq!(get("code_challenge_method"), "S256");
            let (redirect, state) = (get("redirect_uri"), get("state"));
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(100));
                let _ = ureq::get(&format!("{redirect}/?code=CODE%2F1&state={state}")).call();
            });
        };
        let t = sign_in(&endpoints(&base), &cfg(), &open, &AtomicBool::new(false), Duration::from_secs(10)).unwrap();
        assert_eq!(t.refresh_token, "R");
        assert_eq!(t.access_token.as_deref(), Some("A"));
        let form = seen.lock().unwrap()[0].text();
        assert!(form.contains("grant_type=authorization_code") && form.contains("code=CODE%2F1") && form.contains("code_verifier="), "{form}");
    }

    #[test]
    fn sign_in_refuses_an_answer_with_the_wrong_state() {
        let (base, seen) = mock(vec![]);
        let open = |url: &str| {
            let redirect = url.split('&').find_map(|p| p.strip_prefix("redirect_uri=")).map(crate::oauth::pct_decode).unwrap();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(100));
                let _ = ureq::get(&format!("{redirect}/?code=C&state=forged")).call();
            });
        };
        let e = sign_in(&endpoints(&base), &cfg(), &open, &AtomicBool::new(false), Duration::from_secs(10)).unwrap_err();
        assert!(matches!(e, Error::Refused(_)), "{e:?}");
        assert!(seen.lock().unwrap().is_empty(), "no code may be traded after a bad state");
    }

    #[test]
    fn sign_in_can_be_cancelled_and_can_time_out() {
        let e = sign_in(&Endpoints::google(), &cfg(), &|_| {}, &AtomicBool::new(true), Duration::from_secs(10)).unwrap_err();
        assert!(matches!(e, Error::Cancelled));
        let e = sign_in(&Endpoints::google(), &cfg(), &|_| {}, &AtomicBool::new(false), Duration::from_millis(200)).unwrap_err();
        assert!(matches!(e, Error::Cancelled));
        let e = sign_in(&Endpoints::google(), &OAuthConfig::default(), &|_| {}, &AtomicBool::new(false), Duration::from_secs(1)).unwrap_err();
        assert!(matches!(e, Error::Refused(_)));
    }
}
