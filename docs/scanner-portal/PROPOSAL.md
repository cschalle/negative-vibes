# Scanner portal

A proposal for a scanner portal in xdg-desktop-portal, so that sandboxed applications can
scan without bundling drivers or holding `--device=all`.

Status: draft for review. The interfaces below are implemented by a prototype service and
have been driven by two applications, on real hardware and from inside a Flatpak sandbox.
Nothing is final; the open questions in the last section are the parts that most need a
decision from upstream.

Scope of v1: scanners driven by SANE. eSCL and IPP-Scan providers are not part of v1, but
the option model is deliberately abstract enough to be fed by one later.

Interface definitions in this directory:

- `org.freedesktop.portal.Scanner.xml` — frontend interface, what applications call
- `org.freedesktop.impl.portal.Scanner.xml` — backend interface, UI only

## Problem

There is no portal for scanners. A Flatpak application that wants to scan has to bundle
sane-backends and hold `--device=all` for USB scanners and `--share=network` for network
ones. That has a cost on every side:

- **Security.** `--device=all` is access to all of `/dev`, and `--share=network` is the whole
  network, both granted permanently at install time for a device the user may use twice a year.
- **Drivers.** The bundled backends are the only ones the application can use. Host-installed
  drivers, out-of-tree drivers and proprietary vendor drivers are all invisible to it, even
  when the same scanner works fine outside the sandbox.
- **Configuration.** The host's SANE configuration — `dll.conf`, per-backend configuration,
  network scanner definitions — does not reach the sandbox, so a scanner the user already
  set up has to be set up again, if it can be at all.
- **Control.** The user cannot see which applications may scan and cannot revoke it. There is
  no prompt, no list, no entry in the permission store.
- **Duplication.** Every application ships and updates its own copy of sane-backends.

Printing has had a portal for years. Scanning, which is the same kind of device access with
the same kind of consent question, has none.

## Goals

- A sandboxed application can scan with no device, network or filesystem access.
- The user chooses which scanners an application may use, in a dialog, once, and can revoke
  the choice afterwards.
- libsane and its backends run on the host, so host and proprietary drivers work and the
  host's SANE configuration applies.
- The API is rich enough for demanding applications: multi-bit-depth, infrared, multi-pass,
  and backend-specific options all survive it.
- Simple applications stay simple: well-known option names mean a "resolution and mode" UI
  needs no special cases.

## Non-goals for v1

- Scanner **discovery** by sandboxed applications. An application sees only what it was
  granted, never a list of what is on the network.
- **Hardware buttons** (scan-to-application), which need a dispatch model of their own.
- **eSCL / IPP-Scan** as a provider, though the model is meant to allow it later.
- A **per-scan prompt**. Scanning requires a physical action by the user, so the grant is the
  consent; see the decisions table.

## Architecture

The split follows the USB portal: the backend draws the dialog and nothing else, and the
frontend does the work.

```
  sandbox                     |  host
                              |
  application                 |  xdg-desktop-portal
    |                         |    org.freedesktop.portal.Scanner  (frontend)
    |  D-Bus calls -----------+---->  |
    |                         |       |-- permission store, table "scanner"
    |  pixel data <-----------+-------|-- helper process per session -- libsane -- scanner
    |      (pipe fd)          |       |
                              |       '-- org.freedesktop.impl.portal.Scanner  (dialog)
                              |             in xdg-desktop-portal-gtk / -kde
```

Two properties of this layout matter:

**The frontend must not link libsane.** libsane is not thread-safe, it loads backend code
into the process, and that code can be proprietary and of unknown quality. Running it in
xdg-desktop-portal itself would put it in a long-lived process that every portal shares. It
therefore belongs in a helper, one per open scanner, which also means a wedged backend
cannot take down other applications' scans. How that helper is launched and managed is the
first open question below.

**Pixel data does not travel over D-Bus.** A single 7200 dpi frame is hundreds of megabytes.
`StartFrame` returns the read end of a pipe, so the data path is a plain fd with kernel
backpressure, no temporary files and no message size limits — the same handover the USB
portal uses.

## The interface

Consent and enumeration:

