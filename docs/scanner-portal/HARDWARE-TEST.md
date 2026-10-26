# Running the portal prototype on the OpticFilm 8200i

Step 4 of the prototype plan: scan the same frame the direct way (libsane inside the app)
and through the portal, and check that the two agree. Everything runs from one script:

```sh
tools/portal-hw-check.sh --dry-run      # first: the SANE test backend, no hardware
tools/portal-hw-check.sh                # then: the 8200i
```

The script never detaches the scanner from the VM and never power-cycles it. It stops at
the first check with the command to run if the scanner is not usable.

## Before the first run

1. **Driver installed.** By default the script uses the installed package
   (`sudo dnf install ./build/rpm/RPMS/…/sane-backends-plustek8200i-*.rpm`, see
   `~/projects/scanner/README.rpm.md`). `--dev-backend` uses the build in
   `~/projects/scanner/backends/backend` instead (DESIGN.md §11.4, step 1; override with
   `NV_BACKEND_DIR`). The summary records which driver was tested.

   **The script uses a private SANE configuration that lists only `plustek8200i`.** With the
   system configuration, libsane lists this scanner a second time through the stock `genesys`
   backend ("OpticFilm 8200i SE"), and opening that entry reset the scanner on 2026-10-16: the
   kernel logged `device firmware changed` and a new USB address, the product strings were gone,
   and every device name resolved earlier was stale. So the script lists devices only through
   the private configuration, stops if anything else is listed, and stops if the scanner's USB
   address changes during the run. **Do not run `nv-cli probe`, the application or `scanimage`
   without `-d` against this scanner with the system configuration** until that is understood.
2. **Scanner belongs to Linux:**
   `virsh -c qemu:///system detach-device win11 ~/VirtualMachines/harness/etc/hostdev-plustek.xml --live`
   Check with `scanimage -L`. Hand it back afterwards with `attach-device`.
3. **Reference frame loaded** (the tulip negative, strip holder) and left alone for the
   whole run: every comparison is against a scan of this frame. The script asks.
4. `python3-numpy` for `tools/scan-check.py`; `python3-tifffile` is not needed here (PNM only).

Takes a while: release build of `nv-cli` and the prototype, then about eight scans at the
chosen resolution plus a 1200 dpi scan that is cancelled at 30%.

## What it does

| Step | What | Judged by |
|---|---|---|
| C0 | libsane can see the scanner; the prototype service starts on a **private** session bus | scanner listed |
| C1 | `nv-cli probe` direct and through the portal: same capabilities | identical text. A quirks entry (`plustek8200i`) that fails to match through the portal shows up here |
| C2 | Reference: `scanimage -d plustek8200i --format=pnm --mode Color --resolution N [--depth 16]` | made |
| C3 | App scan, libsane in the app (`nv-cli scan`) | completes |
| C4 | App scan through the portal (`NV_SCANNER_BACKEND=portal`), progress timed | completes; **H3**: at least three intermediate percentages |
| H1 | direct vs reference, portal vs reference, portal vs direct | same size, dy within ±0.2 rows, dx 0, ncc ≥ 0.999 (`scan-check.py align`) |
| C5 | Second portal scan (**H2**) | same thresholds against the first |
| C6 | RGB+IR direct and portal (**H5**) | colour passes H1; `iralign` dy 0 dx 0; IR median within 10% between the two |
| C8 | **H4** through the portal: cancel at 30% of 1200 dpi, then scan again on the same session | cancel returns within 2 s; the next scan completes and passes H1 |
| C9 | Service log has no timeouts or "DMA setup failed" | warning only |

