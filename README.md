# Negative Vibes

A GNOME application for scanning colour and black-and-white negatives and slides on
any film-capable scanner that [SANE](http://www.sane-project.org/) supports.

The scan is kept exactly as the scanner delivered it (16-bit, linear, with the infrared
plane when the scanner has one). Everything else is a stack of settings applied on top, so
you can change your mind about the look at any time and export again.

## Features

- **Scanning:** preview, per-frame scan areas, Scan All with manual-advance prompts, Scan Raw
  Only, multi-sample and multi-exposure, infrared capture, cancel at any time. The app reads
  what the scanner can do instead of assuming a model (`data/devices.toml` adds what SANE
  cannot report).
- **Rolls and frames:** settings for the whole roll with per-frame overrides, frame detection,
  resizable crop rectangles, rotate/flip, fine straighten, keep/reject, per-frame undo.
- **Negative conversion:** density-space inversion with film base, dye crosstalk and colour
  cast correction, roll analysis, and a film profile library with a profile builder.
- **Tone and colour:** levels, curves, pipettes, colour wheel, selective colour, IT8
  calibration (through ArgyllCMS), colour-to-grey.
- **Cleaning and detail:** infrared dust removal, software dust removal, brush masks, unsharp
  mask, noise reduction, local contrast, a 1:1 loupe.
- **Output:** TIFF, JPEG, PNG, DNG, HDR RAW and HDRi RAW; ICC profiles; XMP/IPTC metadata;
  naming templates; an export queue.

Help pages are in `help/` (open them with F1 in the app, or `yelp help/C`).

## Building

Requirements: Rust (stable), Meson, GTK 4, libadwaita, blueprint-compiler, lcms2, SANE
(`sane-backends-devel`). Optional at run time: ArgyllCMS for IT8 calibration.

```sh
meson setup _build
ninja -C _build
./run-devel.sh            # runs the development build from Cargo
cargo test --workspace    # headless tests for every crate
```

A Flatpak manifest for development is in `build-aux/`; `build-aux/flathub/` has the one
for release (see the notes there).

## Command line

`nv-cli` shares the scanning and processing code with the app: `scan`, `probe`, `render`
and `calibrate`. Run `cargo run -p nv-cli -- --help`.

## Layout

| Path | What |
|---|---|
| `src/` | The GTK application |
| `crates/nv-sane`, `nv-device`, `nv-acquire` | SANE binding, capabilities, acquisition |
| `crates/nv-process`, `nv-film`, `nv-export` | Pipeline, film profiles, output |
| `crates/nv-store`, `nv-calibrate`, `nv-cli` | Roll storage, IT8 calibration, CLI |
| `DESIGN.md` | Design, milestone status and hardware results |

## Contributing a film profile or a scanner

Scanner quirks go in `data/devices.toml` (the header describes each field). Film profiles are
single JSON files; build one with *Create Film Profile…* from a scan of unexposed film base
and a grey step wedge exposed on the same stock, and send the file.

## Licence

GPL-3.0-or-later.
