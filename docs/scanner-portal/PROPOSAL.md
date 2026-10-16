# Scanner portal: draft proposal

Status: draft, prototyped in Negative Vibes, intended for upstream xdg-desktop-portal.
Scope of v1: SANE only. eSCL / IPP-scan providers come later.

Crates: `nv-sane-portal` (client: proxies and the portal-backed `ScannerBackend`, what the
app uses) and `nv-portal-proto` (the test-double service, scenarios and parity test).

Files here:

- `org.freedesktop.portal.Scanner.xml`: frontend interface (what apps call)
- `org.freedesktop.impl.portal.Scanner.xml`: backend interface (UI only)

## Problem

Flatpak apps have no portal for scanners. They must bundle sane-backends and take
`--device=all` (USB) and `--share=network` (network scanners), so they cannot use
host-installed or proprietary drivers, cannot see the host's SANE configuration, and the
user cannot see or revoke scanner access. Printing has a portal. Scanning does not.

## Design in one paragraph

Print portal style consent, USB portal style fd handover. The user picks scanners in a
portal dialog (`SelectScanners`); the grant is stored per app in the permission store.
The app opens a granted scanner (`OpenScanner`, a `Session`), reads SANE-style typed
options (`GetOptions`, `SetOptions`) and pulls image data one frame at a time
(`StartFrame`) through a pipe fd, with `FrameEvent` signals for state. libsane runs on the
host, never in the sandbox.

## Decisions and why

| Decision | Reason |
|---|---|
| SANE-shaped option model, not a reduced "resolution/mode/source" API | Film scanning needs infrared, 16-bit, multi-pass and backend-specific options. A reduced API would lose them. Well-known option names (`resolution`, `mode`, `tl-x`) keep simple apps simple. |
| Pipe fd per frame, not a file | A 7200 dpi frame is hundreds of MB. A pipe gives backpressure and no temp files. One frame per call mirrors `sane_start` and covers RGB+IR. |
| `StartFrame` returns at once; state in `FrameEvent` | `sane_start` can block for many seconds (warm-up, carriage). End of data on a pipe cannot tell success from failure, so `finished` is explicit. |
| Opaque ids from `SelectScanners`, never raw SANE names | Some backends interpret device names as paths or hosts. Apps must not steer the host to arbitrary files or addresses. |
| App sees only granted scanners | No LAN scanner enumeration by sandboxed apps (fingerprinting, privacy). |
| Typed D-Bus values | Print's all-string settings are a mistake not to copy. |
| Ordered `a(sv)` for `SetOptions`, not `a{sv}` | Found while prototyping: D-Bus dictionaries do not guarantee order, and SANE needs it (`mode` before `three-pass`, which is inactive in gray). |
| `FrameEvent` carries a per-session frame number | Found by the second application: a client that cancels has to wait for the cancelled frame's last event, or its next frame meets it. With a number on each event it can drop leftovers. |
| Automatic values as the vardict `{'auto': true}` | SANE has `SET_AUTO`; Simple Scan uses it for the scan source. D-Bus has no empty structure, so the "no value" marker is a vardict, which also leaves room for other per-option modifiers. |
| `Session` for an open scanner | Free lifecycle: app exit releases the lock, the portal can revoke with `Session.Closed`. |
| No per-scan prompt after grant | Scanning needs a physical action. Revisit if reviewers disagree. |
| Backend = UI only | Same split as `impl.portal.Usb`. |

## Open questions (for upstream review)

1. **Where does libsane run?** The frontend must not link libsane (no thread safety, can load
   proprietary code). Options: (a) a per-session host helper process spawned by the frontend,
   (b) a separate systemd user service the frontend talks to, which can also serve eSCL
   later. Prototype uses (a). Upstream will want a view.
2. **Hotplug / availability** for network scanners: poll via the helper, or rely on avahi.
3. **Button options and filename options:** the draft hides filename-type options and
   refuses to set them. The exact filter list needs a pass over the SANE backends.
4. **Hardware buttons** (scan-to-app): out of scope for v1.
5. **Multiple apps, one scanner:** exclusive lock for v1; how a busy scanner is reported.
6. **Flatpak permission model:** `--device=all` can be dropped; the prototype needed only
   `--talk-name` for its own bus name. The real portal would be covered by
   `--talk-name=org.freedesktop.portal.Desktop`, which apps already have (not tested).
