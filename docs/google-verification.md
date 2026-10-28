# Google OAuth verification: what to submit

Notes for the Google Auth Platform Verification Center. Not part of the application.

## Basics

| Field | Value |
|---|---|
| App name | Negative Vibes |
| User type | External |
| Scope | `https://www.googleapis.com/auth/photoslibrary.appendonly` (the only one) |
| Client type | Desktop app (installed application, loopback redirect with PKCE) |
| Home page | https://linuxrising.org/negative-vibes/ |
| Privacy policy | https://linuxrising.org/negative-vibes/privacy.html (the HTML copy of `docs/privacy.md`; keep them the same) |
| Authorized domain | `linuxrising.org` |
| Support / developer contact | christian.schaller@gmail.com |
| Source code | https://github.com/cschalle/negative-vibes (GPL-3.0-or-later) |

The home page must describe the app, link to the privacy policy, and be on the authorized domain.
The privacy policy must be reachable without signing in.

## Scope justification

Paste into the "How will the scopes be used" field:

> Negative Vibes is a desktop application that turns scans of photographic film negatives into
> finished pictures. After the user exports a picture, the user can choose "Send to Google Photos".
> The application then uploads that one exported picture to the user's own Google Photos library,
> and, if the user turned on "Put the Roll in an Album", creates an album named after the roll
> and adds the picture to it.
>
> We request only `photoslibrary.appendonly`, because adding pictures and creating albums for
> them is the entire feature. We do not read, list, search, edit or delete anything in the user's
> library, so no read or edit scope is requested, and the narrowest scope the Library API offers
> is used. The app does not use the Picker API because it never lets the user choose existing
> photos.
>
> The only data the app stores from Google is the OAuth refresh token, kept in the user's system
> keyring on their own computer. Uploaded content comes from the user's own exported files. We
> operate no server: the app talks directly to Google from the user's computer, and no data from
> Google is collected, shared or sold by us. The user can sign out in the app (which revokes the
> token) or remove access in their Google Account at any time.

## Demo video script

Upload to YouTube as unlisted and put the link in the form. Screen-record at 1080p, narrate or
caption each step, and keep the browser address bar visible during the consent screen.

1. **Introduce** (10 s). "This is Negative Vibes, a desktop app for scanning film negatives.
   I will show how it uses Google sign-in and the Google Photos append-only scope."
2. **Preferences → Google Photos** (10 s). Show the *Sign In…* button and the text saying the
   app can only add pictures.
3. **Sign in** (30 s). Press *Sign In…*. In the browser show the **consent screen with the
   address bar visible**, so the `client_id` is readable. Read out the permission it lists
   (add to your Google Photos library). Allow.
4. **Back in the app** (5 s). The row now says *Signed in*.
5. **Export** (30 s). Open a roll, open Export (Ctrl+E), choose the *Google Photos* preset, tick
   *Put the Roll in an Album*, export one frame.
6. **Result** (20 s). Open Google Photos in a browser and show the new album and picture, with
   the description the app wrote.
7. **Sign out and revoke** (15 s). Press *Sign Out* in Preferences, then show the app listed
   (or no longer listed) under Google Account → Security → Third-party connections.

## Likely questions

- **Why does the app need a client secret in the binary?** Google issues one for Desktop clients.
  An installed app cannot keep it confidential, and PKCE protects the code exchange.
- **Where is the token stored?** The system keyring; in the Flatpak, the Secret portal.
- **Does any server of yours see user data?** No. There is none.
- **Why not `photoslibrary.readonly.appcreateddata`?** The app never reads anything back. An
  album is created once, and its id is remembered locally.