Options: `--dpi N` (default 600), `--out DIR`, `--yes` (no prompt), `--skip repeat,ir,cancel`,
`--streamcheck` (sets `PLUSTEK8200I_STREAMCHECK=1`; look for "0 samples differ" in
`service.log`). Output goes to a temp directory (printed at the end): scans, `summary.txt`,
`service.log` (the driver's `SANE_DEBUG_PLUSTEK8200I=3` output for the portal scans), and the
per-check logs.

## Reading the result

- **Pass** means the portal path is as good as the direct path on this scanner. Two physical
  scans of one frame are never bit-identical, so the thresholds, not equality, decide.
- **C1 fails:** read `probe.diff`. Most likely cause: the quirks did not match, so the
  portal-side capabilities lack IR or the neutralised options.
- **Options libsane shows and the portal hides** is printed under C1. Expect the gamma and
  calibration tables (array options, which the prototype cannot carry yet). Anything else in
  that list needs a look: the app may depend on it.
- **H3 fails** with a jump from 0 to 100%: the portal path is buffering. Compare with
  `scan-check.py progress -- -d plustek8200i --format=pnm --mode Color --resolution 600`
  to see whether the driver or the portal does it.
- **H4 fails:** if `cancel_seconds` is high, the SANE read in the service is blocked in the
  driver (the service only sees the cancel flag between reads, like the app's own worker). If
  the rescan fails or is misaligned, the scanner may need a power-cycle
  (`~/projects/scanner/kasa_control.py off`, wait 5 s, `on`).

## What this does not cover

- **The GTK app itself.** Only `nv-cli` goes through the worker; nobody clicks through the UI.
- **The Flatpak sandbox.** Both paths run on the host. The sandbox is a separate step, described
  at the end of this file.
- **Real portal dialog.** The prototype auto-selects.
- **Exotic frame layouts** beyond what the 8200i produces (RGB, or RGB+IR as a second frame).

## Checked without hardware

`--dry-run` against the SANE test backend: probe, scan, portal scan (bit-identical to the
direct one), cancel and rescan. The alignment verdict was checked on synthetic images
(identical passes; two rows off and different sizes fail). The dry run cannot exercise the repeat
scan (C5), the infrared scans (C6) or the prompts, because the test backend has no infrared and
the thresholds need a real film frame.

## Result on the 8200i

On 2026-10-16 the whole check passed on the OpticFilm 8200i with the installed `plustek8200i`
0.1.25 driver: 23 passed, 0 failed, 0 warnings. The figures are in DESIGN.md §10. Two earlier
runs are why the script now has a private configuration, a time limit on each scan and a
USB-address guard: the first showed a service bug (a colour+infrared scan that never ended), and
the second was cut short when opening the `genesys` entry for the scanner reset it.

On 2026-10-26 the same 23 checks passed on the 8200i through the frontend written in C, with
the installed `plustek8200i` 0.1.28 driver: 23 passed, 0 failed, 0 warnings. That mode is
`XDP_BUILD=<xdg-desktop-portal build directory> tools/portal-hw-check.sh --frontend c`; it
starts the frontend with its helper and a stand-in for the dialog that grants the scanner
without asking, and `--dry-run` works with it too.

## From the Flatpak sandbox

`build-aux/io.github.cschalle.NegativeVibes.PortalTest.json` builds the app with `-Dportal=true`
and no `--device=all` or `--share=network`. Build and enter it from the repository root:

    flatpak-builder --force-clean --disable-rofiles-fuse build-fp-portaltest \
        build-aux/io.github.cschalle.NegativeVibes.PortalTest.json
    flatpak-builder --run build-fp-portaltest <manifest> python3 /app/share/portal-test/scenarios.py

Start the service on the host first. For the scenarios use `SANE_CONFIG_DIR` with only `test` in
`dll.conf`, `NV_SANE_ALLOW_VIRTUAL=1` and `--select test:`. For the 8200i use a directory whose
`dll.conf` lists only `plustek8200i` and `--select plustek`; never the system configuration.
`--run` accepts extra finish arguments such as `--filesystem=DIR` and `--env=NAME=VALUE`, which is
how `hw_cancel.py` and the app (`NV_AUTOSCAN=raw`, `NV_EXPORT_DIR`) were run. On 2026-10-16 the
scenarios, the cancel check and an app raw scan passed; DESIGN.md §10 has the details.