| Member | Purpose |
|---|---|
| `SelectScanners(parent_window, options) → Request` | Shows the dialog and stores the grant. Called on a user action, or when `ListScanners` comes back empty — not on every start. `options` can ask for `multiple` and filter on `required_capabilities` (`film`, `adf`, `infrared`, `flatbed`, `color`). |
| `ListScanners(options) → a(sa{sv})` | The granted, currently available scanners. Each is an opaque id plus `name`, `vendor`, `model`, `backend`, `kind`, `type`, `capabilities`, `available`. |
| `CreateSession(options) → Session` | A monitoring session, needed only for `ScannerEvents`. |
| `ScannerEvents(session, a(ssa{sv}))` | `add`, `change` or `remove` for granted scanners only. |

Scanning, on a `Session` from `OpenScanner`:

| Member | Purpose |
|---|---|
| `OpenScanner(scanner_id, options) → Session` | Opens a granted scanner and takes the host's exclusive lock. May take seconds (lamp warm-up, calibration). Closing the session, or leaving the bus, cancels any scan and releases the scanner. |
| `GetOptions(session, options) → aa{sv}` | SANE-shaped option descriptors in backend order: `name`, `title`, `description`, `type`, `unit`, `size`, `constraint`, `capabilities`. |
| `GetOptionValues(session, options) → a{sv}` | The current value of every readable, active option. |
| `SetOptions(session, a(sv), options) → applied, flags` | Sets options **in array order**. Returns what each option ended up with, and flags `inexact`, `reload-options`, `reload-params`. |
| `GetFrameParameters(session, options) → a{sv}` | The layout the next frame would have, as an estimate, for sizing buffers and previews. |
| `StartFrame(session, options) → fd` | Starts one frame and returns at once with the read end of a pipe. |
| `Cancel(session)` | Cancels the frame in progress. |
| `FrameEvent(session, state, details)` | `started`, `finished`, `cancelled`, `no-documents` or `error`. `started` carries `format`, `depth`, `pixels_per_line`, `bytes_per_line`, `lines` and `last_frame`. Every event carries its frame number. |

A typical exchange:

```
SelectScanners()                     once, on a user action
ListScanners()                       → [(id, {name: "Plustek OpticFilm 8200i", …})]
OpenScanner(id)                      → session
GetOptions(session)                  → descriptors
SetOptions(session, [("mode","Color"), ("resolution",3600)])
loop:
    StartFrame(session)              → fd
    FrameEvent started               → format, depth, bytes_per_line, lines, last_frame
    read(fd) until EOF
    FrameEvent finished
    until last_frame
Session.Close(session)
```

An application detects the portal by checking for the `version` property of
`org.freedesktop.portal.Scanner` on `org.freedesktop.portal.Desktop`, and falls back to
libsane when it is absent.

The backend interface has a single method, `SelectScanners`, which receives the scanner list
the frontend assembled and returns the ids the user picked. The frontend stores the grant.

## Decisions and why

