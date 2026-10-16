# Flathub submission notes

`io.github.cschalle.NegativeVibes.json` is a starting point, not a working build yet.

1. Generate offline crate sources: `flatpak-cargo-generator.py ../../Cargo.lock -o cargo-sources.json`
   (from <https://github.com/flatpak/flatpak-builder-tools>), and make the Meson build run Cargo with `--offline`.
2. Fill in the `sane-backends` checksum, or use the SDK's libsane if the runtime ships one; the
   `plustek8200i` backend has to be built into it, or the app cannot see that scanner inside the sandbox.
3. Check that the GNOME runtime version listed provides GTK 4.14 and libadwaita 1.4 or newer, which Meson requires (DESIGN.md §4.1).
4. ArgyllCMS (IT8 calibration) is optional; add a module for it, or leave the feature greyed out.
5. Add screenshots to `data/*.metainfo.xml.in` (`<screenshots>`), and run
   `flatpak run org.freedesktop.appstream.cli validate` on the result.
6. Confirm the application ID matches the GitHub account that will own the repository.
7. Google Photos needs an OAuth client (see the README). Pass `NV_GOOGLE_CLIENT_ID` and
   `NV_GOOGLE_CLIENT_SECRET` in `build-options.env` once the client has passed Google's app
   verification for the `photoslibrary.appendonly` scope; until then, ship without them and users
   enter their own under Preferences. The sign-in token is stored through the Secret portal.
