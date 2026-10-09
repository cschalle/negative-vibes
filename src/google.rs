//! Google Photos, the application's side: which OAuth client to use, and
//! where the long-lived sign-in token is kept (the system keyring, through the
//! Secret portal inside a Flatpak). The network code is in `nv-gphotos`.

use nv_gphotos::OAuthConfig;

use crate::prefs::Prefs;

const ATTRIBUTES: [(&str, &str); 2] = [("application", "io.github.cschalle.NegativeVibes"), ("service", "google-photos")];

/// The client Preferences names, else the one built into this copy.
pub fn oauth_config(prefs: &Prefs) -> OAuthConfig {
    if !prefs.google_client_id.trim().is_empty() {
        return OAuthConfig { client_id: prefs.google_client_id.trim().to_owned(), client_secret: prefs.google_client_secret.trim().to_owned() };
    }
    OAuthConfig { client_id: crate::config::GOOGLE_CLIENT_ID.to_owned(), client_secret: crate::config::GOOGLE_CLIENT_SECRET.to_owned() }
}

/// The saved refresh token, if there is one. Blocks on the keyring: call from a worker thread.
pub fn load_refresh_token() -> Option<String> {
    futures_lite::future::block_on(async {
        let keyring = oo7::Keyring::new().await.ok()?;
        let items = keyring.search_items(&ATTRIBUTES).await.ok()?;
        let secret = items.first()?.secret().await.ok()?;
        String::from_utf8(secret.to_vec()).ok().filter(|t| !t.is_empty())
    })
}

/// Keep the refresh token. Blocks on the keyring: call from a worker thread.
pub fn save_refresh_token(token: &str) -> Result<(), String> {
    futures_lite::future::block_on(async {
        let keyring = oo7::Keyring::new().await.map_err(|e| e.to_string())?;
        keyring.create_item("Negative Vibes: Google Photos", &ATTRIBUTES, token.as_bytes(), true).await.map_err(|e| e.to_string())
    })
}

/// Forget it. Blocks on the keyring: call from a worker thread.
pub fn delete_refresh_token() {
    let _ = futures_lite::future::block_on(async {
        let keyring = oo7::Keyring::new().await.map_err(|e| e.to_string())?;
        keyring.delete(&ATTRIBUTES).await.map_err(|e| e.to_string())
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferences_override_the_built_in_client() {
        let mut p = Prefs::default();
        assert_eq!(oauth_config(&p).client_id, crate::config::GOOGLE_CLIENT_ID);
        p.google_client_id = "  mine.apps.googleusercontent.com ".into();
        p.google_client_secret = " s ".into();
        assert_eq!(oauth_config(&p), OAuthConfig { client_id: "mine.apps.googleusercontent.com".into(), client_secret: "s".into() });
    }
}