| Decision | Reason |
|---|---|
| A SANE-shaped typed option model, not a reduced `resolution`/`mode`/`source` API | Film scanning needs infrared channels, 16-bit depth, multi-pass and backend-specific options; a reduced API loses them and the application has to go around the portal. Well-known option names keep the simple cases simple. |
| Typed D-Bus values, with constraints and units | The print portal's all-string settings dictionary is the mistake not to repeat: it moves validation into every application. |
| A pipe fd per frame, not a file or D-Bus bytes | Frames reach hundreds of megabytes. A pipe gives backpressure for free, needs no temporary file and no disk access in the sandbox. One frame per call mirrors `sane_start` and covers RGB plus infrared. |
| `StartFrame` returns immediately; state arrives in `FrameEvent` | `sane_start` can block for many seconds on warm-up or carriage movement, which no application should hold a D-Bus call open for. End of data on a pipe also cannot distinguish success from failure, so `finished` has to be explicit. |
| Ordered `a(sv)` for `SetOptions`, not `a{sv}` | D-Bus dictionaries do not preserve order and SANE option setting is order-dependent: `mode` must be set before options that are inactive in the wrong mode, such as `three-pass` in grayscale. A dictionary cannot express that at all. |
| A frame number on every `FrameEvent` | A client that cancels a frame and immediately starts another must be able to tell the old frame's trailing events from the new frame's. A per-session counter, and the rule that a frame ends with exactly one non-`started` event, let it discard anything not newer than the last frame it saw end. Without it every client reimplements a drain-and-wait by hand. |
| Automatic values as the vardict `{'auto': <true>}` | SANE has `SANE_ACTION_SET_AUTO` and real applications use it, for example to let the scanner pick the source. D-Bus has no empty structure, so a vardict stands in for the "no value" marker, and leaves room for further per-option modifiers. |
| `backend` in the scanner info | The scanner id is opaque, so applications cannot match device quirks on it — and device quirks are unavoidable in practice. The driver name has to be exposed on purpose. |
| `type` in the scanner info, alongside `kind` | Applications hide devices that are not scanners (webcams through `v4l`) and rank by device type. `kind`, the connection, cannot express that. SANE's own type string can, as an untranslated hint, never as an identifier. |
| `GetFrameParameters` as an estimate | SANE allows `sane_get_parameters` before `sane_start` and applications use it to size buffers and previews. Without it a client cannot know the frame size until data is already flowing. |
| A dedicated `Busy` error | A locked scanner, a frame in progress and an option set during a frame are ordinary, expected conditions that an application must handle differently from a real failure. Folding them into `Failed` forces clients to guess from the message. |
| Opaque scanner ids, never raw SANE device names | Some backends interpret device names as file paths or network addresses. Handing an application control of that string lets it steer the host at arbitrary files and hosts. |
| The application sees only granted scanners | Enumerating the LAN's scanners is a fingerprinting and privacy problem, and not something an application needs before the user has chosen anything. |
| A `Session` for an open scanner | Lifecycle comes for free: the application exiting releases the hardware lock, and the portal can revoke access with `Session.Closed`. |
| No per-scan prompt once granted | Scanning needs the user to put something in the scanner, so it cannot happen unnoticed; the grant plus revocation is proportionate. Open to argument if reviewers disagree. |
| The backend does UI only | Same split as `org.freedesktop.impl.portal.Usb`, and it keeps libsane out of the desktop shell's process as well as out of the frontend's. |

## Security and privacy

- No `/dev`, no network and no filesystem access is needed by the application; the data path
  is a pipe the portal creates.
- Device ids are opaque and not constructible, so an application cannot name a device that was
  not granted, and cannot smuggle a path or host into a backend's device string.
- Option types that would make the host open a file or socket of the application's choosing —
  file name options above all — are not listed by `GetOptions` and cannot be set.
- Grants live in the permission store, per application, and are revocable; revocation closes
  open sessions through `Session.Closed`.
- The host's exclusive lock means one application at a time has the scanner, so one sandbox
  cannot read another's scan in progress.
- Backend code runs in a helper process, not in xdg-desktop-portal, so a backend that crashes
  or hangs affects one session.

## Notes for implementers

Two things are easy to get wrong and are worth stating in the interface documentation rather
than leaving to each implementation:

1. **Cancellation must not depend on the application reading.** If the application stops
   reading, the pipe fills at 64 KB and the writer blocks. An implementation that cancels
   from the same thread will then never see `Cancel()`, and the scan hangs until the
   application reads or exits. The writer has to be non-blocking and poll for cancellation.
   The rule for the API is: *a frame is cancelled by `Cancel()` or by the application closing
   the pipe, and neither may depend on the application reading the data.*
2. **Option state can be per device, not per handle.** Some SANE backends keep option values
   on the device, so a new session can inherit the previous one's settings. This is existing
   libsane behaviour that the portal passes through rather than hides; applications should set
   every option they depend on instead of assuming defaults.

## Validation

A prototype service implements the frontend interface over libsane, with the dialog and the
permission store stubbed (auto-grant). Two applications have been ported to it, and both ran
against it from inside a Flatpak with neither `--device=all` nor `--share=network`.

