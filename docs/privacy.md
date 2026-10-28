# Negative Vibes privacy policy

Effective: 2026-10-28

Negative Vibes is a free, open-source (GPL-3.0-or-later) desktop application for scanning and
converting film negatives. It runs on your computer. This page says what it does with your data,
which is almost nothing outside your computer. The one exception is the optional Google Photos
feature.

## What stays on your computer

Your scans, rolls, settings, exported pictures and film profiles are stored on your computer, in
the folders you choose and in the application's own data folder. The application has no account
system, no analytics or telemetry, and no advertising. We, the developers, receive nothing from it
and have no server that it talks to.

## Google Photos (optional)

Negative Vibes can add your exported pictures to your Google Photos library. This is off until you
choose **Sign In** in Preferences. If you never sign in, the application makes no connection to
Google.

**What it asks Google for.** One permission:
`https://www.googleapis.com/auth/photoslibrary.appendonly`. It lets the application add pictures
to your library and create albums for them. It does **not** let the application see, list, change
or delete any picture or album already in your library.

**What it sends to Google.** Only when you export with *Send to Google Photos* turned on:

- the exported picture file, and its description (the caption you wrote, the film, the roll
  name and the frame number);
- the name of the roll, as an album title, if *Put the Roll in an Album* is on.

**What it keeps.** After you sign in, Google gives the application a refresh token. It is stored
on your computer in the system keyring (in a Flatpak, through the Secret portal). The application
never sees your Google password, and it does not store your name, your email address or anything
about your library. The token is not sent anywhere except to Google.

**Sign out and revoke.** **Sign Out** in Preferences deletes the token from the keyring and tells
Google the access is no longer needed. You can also remove the access at any time in your Google
Account, under Security, "Your connections to third-party apps and services".

**Limited use.** Negative Vibes's use and transfer to any other app of information received from
Google APIs adheres to the
[Google API Services User Data Policy](https://developers.google.com/terms/api-services-user-data-policy),
including the Limited Use requirements, and the Google Photos APIs User Data and Developer Policy.
The application:

- uses the access only to upload the pictures you export and make the albums for them, which is
  the feature you see;
- does not transfer data from Google to anyone, and does not sell or share user data;
- does not use the data for advertising, and does not use it to train any model;
- does not let people read your data: nothing from Google is collected by us at all.

## Children

The application is not aimed at children and collects no personal information from anyone.

## Changes

If this policy changes, the new version is published at this address with a new effective date.
The history is in the project's git repository.

## Contact

Questions: christian.schaller@gmail.com or an issue at https://github.com/cschalle/negative-vibes/issues.