7. **Bundled backend fallback:** exotic scanners (our `plustek8200i`) still need the USB
   portal and bundled backends until the host ships their driver.

## Prototype plan (in Negative Vibes)

Goal: exercise the whole API with a real app before proposing it upstream.

1. **Trait in `nv-sane`.** The wrapper already has the right shape: `Session::get_devices`,
   `open`, `Handle::all_options`, `get_option`, `set_option`, `get_parameters`, `start`,
   `read`, `cancel`. Put these behind a `ScannerBackend` trait with a `LibSane`
   implementation (today's code) and a `Portal` implementation. The single worker thread
   stays; it owns one backend.
2. **Prototype portal service** (new crate `crates/nv-portal-proto`, Rust, `zbus`). Implements
   `org.freedesktop.portal.Scanner` on the session bus under a test name, using the same
   libsane wrapper in a helper subprocess per session. Permission store and the dialog are
   stubbed (auto-grant, logged). This is only a test double for the API; the real thing
   belongs in xdg-desktop-portal (C).
3. **Portal client in the app.** Behind a cargo feature and a runtime switch, with a
   `--talk-name` for the test service in the Devel manifest. Pass `Cancel`, an empty feeder
   (`no-documents`), revoke (`Session.Closed`) and a mid-frame close of the pipe.
4. **Verify** with the §11.2 test backend matrix and the §11.3 simulator, then real scans on
   the 8200i, comparing bytes against the direct libsane path (they must match exactly).
5. **Port to xdg-desktop-portal** in C: `src/scanner.c`, permission store table `scanner`,
   `data/org.freedesktop.portal.Scanner.xml`, backend stub in `xdg-desktop-portal-gtk`,
   docs and a test in `tests/`. Then open the upstream issue with the measured results.

## Prior art

- sane-devel, Sept 2020, "Sandboxing scanner applications" (Bastien Nocera): host-side
  service preferred over bundled drivers.
- PWG IPP-scan and WS-Scan: similar to each other; reviewers asked that the model not
  be locked to SANE. The v1 option vocabulary is abstract enough (typed, constrained,
  units) to be fed by an eSCL provider later.

## Prototype status (step 2)

`crates/nv-portal-proto` implements the draft interface over libsane: `nv-portal-proto`
(service, on its own bus name, never the real portal's) and `nv-portal-proto-client`.
Stubbed: the permission store (grants in memory, global) and the dialog (auto-selects, with
`--select SUBSTRING`). Not implemented: `ScannerEvents` (declared, never emitted).

Verified against the SANE `test` backend, which needs no hardware (`tests/run-scenarios.sh`
runs 28 checks on a private bus; the client's output matches `scanimage` byte for byte for
gray, 8-bit RGB, 16-bit RGB and three-pass red/green/blue frames):
selection and grants, opaque ids, the exclusive lock, other apps refused on someone else's
session, option errors, option order, `Cancel()`, closing the pipe early, release when the app
leaves the bus, hand scanner (`lines` = -1), and `no-documents` from an empty feeder.

### What the prototype found

1. **Ordered option list.** `SetOptions` is now `a(sv)` (see the table above).
2. **A full pipe stalls Cancel.** If the app does not read, the pipe fills at 64 KB, the
   writer blocks, and `Cancel()` is never seen: the SANE thread hung until the app read or
   closed the pipe, holding up every other session. Fixed with a non-blocking writer that
   polls and checks the cancel flag. Any implementation of this API needs the same care, so
   it belongs in the interface docs: *a frame is cancelled by `Cancel()` or by closing the
   pipe, and neither may depend on the app reading*.
3. **Arrays are missing.** `nv-sane` reads one value per option, so array options (gamma
   tables, some calibration data) cannot be carried yet. The prototype hides them. Either
   v1 says options are scalar, or the descriptor needs an array form (the draft's `size` is
   in place for it, but nothing fills it in).
4. **The device-type filter.** The app skips SANE type "virtual device" to hide webcams, but
   the SANE `test` backend has that type too. The portal should drop `v4l:` by backend
   name, and the app should not rely on the type string.
5. **One SANE thread blocks all sessions** while a frame runs. That is the prototype's
   limit and the reason the proposal wants one helper process per session.

## Prototype status (step 3): the app on the portal

`nv-sane-portal::PortalBackend` implements `nv_sane::ScannerBackend`/`ScannerHandle`. The
acquisition worker picks it at run time: build with `--features portal` (on `nv-acquire`,
`nv-cli` or `negative-vibes`) and set `NV_SCANNER_BACKEND=portal`; `NV_PORTAL_BUS_NAME`
names the portal (default: the prototype's `org.freedesktop.NvPortalProto`). Without the
feature the variable is an error, never a silent fallback to libsane. The Devel manifest
gets `--talk-name=org.freedesktop.NvPortalProto`; its `--device=all` stays until the
portal path is the default.

Verified: `nv-cli scan` through the worker completes a 16-bit RGB scan over the portal, and
`cargo test -p nv-portal-proto --test backend_parity` (private bus, SANE test backend) scans
the same options through the portal and through libsane via the same traits: frame
parameters and pixel data are identical for gray, 8-bit, 16-bit and three-pass; option
descriptors and constraints match; a busy scanner, an empty feeder, a rejected value and a
cancel with an unread pipe all arrive as the SANE conditions the worker already handles.
**Not verified at that point:** the GTK app itself (nothing was clicked), a real scanner (the
8200i), the Flatpak sandbox, or a real portal dialog. **Since then** the 8200i has been checked:
the portal path matches libsane on colour, infrared, repeat and cancel, within sensor noise
(DESIGN.md §10). The Flatpak sandbox has been checked too: the scenarios pass from a build with no
`--device=all` and no network, and the GTK app made a preview and a raw scan on the 8200i from
it, so fd passing and signals work through the Flatpak D-Bus proxy (DESIGN.md §10). A real
portal dialog and clicking through the GUI are still not verified.

### What step 3 found

1. **Frame parameters come only with `started`.** libsane allows `sane_get_parameters`
   before `sane_start` (an estimate for sizing the UI). The portal gives them once the frame
   starts. The worker only reads them after `start`, so nothing broke, but a client that
   wants the size up front has no call for it. Consider `GetFrameParameters` (estimate).
2. **"Busy" has no portal error.** The portal's error set has `Failed`, `InvalidArgument`,
   `NotAllowed`; a locked scanner is `Failed`, which the client maps to `DEVICE_BUSY`. That
   is a guess from one cause. Either document that `Failed` from `OpenScanner` means busy, or
   add a dedicated error name.
3. **`SelectScanners` needs a window.** The worker has no window handle, so the backend
   passes an empty `parent_window` and `get_devices` opens the dialog by itself when nothing
   is granted. In the app, selection should be a UI action that passes its window identifier;
   `get_devices` should then only list.
4. **Device identity changes meaning.** The portal's opaque id replaces SANE's device name
   as what `open` takes. It is stable (a hash of the name here), so a stored name keeps
   working with the same backend, but a name saved under libsane will not resolve under the
   portal. Anything that persists `device_name` (roll metadata, settings) must treat it as
   backend-specific, or store vendor and model next to it.
5. **`DeviceInfo.kind` differs.** libsane's field is the SANE type string ("film scanner"),
   the portal's was the connection (`usb`, `network`). The app's webcam filter keys on the
   type string, so it only worked for libsane. Fixed by the second application's findings
   below: the portal now sends the type string as `type`.
6. **Option state is the backend's.** The SANE test backend keeps option values per device,
   not per handle, so a session can inherit another's settings. That is libsane behaviour
   the portal passes through; apps should set every option they depend on.

### Step 3 findings: what was done

1. **Frame parameters before a frame: fixed.** New `GetFrameParameters` (an estimate, same
   keys as `started`; `Busy` during a frame). The client's `get_parameters` returns the final
   values once a frame has started, the estimate otherwise, and drops the cached values when
   a frame ends or an option is set.
2. **Busy: fixed.** New `org.freedesktop.portal.Error.Busy` for a locked scanner, a frame in
   progress, and `SetOptions` or `GetFrameParameters` during one. `Failed` is no longer
   mapped to "busy". `Busy` is not in xdg-desktop-portal's error set; upstream may prefer
   another name.
3. **Selection needs a window: fixed in the API, not yet in the GTK app.** `get_devices` only
   lists. `ScannerBackend::select_devices(parent_window)` shows the dialog; the worker has a
   `SelectScanners` command, the app an `app.select-scanner` action and a "Choose Scanner…"
   button on the "No Scanner Found" page (shown only with `NV_SCANNER_BACKEND=portal`), and
   `nv-cli scan` asks once when nothing is granted. **The app still passes an empty window
   identifier**: ashpd's GTK helper needs gtk4 0.11 and the app uses 0.9, so exporting a
   handle needs `gdk4-wayland`/`gdk4-x11` at 0.9. Until then the real portal's dialog would
   not be attached to the app's window.
4. **Device identity: bigger than recorded, fixed.** Nothing persists `device_name`, but
   quirks are matched on the backend name, which the code took from the device name. Through
   the portal that is an opaque id, so the 8200i's quirks (infrared, neutralised options)
   would silently never have matched. The portal now reports `backend`, `DeviceInfo` carries
   it, and the worker and CLI match quirks on it. Covered by the parity test.
5. **Webcam filter keyed on the SANE type: fixed.** `DeviceInfo::is_webcam()` (backend or name
   `v4l`, or a camera type) is used by the worker and the service. The worker's "virtual
   device" exclusion stays for libsane (it hides the SANE test backend), with
   `NV_SANE_ALLOW_VIRTUAL=1` to allow it for testing. The portal now reports the SANE type too
   (`type`), and the client puts it in `DeviceInfo.kind`, so that exclusion works through it.
6. **Option state shared across sessions: not changed.** It is how SANE's test backend
   behaves with libsane too. The tests set every option they depend on.

Also found while verifying: `Handle::set_option` wrote an int's bits into a fixed-point
option, so the app's integer resolution became about 0.001 dpi on the SANE test backend
(3x3 pixels). The portal service converted by option type and was right. `Handle::set_option`
now converts between int and fixed by the descriptor, so both backends agree. Real film
scanners have integer resolution; the 8200i is not affected.

## What a second application found

GNOME Simple Scan was ported to the draft (`SIMPLE_SCAN_BACKEND=portal`; a `ScanBackend`
interface with a libsane and a portal implementation). It ran in a Flatpak without
`--device=all` and `--share=network` against the prototype service and scanned on the
OpticFilm 8200i. What it needed beyond the first application:

1. **Automatic values.** `SetOptions` could not express SANE's `SET_AUTO`, which Simple Scan
   uses to pick the scan source, so the port hid the `automatic` capability and fell back to
   choosing by name. `SetOptions` now accepts the vardict `{'auto': true}` as a value, for
   options that have the `automatic` capability (others are `InvalidArgument`). It is a vardict
   because D-Bus has no empty structure; `()` would not have been valid on the wire.
2. **A frame number on `FrameEvent`.** A client that cancels, or starts the next frame at once,
   had to drain events and wait for the old frame's last event by hand. Each `StartFrame` now
   starts a numbered frame (from 1, per session), every event carries `frame`, and a frame
   ends with exactly one event that is not `started`. A client drops events that are not newer
   than the last frame it saw end. The Rust client does this; the Simple Scan port still uses
   the older drain-and-wait and works, and can use the number.
3. **The SANE device `type` in the scanner info.** Simple Scan hides `virtual device` and ranks
   by type; `kind` (the connection) cannot say that. `type` is SANE's string, untranslated, as a
   hint.
4. **Documentation only:** option numbering (the portal leaves out SANE's option 0, so a
   port that numbers as SANE does starts at 1, and groups stay in the list), the value types
   (`b`, `i`, `d`, `s`; int ranges are `(ddd)`), cancelling `SelectScanners` with
   `Request.Close`, selecting on a user action, and how an app finds out the portal exists.

Not changed: string option sizes and the `info` flags, which Simple Scan never needed.

After these changes the service, the Rust client and the Simple Scan port were run again on the
8200i: the hardware check passed 23 of 23 (including colour+infrared, two numbered frames in one
session, and cancel then rescan), Simple Scan scanned from the sandbox, was cancelled
mid-frame and scanned again in the same session, and the device arrived as `film scanner`. The
scenario script has 40 checks (10 new: type, automatic values, frame numbers). Simple Scan did
not exercise `{'auto': true}`: the test backend that has an automatic option is hidden by its
own filter, and the 8200i has none; the Python scenarios and the parity test cover it.