| Checked | How | Result |
|---|---|---|
| Interface behaviour | 40 scenario checks on a private bus, against SANE's `test` backend | Pass: grants, opaque ids, the exclusive lock, other applications refused on someone else's session, option errors, option order, automatic values, frame numbers, `Cancel()`, closing the pipe early, release when the application leaves the bus, hand scanner (`lines` = -1), `no-documents` from an empty feeder |
| Pixel data correctness | Prototype client output compared with `scanimage` | Byte for byte identical for grayscale, 8-bit RGB, 16-bit RGB and three-pass red/green/blue |
| Portal path versus libsane | Parity test scanning the same options through both, via one trait | Frame parameters, pixel data, option descriptors and constraints identical; busy scanner, empty feeder, rejected value and cancel-with-unread-pipe all surface as the same conditions |
| Real hardware | Plustek OpticFilm 8200i film scanner, 23 checks | Pass, including colour plus infrared, two numbered frames in one session, and cancel then rescan; colour, infrared, repeat and cancel match libsane within sensor noise |
| Flatpak sandbox | Scenarios and a GTK application, no `--device=all`, no network | Pass: preview and raw scan on the 8200i, so fd passing and signals work through the Flatpak D-Bus proxy |
| A second, independent application | GNOME Simple Scan ported to the draft | Scanned from the sandbox on the 8200i, was cancelled mid-frame and scanned again in the same session; the device arrived as `film scanner` |

The second application is what produced the frame numbers, the automatic values, the `type`
field and a good deal of the interface documentation: the first application's needs alone
were not enough to settle them.

Not yet verified: a real backend dialog in a desktop shell, window-handle attachment for that
dialog, network scanners, and any provider other than SANE.

## Open questions

1. **How is the host helper launched?** Either the frontend spawns a helper process per
   session, or it talks to a separate systemd user service that owns libsane and could also
   serve eSCL later. The prototype spawns per session. This is the main structural question
   for review.
2. **Hotplug and availability** for network scanners: poll through the helper, or rely on
   Avahi?
3. **Array-valued options.** Gamma tables and some calibration data are arrays. The
   descriptor has `size` for it but v1 carries one value per option and hides arrays.
   Either v1 states that options are scalar, or the value encoding needs an array form.
4. **Is `Busy` an acceptable error name?** `org.freedesktop.portal.Error.Busy` is not in
   xdg-desktop-portal's existing error set, and the condition is real and common. Add it, or
   use another name?
5. **The option filter list.** Hiding file name and button options needs a pass over the
   actual SANE backends to be sure the list is right.
6. **One scanner, several applications.** v1 takes an exclusive lock. Is that the right
   policy, and is `Busy` enough to report it?
7. **Flatpak permissions.** Applications would need no new permission:
   `--talk-name=org.freedesktop.portal.Desktop`, which they already have, should cover it,
   and `--device=all` can then be dropped. This has not been tested against the real
   frontend.
8. **Bundled backends as a fallback.** Scanners whose driver the host does not ship — the
   8200i is one — still need the USB portal and bundled backends. Worth saying so explicitly
   in the documentation.

## Upstreaming plan

1. Open an issue on xdg-desktop-portal with this proposal and the measured results, and
   settle the open questions, question 1 first.
2. Implement the frontend in C: `src/scanner.c`, the `scanner` permission store table,
   `data/org.freedesktop.portal.Scanner.xml`, documentation and tests in `tests/`.
3. Implement the dialog in `xdg-desktop-portal-gtk`, with a KDE counterpart to follow.
4. Land the Simple Scan port once the interface is accepted, as a non-prototype consumer.

## Prior art

- sane-devel, September 2020, "Sandboxing scanner applications" (Bastien Nocera): concluded
  that a host-side service is preferable to bundled drivers. This proposal is that service,
  with a consent dialog and a permission store.
- The print portal (`org.freedesktop.portal.Print`): the consent model this follows, and the
  all-string settings dictionary it does not.
- The USB portal (`org.freedesktop.portal.Usb`): the fd handover and the frontend/backend
  split this follows.
- PWG IPP-Scan and WS-Scan: close to each other and to this option model in spirit.
  Reviewers of earlier attempts asked that the API not be locked to SANE; the v1 option
  vocabulary is typed, constrained and carries units, which an eSCL provider can fill in
  without changing the wire format.
