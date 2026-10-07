# Negative Scanner Application — Feature Requirements

Reference: **SilverFast 9 SE Plus 9.2.10** with a **Plustek OpticFilm 8200i**. I surveyed it
over RDP in the Windows VM on 2026-10-06 and read its on-disk preferences and profile
database in `C:\ProgramData\LaserSoft Imaging\SilverFast 9` and
`%APPDATA%\LaserSoft Imaging\SilverFast 9`.

The goal is a feature list for a new GTK application that drives our Linux driver.
Priorities:

- **P0**: must have for a usable negative-scanning app (MVP)
- **P1**: expected by serious users; build next
- **P2**: nice to have, or SilverFast-specific polish

Items marked *(observed)* come straight from the SilverFast UI or its preference
files. Items marked *(suggested)* aren't in SilverFast SE Plus but are worth having.

---

## 1. Application layout (observed)

SilverFast uses a three-column layout, which is a sensible model to follow:

| Area | Contents |
|---|---|
| Left dock | Collapsible, stackable tool panels (Scan Dimensions, Densitometer, Navigator, Picture Settings, NegaFix, Histogram, Gradation, USM, iSRD, …). Each panel header has **enable checkbox**, **reset/disable (×)**, and **undock/float** buttons; each panel footer has **help (?)**, **play tutorial (▶)**, **reset (↻)**. |
| Top toolbar | Workflow actions: Prescan, Auto, Histogram, Gradation, Global CC, Selective CC, Scan, Share. |
| Vertical tool strip | Info, Zoom (%), Rot/Flip, Pipette, USM, iSRD, SRDx, AACO, GANE, ME, PDF, PrinTao. A red dot on an icon means "this filter is active". |
| Center | Large preview with frame rectangles (draggable handles, frame number) and zoom level. |
| Status bar | Zoom %, active **Input Profile** (ICC). |
| Bottom-left | **Scanner Status** queue: thumbnail, resolution → output path, n/m progress, % bar, state ("Finished"). |

Mode buttons above the left dock: **Scan mode** (Transparency), **Image type**
(Positive/Negative/Kodachrome), **Color depth** (48→24 bit, …), **Frame** tool.

Window management *(observed, P2)*: All windows visible / dock all / expand or
collapse floating and docked windows / collect floating windows. A GTK equivalent:
`GtkPaned` and an expander-based sidebar, with floating tool windows as an optional
extra.

Two UI modes *(observed)*:
- **Expert mode**: the full panel UI above.
- **WorkflowPilot**: a guided wizard (see §12).

---

## 2. Scanner control and acquisition

| Feature | Priority | Notes |
|---|---|---|
| Device detection and selection | P0 | Via our Linux driver / SANE backend. Show model name in title ("Plustek OpticFilm 8200i"). |
| **Prescan** (overview) | P0 | Ctrl+Shift+S. Low-res scan of the whole holder window. SilverFast caches the last prescan per scanner (`Previews\PrevT(<model>).psd` + `.xml`), so reopening the app shows it immediately. |
| Auto-prescan on start | P1 | `AutoStartOverviewPrescan=TRUE`; option "Empty preview at start". |
| **Scan** (final scan) | P0 | Ctrl+S. Scans the active frame at the chosen resolution and writes the file. |
| **Batch scan** | P0 | Ctrl+Alt+S. Scans all defined frames in sequence, each with its own settings. Setting `NumberBatchScans`. |
| Cancel | P0 | Ctrl+X. |
| Scan queue / status panel | P0 | Per-job thumbnail, target path, progress, state. |
| Scan mode: Transparency | P0 | The 8200i only does transmissive. Keep the abstraction for flatbeds and reflective scanning later. |
| **Infrared channel** (for dust removal) | P0 | Needed for iSRD. "Always IR prescan 100%" option. |
| **Multi-Exposure (ME)** | P1 | Toggle in tool strip. Scans twice (normal + long exposure) and merges for more shadow density range. Preferences: "Maximum offset for ME alignment" (default 20 px), "Multi-Exposure Preview". |
| Analog gain / exposure (AG) | P1 | `AGRedAmp/AGGreenAmp/AGBlueAmp/AGBrightnessAmp`: per-channel exposure boost at the hardware level. |
| Prescan / scan draft quality | P2 | `IsPrescanDraft`, `IsScanDraft`, `HiResPrescanFactor`. |
| Three-line / shutter mode | P2 | Device-specific (`ThreeLineScan`, `UseShutterMode`). |
| Completion sound | P2 | Preferences → Special: "Process completed sound", "Play sound for …". |

## 3. Image type and colour depth

| Feature | Priority | Notes |
|---|---|---|
| Image type **Negative** | P0 | Ctrl+3. Enables the NegaFix inversion pipeline (§4). |
| Image type **Positive** (slides) | P0 | Ctrl+1. |
| Image type Kodachrome | P2 | Ctrl+2. Uses a dedicated Kodachrome input ICC profile. |
| Colour depth **48→24 bit** | P0 | Scan 16 bit/channel, apply corrections, output 8 bit/channel. |
| 16→8 bit grey | P0 | For B&W negatives. |
| 16→1 bit (line art) | P2 | Pointless for film. Skip. |
| **48-bit HDR RAW** | P0 | Linear, uncorrected 16-bit/channel output for later processing. |
| 16-bit HDR RAW (grey) | P1 | |
| **64-bit HDRi RAW** | P1 | RGB + infrared channel stored in the file, so dust removal can be done later. |
| 32-bit HDRi RAW (grey + IR) | P2 | |
| "Colour correction in grey scale" | P2 | Preferences → Special. Apply colour tools before converting to grey. |
| Colour → grey conversion presets | P1 | `SC2GSettings.xml`: Red / Orange / Yellow / Green / Blue filter, Infrared, Infrared IV. Each is `WeightRed/Green/Blue` channel weights. These behave like B&W camera filters. |

## 4. Negative conversion (NegaFix): **the core feature**

UI panel (observed):

| Control | Notes |
|---|---|
| **Vendor** dropdown | Agfa, Boots, Ferrania, Fotolabo, Fuji, Ilford, Kodak, Konica, Lucky, Mitsubishi, Polaroid, Tudor, Other, plus "<Delete…>" for user profiles. |
| **Type** dropdown | Film stock within vendor. |
| **ISO/ASA** dropdown | Film speed; also `<Standard>`, `<Linear>`, `Monochrome`. |
| **CCR** checkbox | Colour cast removal. |
| **Auto** checkbox | Automatic mask/range detection from the image. |
| **Exposure** slider | −/+ exposure compensation (default 0.0). |
| **Tolerance** slider | How aggressively auto-levels clip (default 16). |
| Expanded mode (P1) | User-editable orange-mask colour, per-channel curves, save as a custom film profile. |

### 4.1 Film profile database (observed)

Profiles are stored **per scanner model**:
`NegaFix\<ScannerFamily>\<ScannerFamily> <Vendor> <Film> <ISO>.bin`
(e.g. `PlustekFilmscanner Kodak Portra 400 NC.bin`). There are about 120 profiles for the
Plustek family. Families include Epson, EpsonV700, Nikon LS-9000/5000, Canon 8600F,
Plustek OF120/OF7600/Filmscanner, Reflecta, Minolta 5400, Microtek and more.

**The `.bin` payload is encrypted or obfuscated** after a 256-byte name header, so the
data **cannot be reused**. We need **our own profiles**, ideally measured on our own
scanner with unexposed (base) film plus a grey step wedge for each stock.

Film stocks to cover (from the Plustek list):

- **Colour negative:** Kodak Portra 160 NC/VC, 400 NC/UC/VC/BW, 800; Ektar 100; Gold
  100/200/Max 400; Ultra Color; Royal Gold; Supra; VR; Max; Farbwelt; Pro Image 100;
  Ektapress. Fuji Superia 100–1600, Reala, Pro 160/160S, Pro H 400, NPH/NPS/NPC/NPZ,
  Press, Super HQ/HG. Agfa Vista, Ultra, Optima II, HDC Plus, Portrait 160. Konica
  Centuria, VX, Impresa. Ferrania. Polaroid HD.
- **B&W / chromogenic:** Kodak T-Max 100/400, Tri-X 400, Plus-X 125, BW400CN, T400CN.
  Ilford Delta 100/400, FP4+, HP5+, Pan F+, SFX 200, XP2 Super. Agfa APX 25/100/400.
  Konica Monochrome VX 400. Tudor TPX.
- **Generic:** `(Standard)`, `(Linear)`, `Monochrome`.

Ship modern stocks too *(suggested)*: Portra 160/400/800 (current), Ektar, Gold 200,
UltraMax 400, ColorPlus 200, Fuji 200/400, Lomography, CineStill 50D/800T (remjet-free;
no orange mask on 800T variants), Harman Phoenix.

### 4.2 Conversion model (from `NegaFixSettings.xml`, observed)

The stored parameter set shows how SilverFast models a negative. It's a good blueprint
for our pipeline:

```
FilmVendor / FilmType / FilmBrand        profile selection
OrangeMask{Black,Shad,High,White}{R,G,B,A} 4 anchor points per channel describing the
                                          film base + density range (mask colour at
                                          D-min/D-max: e.g. High R=0.357 G=0.115 B=0.077)
IsNeutralizeMode, NeutralizeFactor{R,G,B} per-channel neutralisation multipliers
IsAutoMode                               auto-detect mask/range from image
GammaOut = 2.2                           output gamma after inversion
CurveValue = 0.5                         midtone curve
```

Pipeline to implement (P0):
1. Scan **linear 16-bit** RGB (+IR).
2. Estimate **film base / orange mask** colour automatically from unexposed
   border/inter-frame areas or the brightest region. Also allow manual pick (P0) with the
   pipette.
3. Divide out the mask per channel, then invert in **density (log) space**, not by
   simple `1-x`.
4. Per-channel **black/white point** (D-min/D-max) with tolerance-based clipping.
5. Per-channel gamma to align the film's dye curves (from profile), then output gamma
   (2.2).
6. Neutralise residual cast (CCR).
7. Hand off to the regular positive tools (§5).

Other requirements:
- Save **user film profiles** (P1) and share/import them (P2).
- Copy NegaFix settings to all frames of the same roll (P0; see §7).

## 5. Image correction tools

### 5.1 Auto adjust (observed)
Toolbar **Auto** (Shift+A) plus menu Edit → Auto Adjust:
Auto, **Auto CCR** (Ctrl+Shift+A), Red CCR, and scene presets **Landscape, Portrait,
Snow, Night, Evening, Gold, Technic, Skin, Document, Newspaper**, plus
**Highlight and Shadow Cast**, **Highlight Cast**, **Shadow Cast**, and
**Reset Auto** (Ctrl+Alt+A).

Auto-algorithm parameters (from `OptionsSP*.xml`, P1 as advanced prefs):
`AutoThresholdBlack/White` (0.02), `AutoTargetBlack/White` (0.02/0.98),
`AutoRangeBlack/White`, `AutoToleranceHighlight`, `AutoMidtoneFactorToLight/Shadow`,
`AutoColorCastRemoval` (strength 1.0), `AutoFrameInset` (ignore 10 % border when
measuring), `WantFullResolutionImageAutomatics`.

| Priority | Feature |
|---|---|
| P0 | Auto levels + auto colour cast removal |
| P1 | Scene presets |
| P0 | Auto measurement ignores frame border / film rebate (inset %) |

### 5.2 Picture settings (quick sliders, observed), P0
- **Midtone** (with "N" = neutral toggle)
- **Contrast**
- **Saturation** (with "A" = auto)

### 5.3 Histogram, P0 (observed)
- Input histogram, combined and per channel (grey / R / G / B toggle buttons)
- Draggable **shadow / midtone / highlight** sliders, numeric fields (0–255)
- Shadow and highlight "N" neutral toggles
- **Colour Cast Removal** checkbox + strength slider (0–100)
- Output-range slider (vertical)
- Auto button inside the panel
- Option for high-res data for histograms (`HiResDataForHistograms`)

### 5.4 Gradation curves, P0 (observed)
- Master curve plus R/G/B channel curves (channel toggle buttons)
- Grid with draggable control points; numeric input/output fields along the axis
  (0, 32, 64 … 255)
- Midtone and Contrast sliders under the curve
- Gamma for gradation: preference "Gamma Gradation" (default 2.20)

### 5.5 Global colour correction, P1 (observed)
- Colour wheel: drag the centre point to shift the overall colour balance
- Six colour handles (R, Y, G, C, B, M) on the outer ring
- Separate correction for **shadows / midtones / highlights** (buttons 25 / 50 / 75)
  plus "all"

### 5.6 Selective colour correction, P2 (observed)
- Six colour ranges: Red, Green, Blue, Cyan, Magenta, Yellow (source → target swatch)
- Hue/saturation/lightness vertical slider (H) per selected colour
- Per-colour preset dropdown ("none", …)
- **ACR** (Adaptive Colour Restoration) button + slider, which revives faded colours.
  ACR is **P1** for old/faded negatives.

### 5.7 Pipette / grey balance, P0 (observed)
Edit → Pipette: **White Point**, **Neutral Point** (Ctrl+N), **Black Point**,
**Reset Auto**. Click in the preview to set. Option: fixed midtone destination
(`MidPipFixedDestination`).

### 5.8 Densitometer, P1 (observed)
- Live readout under the cursor: R, G, B, K (and an extra luminance row), each as
  **before → after** correction
- Magnified loupe of the area under the cursor
- Sample radius (1 px, 3×3, …); units (inch, …); point mode (follow mouse or fixed
  measurement points)
- Optional per-point mini histograms (`DensitometerVisualizeHistograms`)

## 6. Image enhancement filters

| Filter | Priority | Observed controls | Notes |
|---|---|---|---|
| **USM: unsharp masking** | P0 | Presets (Auto Sharpness, …), **Power** (260), **Radius** (1.4), **Threshold** (0) | Ctrl+U. "Auto Sharpness" preference. Preview modes below. |
| **iSRD: infrared dust and scratch removal** | P0 | Mode (**Correct / Mark / Original**), **Automatic**, **Detection** (12), **Mask** (draw/erase brush, clear) | Needs the IR channel. Max IR/RGB alignment offset 20 px (prefs). Configurable mark colour. |
| **SRDx: software dust removal** (no IR) | P1 | Mode (Mark / Correct / Original), mark colour, **Defect type: Dark / Bright**, **Detection** (1), **Tile size** (3), **Mask** | Fallback for B&W silver film, where IR doesn't work. Important for B&W negatives. |
| **GANE: grain and noise elimination** | P1 | Presets: Light / Medium / Strong GANE | |
| **AACO: auto adaptive contrast optimisation** | P2 | Presets: Low / Normal / High / Max. Effect | Local tone mapping / shadow recovery. |
| Descreening | P2 | Presets: Art print, Magazine, Newspaper | Not relevant for film. Skip. |

**Filter preview modes** (observed, P0) apply to every heavy filter:
- Whole-frame (low-res) preview
- **1:1** loupe preview at actual scan resolution
- **HQ** preview (full-quality processing of a region)
- Settings popup per filter (⚙)
- Option to automatically run a 100 % prescan when a filter is enabled
  (`WantAutomaticPrescan100_*`)

**Mask** support (P1): paint include/exclude areas for iSRD/SRDx so dust removal
doesn't touch fine detail like stars or eyelashes.

## 7. Frames, multi-frame and film-strip handling

From the Frame menu and Frame tool *(observed)*:

| Feature | Priority | Shortcut |
|---|---|---|
| New frame / draw frame (rubber band in preview) | P0 | |
| **Find frame** (auto-detect frames on a strip) | P0 | Ctrl+F |
| Duplicate frame | P1 | Ctrl+D |
| Select next / previous frame | P0 | Alt+→ / Alt+← |
| Delete active / delete all | P0 | Ctrl+Backspace / Ctrl+Shift+Backspace |
| **Copy settings to all frames** | P0 | |
| Reset frame | P0 | Ctrl+Shift+R |
| Maximise active frame | P1 | Alt+A |
| Flip orientation (portrait/landscape) | P0 | Ctrl+R |
| Per-frame independent settings | P0 | Each frame keeps its own NegaFix, curves, filters, etc. |
| Frame number labels on preview | P0 | |
| Auto-frame inset | P1 | `AutoFrameInset` 10 %: shrink detected frames to avoid black borders. |
| Keep aspect ratio lock | P1 | |

*(suggested, P1)* Holder-aware frame templates: 35 mm strip (6 frames, 36×24), half-frame,
mounted slides. The 8200i film holder positions are fixed, so frame finding can use known
pitch.

## 8. Rotation, flip, zoom

| Feature | Priority | Shortcut |
|---|---|---|
| Rotate 90° CCW / CW | P0 | Ctrl+7 / Ctrl+9 |
| Flip horizontal / vertical | P0 | Ctrl+Shift+8 / Ctrl+8 (film often loaded emulsion side wrong) |
| Fine rotation / straighten | P1 *(suggested)* | |
| Zoom in/out of preview | P0 | Ctrl++ / Ctrl+- |
| Zoom to frame (magnifier) | P1 | |
| Navigator (overview thumbnail with viewport rectangle) | P1 | |
| Show masks toggle | P1 | Ctrl+M |

## 9. Resolution, size and output dimensions

Scan Dimensions panel *(observed)*:

| Control | Priority | Notes |
|---|---|---|
| **Resolution** slider + numeric ppi field | P0 | Shows resulting file size live (e.g. "2400ppi 24 MB"). |
| **Resolution presets** | P0 | Typesetter (600), Photo quality (300 / 225), Draft (150), Web (72), Custom. For film, offer the **native optical steps** of the scanner (our driver currently supports 300/600/1200/2400/3600; SilverFast also offers 7200, which our driver can't do yet, see §18). |
| **Format** presets (output size) | P1 | A5, 10×15 cm, 9×13 cm, US Legal, US Letter, 5×7″, 4×8″, 4×6″, Custom; buttons for portrait/landscape. |
| Expert settings: input W/H, zoom (scale) %, output W/H | P1 | Linked/locked aspect. |
| Units | P0 | inch, cm, mm, pica, point, pixel. |
| Named size presets | P2 | `ScanDimensionsSettings`: Very small … Very large, Presentation Full HD (1920×1080) / HD ready (1280×720) / Standard TV (720×576), Print, Web size in pixels. Resolution is computed to hit the target pixel count. |
| Lock scale / lock output X/Y | P2 | |

## 10. Output and file handling

| Feature | Priority | Notes |
|---|---|---|
| File name + **auto-incrementing** numbering | P0 | "Untitled", "Untitled (2)", … Custom separators (`SACFileSeparatorType`), optional date in name, start index. |
| Output folder chooser | P0 | Remember last directory. |
| **TIFF** (8/16-bit, LZW/ZIP) | P0 | Default format. |
| **DNG** (linear raw) | P1 *(suggested)* | Better than SilverFast's proprietary HDR "RAW" TIFFs for darktable/RawTherapee workflows. |
| JPEG (quality) | P0 | `JPEGQuality=100`. |
| PNG | P1 *(suggested)* | |
| PSD | P2 | |
| JPEG 2000 (lossless / lossy quality, ratio, size) | P2 | |
| PDF | P2 | Toolbar PDF button. |
| **Embed ICC profile** | P0 | `CMSWantEmbedProfile=TRUE`. |
| **IPTC / XMP metadata** | P1 | Byline, caption, headline, title, keywords, city, province, country, copyright, credits, source, date, time, special instructions; **saved IPTC presets** (Tools → Manage IPTC Presets). *(suggested)* Also film stock, camera, roll/frame number, scan date and scanner settings. Map to XMP/EXIF. |
| Open image after scan | P1 | "Show image after scan" with a chosen application. |
| Share | P2 | Toolbar Share button. |
| Print (PrinTao) | P2 | Skip. |
| Express scan / archive mode | P1 | `ExpressScan`, `SACIsActive` (Scan-and-Archive): fast scan to RAW straight into an archive folder for later processing. |

## 11. Colour management (CMS)

Preferences → CMS *(observed)*:
- **Colour management on/off**: P0
- **Input profile**: scanner ICC profile (here `SFprofT (OpticFilm 8200i)` for
  transparency) shown in the status bar: P0
- **Internal/working space**: Adobe RGB (1998) by default; also sRGB, ProPhoto: P0
- Output profile, grey profile, soft proof: P1
- Separate input profiles for Transmissive, Reflective, Kodachrome, File: P1
- **IT8 calibration** (scan an IT8 target, build the scanner ICC profile; `IT8UseFindFrame`): P1.
  That's a big quality win, and we can build it on ArgyllCMS (`scanin` + `colprof`).
- Use LittleCMS 2 for the pipeline *(suggested)*.

## 12. Guided mode (WorkflowPilot), P2 but useful

A step bar plus a "Start" button. Steps observed:
**Take Off → Prescan → Frames & Resolution → Auto Enhancement → Export Options →
Scan & Process → Landing**.

- **Source**: Negative / Kodachrome / Slide
- **Task**: Archive, Black and white, Color ▸, Edit ▸, Presentation, Print, Quick fix ▸,
  Repair ▸, **Simple Scan**, Web. Each task description explains what it does.
- Options: apply to all frames, fix aspect ratio, save frame settings, after-scan
  action ("Ask"), default resolution scale for transmissive (12×).

For a GTK app, an **AdwNavigationView / wizard** "Quick scan a roll" flow is the
equivalent. Make it the default for newcomers, with an expert mode toggle.

## 13. Settings, presets, undo

| Feature | Priority | Notes |
|---|---|---|
| **Undo / redo** of every adjustment | P0 | Ctrl+Z / Ctrl+Y. Undo is per-operation ("Undo Scan dimensions"). |
| **Save / load complete settings** | P0 | "…" menu: Save Settings, Load settings, Delete settings, WfP Presets. |
| Per-tool presets | P1 | Each tool (USM, GANE, NegaFix, SC2G, scan dimensions) has named presets stored as separate XML. Use one JSON/GVariant file per tool in `~/.config/<app>/presets/`. |
| Factory default reset | P0 | Per panel (↻) and global. |
| Remember last job / auto-save current job | P1 | `AutoLoadLastJob`, `AutoSaveCurrentJob`. |
| Persistent tool-dialog layout presets | P2 | Which panels are open/expanded/floating. |
| Tooltips on/off | P2 | |
| RAM limit, temp directory | P2 | "RAM limit (MB)", "Temporary files will be saved in". |

## 14. Preferences dialog (observed structure)

- **General**: units of measure, gamma gradation (2.2), HDR RAW allowed, show image after
  scan (+ app), temp folder, RAM limit, persistent tool dialog preset, empty preview at
  start, tooltips.
- **CMS**: enable, input profile, internal profile.
- **Special**: Multi-exposure preview, Always IR prescan 100 %, colour correction in
  grey scale, Include HiRePP, Auto sharpness, max offset for iSRD alignment, max offset
  for ME alignment, dock animation speed, process-completed sound, play sound for.
- **WorkflowPilot**: after-scan action.

## 15. Help and onboarding (P2)
- Context help (?) per panel, video tutorials (▶) per panel, shortcuts list,
  documentation link, Info overlay (Ctrl+I).
- *(suggested)* GNOME-style: `AdwShortcutsWindow`, Yelp help pages, inline tooltips.

---

## 16. Suggested MVP scope (P0 only)

1. Device connect, **prescan**, preview with zoom
2. Frame drawing + **Find frame** for 35 mm strips, next/prev, per-frame settings,
   copy settings to all
3. Image type **Negative / Positive / B&W negative**
4. **Negative inversion**: auto film-base detection, manual base pick, density-space
   inversion, per-channel levels, CCR, exposure/tolerance, a small set of generic
   profiles (Standard, Linear, Monochrome) plus 5–10 popular stocks measured ourselves
5. Histogram (levels) + curves + midtone/contrast/saturation + pipettes
6. **IR dust removal** (iSRD equivalent) with Correct/Mark/Off and detection strength
7. Unsharp mask (power/radius/threshold) with 1:1 preview
8. Resolution selection (native optical steps), 48→24 bit and 16-bit/HDR RAW output
9. TIFF/JPEG output, auto-numbered file names, embedded ICC (sRGB/Adobe RGB)
10. Batch scan of all frames with a progress queue
11. Rotate/flip, undo/redo, save/load settings

## 17. Implementation notes for the GTK app

- **Stack**: GTK 4 + libadwaita; GEGL or a custom 16-bit/float pipeline (babl for
  format conversion); LittleCMS 2; libtiff; exiv2 or gexiv2 for metadata.
- **Non-destructive pipeline**: keep the raw linear 16-bit + IR scan in memory or on
  disk, and treat all corrections as a parameter stack re-rendered for preview (a
  low-res proxy for interactivity, full-res on export). That gives cheap undo/redo,
  per-frame settings, and the HDRi-RAW "process later" workflow for free.
- **Separate hardware settings** (resolution, IR, ME, exposure/AG) that need a
  rescan from **software settings** that only need re-rendering. SilverFast's
  biggest usability problem is that this distinction is invisible.
- **Prescan cache** per device so the last preview appears instantly.
- **Film profile format**: an open, documented JSON format (base/mask RGB, per-channel
  D-min/D-max, per-channel gamma, neutralise factors, optional 1D LUTs), plus a
  calibration helper that builds one from a scan of unexposed film and a grey card.
- **Threading**: scanning runs in a worker thread with a GTask/GAsync API to the
  driver. The preview rerender must never block the UI.

---

## 18. Linux driver review: gaps relevant to the application

Reviewed on 2026-10-06: `~/projects/scanner/backends/backend/plustek8200i*.c`
(SANE backend `plustek8200i`), `NEXT_SESSION.md`, and the decompiled vendor driver in
`docs/decompiled_macos_driver/ScanApi.c`.

Most of §1–§17 is application territory: NegaFix, curves, USM, GANE, frames, file
output and CMS all run on the host, on raw data. The driver's job is to deliver **clean,
linear, repeatable, unmodified 16-bit RGB + IR data** for a requested area, with
**progress and cancel**. The findings below are measured against that contract.

### 18.1 What the driver already provides

| Capability | Status |
|---|---|
| Colour / Gray / **Infrared** modes (`--mode Color\|Gray\|Infrared`) | ✅ working |
| Resolutions 300 / 600 / 1200 / 2400 / 3600 dpi, square pixels, measured | ✅ working |
| 16-bit output (8-bit also offered) | ✅ |
| Correct orientation (horizontal mirror) and geometry (aspect 1.43 vs 1.44) | ✅ |
| Per-device AFE gain/offset calibration, visible and IR (`tools/calibrate_afe.py` → `~/.sane/plustek8200i-afe.conf`) | ✅, but external tool |
| IR dust removal in the driver (`--ir-clean=yes`, two passes, no registration needed) | ✅ implemented; benefit not yet demonstrated (per NEXT_SESSION) |
| `sane_cancel` stops scan + motor | ✅, but see 18.2 #4 |

### 18.2 Gaps, ranked by impact on the application

**P0: blocks or corrupts the negative workflow**

1. **Automatic "grey-world" white balance is always applied to colour scans**
   (`plustek8200i.c` ~L2509). Each channel is scaled so its mean matches the
   largest channel mean, then clipped to 65535. For negatives this is harmful:
   - It's **scene-dependent**, so every frame of a roll gets different channel gains.
     NegaFix-style conversion needs the film-base (orange mask) colour to be
     **identical across the roll**, and the app can't recover it.
   - Multipliers are ≥ 1 and the result is clipped, so **highlight data is
     destroyed**. That isn't reversible in the app.
   - It invalidates IT8 / ICC scanner profiling (§11), which needs a fixed,
     deterministic response.
   → **Fix:** make it an option (`--auto-wb`, default **off**), or delete it and leave
   colour balance to the app. Per-channel *hardware* gain (AFE calibration) is the
   right place for fixed balance.

2. **`film-type` (Negative/Positive) is a no-op.** It's parsed and logged, then ignored.
   → Either remove it, or (better) use it to select **film-type-specific AFE gain
   sets**. A colour negative's orange mask makes blue and green far denser than red,
   so negatives need different analogue gains to use the ADC range. The vendor
   plist already has separate Negative/Positive (and NegativeIR/PositiveIR) profiles.
   `calibrate_afe.py` should then calibrate one gain set per film type, ideally
   against a piece of unexposed film base. **No inversion in the driver**; that's the
   app's job.

3. **Scan area is mostly ignored.** `tl-y` shifts the feed (FEEDL), but `tl-x`,
   `br-x` and `br-y` have no effect, so the device always returns the full 1.44″
   frame. SANE frontends that set a region get a mismatch.
   → Minimum: **software crop** in the driver so the SANE contract holds. Better:
   hardware Y-range (shorter LINCNT) to save time at 2400/3600 dpi. Also offer a
   slightly **over-scanned area** (see #11) so the app can do frame detection.

4. **The whole scan happens inside `sane_start`.** All acquisition and processing
   (resample, mirror, WB, IR repair) finishes before the first `sane_read`, and
   `sane_read` only copies from RAM. Consequences for a GUI:
   - **No progress reporting** during the physical scan, which takes minutes at
     3600 dpi with IR. §2 lists a progress queue as P0.
   - **No mid-scan cancel**: `sane_cancel` only takes effect after `sane_start`
     returns. SANE allows `sane_cancel` from another thread; the acquisition loop
     needs to check a cancel flag between bulk reads, then park the carriage.
   - `sane_set_io_mode` / `sane_get_select_fd` return UNSUPPORTED, which is fine if
     the app runs SANE in a worker thread (planned in §17).
   → **Fix:** move acquisition into `sane_read` (stream lines as they arrive, which
   suits the GL845 ring buffer), or at least run it in a thread started by
   `sane_start` with a progress counter and cancel flag. Streaming also cuts peak
   memory (a 3600 dpi RGB+IR scan is ~160 MB in RAM today).

5. **No shading (flat-field) correction.** DVDSET is deliberately left off and the
   calibration read has a `TODO: Process this data for shading correction`
   (~L1464). Without per-pixel dark/white correction, CCD pixel response
   differences and lamp falloff show up as **column streaks and vignetting**.
   Negative inversion and contrast stretching amplify both. This affects every scan
   and every profile.
   → Implement **software shading correction**: capture a dark line (lamp off) and a
   white line from the shading window at the top of the travel (FEEDL≈70 already
   lands there), average N lines, then apply
   `out = (raw − dark) / (white − dark) × target` per pixel and channel before
   anything else. The IR pass needs its own reference.

**P1: needed for specific app features**

6. **Calibration lives outside the driver** (Python tool + conf file), and per
   NEXT_SESSION, *any geometry change silently invalidates it*. The app needs
   either automatic per-scan dark/white calibration (comes with #5), or a
   `calibrate` button/option exposed through SANE so the GUI can offer
   "Calibrate scanner" (SANE `SANE_NAME_CALIBRATE`-style button option).

7. **No exposure control → Multi-Exposure (ME) and analogue gain (AG) not possible.**
   Registers 0x14/0x15 (EXPB) were measured to have **no effect**. Only the AFE gain
   works. SilverFast's ME (a second, longer-exposure pass merged for shadow detail
   in dense slides and negatives) needs a real way to raise exposure: AFE gain,
   slower line period (LPERIOD) and motor speed, or lamp duty.
   → Expose **per-channel analogue gain** as SANE options (`--gain-r/g/b`, advanced
   group). Investigate a "long exposure" mode for ME. Multi-pass averaging
   (multi-sampling) is a cheap alternative: scan-to-scan noise is ~400 counts
   (NEXT_SESSION), so averaging 2–4 passes visibly helps. It works today because
   passes register to 0–1 px.

8. **IR is a separate scan, not data the app can combine.** `--ir-clean` hides the IR
   frame inside the driver, with fixed thresholds (median − 5·MAD, runs < 1/32 frame).
   The app's iSRD needs **Correct / Mark / Off, a Detection slider and masks**
   (§6), and HDRi RAW (§3) needs the **IR plane saved alongside RGB**.
   → Add a mode that returns **RGB + IR from one `sane_start`** (e.g. "Color+Infrared"
   as a second `SANE_FRAME_GRAY` after the RGB frame with `last_frame=false`, or a
   documented two-scan sequence). Move the repair algorithm into the app. Keep
   `--ir-clean` for `scanimage` users.

9. **8-bit output is just the high byte of linear data.** For negatives that leaves
   only a few dozen useful levels per channel. The app should always request 16 bit.
   → Consider dropping 8-bit, or at least documenting that it's linear. Gamma
   belongs in the app.

10. **Lamp warm-up / stabilisation isn't enforced.** `plustek8200i-lamp.c` has a
    warm-up timer, but the main scan path never uses it, and there's no
    wait-for-stable step. Cold-start lamp drift causes colour shifts between the
    first and later frames of a roll, which breaks "copy settings to all frames".
    → Add a warm-up wait (or measure white-reference stability before scanning),
    and report it to the app as status.

11. **Frame positioning / overscan.** The 8200i holder is pushed by hand to click-stops,
    so frames don't sit in exactly the same place. The app's *Find frame*, auto inset
    and crop (§7) need a small margin of holder/rebate on all four sides.
    Vertically there's ~11 % margin today; horizontally the 1.44″ line may cut
    frame edges.
    → Offer a maximum "full window" scan area (`PAGESIZE` 36.58 × 25.4 mm per plist)
    and let the app crop.

**P2: polish / later**

12. **7200 dpi doesn't work** (first bulk read fails, device drops off the bus;
    limit is between a 5184 and a 10368 px line, possibly 4 MB SDRAM). SilverFast
    offers it, but the effective resolution of this scanner is ~3600 dpi anyway, so
    it's low value. Leave it out until the ceiling is understood.
    Intermediate resolutions (e.g. 1800) can be produced by downsampling in the app.

13. **No `preview` option** (`SANE_NAME_PREVIEW`). The app can just request 300 dpi,
    but the standard option lets generic frontends (XSane, Skanlite) do quick
    prescans, for example by skipping IR and the expensive post-processing.

14. **Front-panel buttons not supported.** The 8200i has front-panel scan buttons. The
    vendor driver sets up the USB **interrupt endpoint** (`CIOUSB::InitInterrupt`)
    and polls `PollingButtonEvent`. Exposing them as SANE sensor options
    (`SANE_CAP_HARD_SELECT`, read-only) would let the app start a scan from the
    button. The endpoint's protocol is still unknown (`docs/8200i_driver_spec_review.txt`).

15. **Holder detection (verify).** The vendor driver has `HolderHomeSensorSel`,
    `LedHolderID` and `FilmHolderPosition`. If these apply to the 8200i (rather than
    only motorised-holder models like the OpticFilm 120/135i), the driver could
    report *holder inserted* and *holder type* (slide vs. strip), and the app could
    auto-select Positive/Negative.

16. **Gray mode is a fixed luminance mix in the driver.** For B&W negatives the app
    should request Color and apply its own channel weights (§3, SC2G filter presets).
    Gray mode is fine for `scanimage`.

17. **Debug env vars as the only configuration** (`PLUSTEK8200I_AFE`, `_FEEDL`,
    `_CCD_PROFILE`, …). For the app, anything a user should tune (gains, frame
    offset) belongs in SANE options in an "Advanced" group or in
    `plustek8200i.conf`. The rest can stay env-only.

### 18.3 Proposed driver ↔ application split

| Concern | Driver | Application |
|---|---|---|
| Lamp, motor, AFE gain, exposure, warm-up | ✔ | |
| Shading / dark / white calibration | ✔ | trigger + status UI |
| Geometry: resolution, area, mirror, square pixels | ✔ | frame detection, crop, rotate |
| Raw linear RGB and IR planes, 16-bit | ✔ | |
| Progress, cancel, streaming | ✔ | progress bar / queue UI |
| Film-type-specific analogue gains | ✔ | selects film type |
| White balance, negative inversion, film profiles | ✘ (remove auto WB) | ✔ |
| Dust removal from IR | optional (`scanimage`) | ✔ with UI controls |
| Multi-exposure / multi-sample merge | provides passes and exposure control | ✔ merges |
| ICC profiling (IT8) | needs deterministic raw mode | ✔ |

### 18.4 Recommended driver work order before the app MVP

1. Make auto white balance optional and default it to off (#1). This is small and has the biggest effect.
2. Progress plus mid-scan cancel, ideally by streaming in `sane_read` (#4).
3. Software shading correction with automatic dark/white reference (#5, also fixes #6).
4. Honour the scan area, with a software crop at minimum, plus a full-window overscan (#3, #11).
5. Film-type gain sets (#2).
6. RGB + IR delivered together for the app's dust removal and HDRi RAW (#8).
7. Lamp warm-up wait (#10).
8. Then gains/exposure for ME (#7), buttons (#14) and holder detection (#15).

---

## 19. Evaluation of NegPy as a starting point

Reviewed on 2026-10-06: https://github.com/marcinz606/NegPy at commit `5b6d897`
(v0.63.0), plus its Plustek driver dependency https://github.com/jboneng/pyopticfilm
(v1.3.4). I read the source and docs only; **I didn't run either**, and nothing was
tested on our scanner.

### 19.1 What NegPy is

- A full desktop app for turning film scans (scanner TIFFs, camera RAWs, DNG) into
  finished pictures, **with scanning built in**. GPL-3.0, Python ≥ 3.13, Linux, macOS
  and Windows.
- **Very active:** PR numbers are up to #1282, there were dozens of commits in the last
  week alone, and there are ~430 test files (~95k lines of tests). It's also labelled
  "Development Status: Alpha" and is driven mainly by one maintainer, so its internals
  change fast.
- UI: **PyQt6** (+ QtCharts). Rendering: the same pipeline implemented twice, on CPU
  (numpy/Numba) and GPU (24 **WGSL** compute shaders via `wgpu`: Vulkan/Metal/DX12),
  kept in numerical parity by tests.
- Heavy dependencies: numpy, numba, opencv, rawpy, tifffile/imagecodecs, onnxruntime
  (opt-in CLIP search), wgpu. Optional scanner backends: `python-sane`, `pyopticfilm`,
  `nkscan` (Nikon), `pieusb`, `gphoto2` (camera scanning).

### 19.2 ⚠ The most important finding: pyopticfilm claims to drive our chipset

**Two hardware generations share the "OpticFilm 8200i" name.** Identify them by USB ID,
never by marketing name (the "SE"/"Ai" suffixes refer to the bundled software):

| USB ID | ASIC | Linux support before this project |
|---|---|---|
| `07b3:130d` | Genesys **GL845** (older 8200i) | SANE `genesys` lineage. pyopticfilm: "probe only", scanning locked |
| **`07b3:1825`** | Genesys **GL128** (newer) | **None.** SANE genesys has no GL128 command set |

**Our unit is `07b3:1825`, the newer GL128 hardware**
(`harness/captures/plustek/lsusb-verbose.txt`: "Film Scanner(A2M)").

pyopticfilm, which NegPy uses as its "pyOpticfilm (Plustek)" backend, lists exactly
`07b3:1825` (which it calls "8200i SE") as **hardware-tested**. Its docs say the GL128
path is *"capture-derived (not a SANE port)"* and warn *"do not compare the GL128 path
to SANE genesys (there is no GL128 command set)"*. The project only started on
**2026-08-12**, so this is brand-new work done in parallel with ours, not established
Linux support. Its own validation covers only the maintainers' units. **Treat its claims
as promising but unverified on our hardware.**

According to its README, for the GL128 / `07b3:1825` path pyopticfilm already provides
most of the driver gaps in §18:

| §18 gap | pyopticfilm (per its docs) |
|---|---|
| #5 shading / flat-field correction | ✅ dark + white shading strips, ASIC DVDSET shading "aligned with SilverFast capture order", cached per resolution |
| #4 progress and cancel | ✅ "progress and cancel hooks for long scans" |
| #3 scan area | ✅ normalised `area` crop; NegPy adds a 1200 dpi full-window prescan with crop |
| #8 IR together with colour | ✅ `infrared=True` returns RGB + `ScanImage.ir` **in the same pass** |
| #7 exposure / multi-exposure | ✅ Adaptive Multi-Exposure (short + adaptive long pass, SNR-weighted merge) and manual exposure register overrides |
| — multi-sampling | ✅ Multi-Pass, 2–9 aligned repeats stacked for noise reduction |
| #10 lamp warm-up | ✅ `scanner.warmup()` (init, home, lamp on) |
| #12 7200 dpi | ✅ 150–7200 dpi claimed, with exposure-table limits noted at 7200 |
| #1 no forced auto white balance | ✅ returns 16-bit RGB numpy; "applications own post-processing" |

**Chipset mismatch in our own driver's documentation.** Our C driver's comments and
register references cite **GL846** datasheets and SANE `genesys` (`gl846.cpp`), but the
hardware is GL128. GL128 evidently shares enough with the GL84x family that our
capture-driven approach works. Still, where we've reasoned from GL846 docs, results may
not hold. That would fit several surprises in `NEXT_SESSION.md`: EXPB (0x14/0x15)
having no effect, 7200 dpi failing, and "REG06/SCANMOD = 0xF0" being the missing piece.
pyopticfilm's `docs/register-reference.md` (a confidence-tagged GL128 register catalog)
and `docs/gl128-model-comparison.md` are the best GL128 references found so far. They're
worth cross-checking against our register findings, in both directions, since our
work could help them too.

**Recommendation:** before investing further in the §18 driver work order, spend an
afternoon running pyopticfilm against the scanner. It needs the device detached from the
VM and a udev rule, and it's userspace libusb, so it needs no root and no kernel driver.
Compare output quality with our SANE driver and with SilverFast. If pyopticfilm holds
up, the cheapest path is:

- the GTK app uses **pyopticfilm directly** for the GL128 8200i (as NegPy does), and
- our C SANE backend becomes the route for **generic SANE frontends**
  (XSane/Skanlite/scanimage). It can use pyopticfilm's documented GL128
  shading/exposure behaviour to close its own §18 gaps faster. **Mind the licences,
  though:** pyopticfilm is GPL-3.0-or-later, while our backend is GPL-2.0-or-later like
  the rest of SANE, so code can't simply be copied across. See §20.4.

NegPy also has a **SANE backend** (`infrastructure/scanners/sane_backend.py`), so our
existing `plustek8200i` driver would work in NegPy today. The §18 issues (forced auto
WB, scan-in-`sane_start`) would show up there exactly as described.

### 19.3 Is it a good starting point for a GTK4 app?

**Yes for the engine, no for the UI.** The code is cleanly layered:

| Package | Lines | Qt-dependent |
|---|---|---|
| `negpy/features/` (per-feature `models.py` / `logic.py` / `processor.py` / `.wgsl`) | ~19k | 0 files |
| `negpy/services/` (rendering engine, scanning, export, assets/rolls DB) | ~17k | 0 files |
| `negpy/infrastructure/` (loaders, scanners, GPU, storage) | ~12k | 1 file (monitor ICC detection) |
| `negpy/kernel/`, `negpy/domain/` (config dataclasses, migrations) | ~3.5k | 0 files |
| **`negpy/desktop/`** (PyQt6 views, sidebars, workers, controller) | **~58k** | 117 / 141 files |

So about **52k lines of Qt-free core** could sit underneath a GTK4/libadwaita frontend
written in **PyGObject**. The core's entry points are `WorkspaceConfig` (one frozen
dataclass per edit), `DarkroomEngine.process()` (CPU) / `GPUEngine` (WGSL) behind
`ImageProcessor`, the scanner backends in `infrastructure/scanners/`, and the export
service. `wgpu` renders off-screen, so the result can go into a `GtkPicture` via
`GdkMemoryTexture` (or a `GdkDmabufTexture` later), with no Qt involved.

Options, in order of preference:

1. **GTK4 frontend over NegPy's core, developed with upstream.** Propose to the
   maintainer that the core become an importable package (`negpy-core`), and build a
   GNOME/libadwaita app on it. This avoids rewriting ~52k lines of hard-won
   image science, and we inherit fixes. The risk is the core's churn: a fast-moving
   alpha codebase with no stable API. Without upstream buy-in, keeping a separate
   frontend in sync is costly.
2. **Fork and replace `negpy/desktop/`.** Full control, but we take on maintenance of
   the engine and lose upstream velocity. The 58k-line UI layer is also bigger than the
   core, so "just swap the UI" is still a large project. Expect to replicate much of
   `AppController`/`AppState` (sessions, rolls, undo, workers → `GTask`/threads).
3. **Use NegPy as is, and spend the effort on the driver.** If the goal is "a good
   Linux negative-scanning workflow" rather than specifically a GTK app, NegPy plus
   pyopticfilm (or our SANE driver) may already deliver it on Linux today.

Packaging note: a Flatpak with numba, opencv, onnxruntime and wgpu is feasible but large.
NegPy ships an AppImage today.

### 19.4 Coverage of our feature list (§1–§17)

✅ = present, ◐ = partial or different approach, ✘ = not found. Based on its docs and
source tree; not exercised.

| Our section | NegPy coverage |
|---|---|
| §2 Scanner control | ✅ prescan (Plustek: 1200 dpi full-window prescan + crop), scan, Stop/cancel, IR, **multi-exposure**, **multi-pass**, strip preview + frame picking (feeders), output folder becomes a roll. ◐ No user-facing analogue gain (exposure overrides are debug-only). |
| §3 Image types | ✅ Colour negative, B&W negative, Slide, Kodachrome. 8/16-bit, `TIFF (mono)` for B&W. ✅ linear export / flat 16-bit master ≈ HDR RAW. ✅ reads RGB+IR 4-channel TIFF/DNG, SilverFast iSRD TIFFs, HDRi RAW DNGs. ✘ SilverFast-style colour→grey filter presets (not found). |
| §4 Negative conversion | ✅ **Different and arguably better than NegaFix**: no film-stock profiles; the orange mask is removed from each channel's own sensitometry, and the print is built in density space on an H&D curve (toe/straight/shoulder) graded in ISO-R like darkroom paper. Per-film **crosstalk matrices** (dye unmixing, editable `.toml`), Cast Removal, Auto Density / Auto Grade. **Roll Analysis** gives every frame of a roll the same baseline, which is exactly the consistency problem in §4 and §18 #1. ✘ No SilverFast-style vendor/stock/ISO profile picker. |
| §5 Corrections | ✅ histograms (lin/log, clipping), photometric curve, white balance (filtration), density/contrast/toe/shoulder, R/G/B white/black point trims, spot densitometer/probe, ring-around, step wedge, zone overlay. ◐ Tone control is the print-curve model, not free-form point curves. ✘ Selective colour correction (six-colour) not found. ◐ Global colour shown as filtration + hue trim rather than a colour wheel. |
| §6 Filters | ✅ IR dust removal (**two methods: NegPy and OpenICE**), optical dust/hair detection without IR (≈ SRDx), manual heal, scratch, transport-line repair, clone, exclusion masks. ✅ Sharpening: USM **or Richardson-Lucy deconvolution**, edge masking. ✅ CLAHE local contrast (≈ AACO). ◐ Chroma denoise only; no luminance grain reduction (≈ GANE). |
| §7 Frames | ✅ auto-crop per frame and per roll, film-edge vs image-only, rebate trim, aspect ratios, half-frame split, stitching, per-frame vs roll-wide settings ("scope pair"), copy-to-roll. |
| §8 Rotate/flip/zoom | ✅ crop and straighten, auto-skew, tilt/swing (perspective). |
| §9 Resolution / size | ✅ dpi from list or typed. ✅ export size presets. |
| §10 Output | ✅ JPEG, TIFF, PNG, WebP, JPEG XL. Jinja2 filename templates, export presets, contact sheets, borders/mats. EXIF/XMP with camera, lens, film, development, date/place. ◐ DNG input yes; DNG output not confirmed. |
| §11 CMS | ✅ ICC, monitor profile detection, soft proofing with paper/printer profiles, Input ICC override. ✘ IT8 target → scanner ICC profile builder (not found). |
| §12 Guided mode | ◐ first-launch tour + "Scanning setup" wizard, but no step-by-step scan workflow like WorkflowPilot. |
| §13 Undo / presets | ✅ non-destructive, SQLite edits keyed by content hash (survive rename/move), `.negpy` sidecars, persistent undo history, named "work prints" (snapshots), presets, roll defaults. |
| §14–15 Prefs / help | ✅ per-panel ⓘ help from the user guide, remappable shortcuts, performance prefs, `override.toml` recovery. |

**Missing in NegPy compared with our list:** selective colour correction, luminance
grain/noise reduction (GANE), IT8 scanner calibration, free-form point curves,
colour→grey filter presets, and a guided scan wizard. These are the obvious things a
GTK frontend (or upstream contributions) would add.

### 19.5 NegPy features not in SilverFast, worth exposing in our GUI

Sorted by value for a negative-scanning workflow:

1. **Roll-level workflow**: library of rolls; roll-wide defaults with per-frame
   overrides (frame/roll scope buttons on every panel); **Roll Analysis** for one
   consistent baseline per roll; **Scenes** (sub-groups with their own baseline);
   keep/reject triage with badges. SilverFast works frame by frame.
2. **Darkroom print model**: density-space H&D curve, ISO-R paper grades,
   **paper profiles** (Ilford, Kodak, Foma, Fuji datasheets), print density.
   Familiar to analogue photographers and more physically meaningful than
   levels/curves.
3. **Crosstalk / dye unmixing matrices** per film and process, user-editable.
4. **Darkroom tools**: **test strips** (exposure series overlay), **colour
   ring-around**, **step wedge**, **zone overlay + zone placement**, negative
   statistics.
5. **Dodge and burn** with polygon/oval/card-edge masks, each with strength,
   feather, grade and tone limit; **contrast mask**; **edge burn**; **tilt and swing**.
6. **Better dust tools**: OpenICE method, optical hair detection, **transport-line
   scratch repair** (click once to trace a full-length scratch), grain-preserving heal.
7. **Multi-Pass** scanning (stacked repeats for noise) alongside multi-exposure.
8. **Half-frame splitting, stitching** (multi-shot scans of large formats) and
   **HDR merge** of bracketed exposures.
9. **Deconvolution sharpening** (Richardson-Lucy) to undo scanner lens blur.
10. **Lab-scanner look** (Frontier/Noritsu-style): chroma, skin protection, chroma
    denoise, **halation** and **glow** film effects.
11. **B&W alternative processes**: lith and cyanotype simulation, toners.
12. **Peek negative / before-after / reference view** to compare against the raw
    negative or another frame.
13. **Search**: by metadata, and opt-in **semantic search** (local CLIP model).
14. **Gear catalog**: cameras, lenses, film stocks, development, auto-matched from
    folder names and written to EXIF/XMP.
15. **Camera scanning**: tethered DSLR/mirrorless "scanning" via gphoto2 with live
    view, focus peaking, RGB narrowband light (Scanlight) and **trichrome merge**.
    Out of scope for the 8200i, but cheap to keep if we build on NegPy's core.
16. **Work prints** (named snapshots of an edit) and **content-hash keyed edits**
    that survive moving or renaming files.
17. **Finishing**: filed-carrier borders, print mats, contact sheets.

### 19.6 Revised recommendation

1. **Test pyopticfilm on our 8200i (`07b3:1825`) first.** It may make most of §18
   unnecessary for the app, and its GL128 sequences could unblock our SANE driver.
2. **Build the GTK4/libadwaita app on NegPy's Qt-free core** (PyGObject + `wgpu`
   off-screen rendering into `GtkPicture`). Open the conversation with the NegPy
   maintainer about splitting out a core package before writing frontend code.
3. Treat §4 (NegaFix-style profiles) as **superseded** by NegPy's sensitometric model
   plus Roll Analysis. Keep §5.6 selective colour, GANE-style grain reduction, IT8
   calibration and a WorkflowPilot-style guided scan as **our additions**.
4. Our app's licence will need to be **GPL-3.0** (both NegPy and pyopticfilm are GPL-3).

---

## 20. Using pyopticfilm to close the gaps in our SANE driver

Short answer: **yes, it's the most useful outside reference this project has had.**
It's an independent, working implementation for the **same GL128 chip and the same USB
ID (`07b3:1825`)**, reconstructed from **Windows/SilverFast USB captures**. Our
driver came from the **decompiled macOS driver** plus our own Windows captures. The two
efforts are complementary, and where they agree we can treat a finding as solid.

Reviewed from source (`pyopticfilm` v1.3.4, `src/pyopticfilm/asic/gl128.py`,
`scan/session_gl128.py`, `scan/calib_gl128.py`, `device/gl128_common.py`,
`device/model_8200i.py`, `docs/register-reference.md`). Not yet run on our hardware.

### 20.1 Our driver's register map is GL846, the hardware is GL128

`plustek8200i.h` defines registers after the GL846 datasheet and SANE `gl846.cpp`.
pyopticfilm's `docs/register-reference.md` is a GL128 catalog with a confidence level
per entry (capture-confirmed / inherited / suspected / unknown). Where they differ, our
driver's open problems line up with the GL128 map:

| Function | Our header (GL846) | pyopticfilm GL128 (capture-confirmed) | Consequence for us |
|---|---|---|---|
| Exposure | `EXPR/EXPG/EXPB` 0x10–0x15 | **`REG_EXPOSURE` 0x7D–0x7F** (24-bit, baseline 14000) + **per-channel exposure tables in AHB RAM** at 0x10000000 / 0x10004000 / 0x10008000 (R/G/B) | **Explains "EXPB does nothing"** (`NEXT_SESSION.md`, 2026-10-06). We were writing the wrong registers. Real exposure control (§18 #7, multi-exposure) is reachable. |
| Pixel window (X crop) | `STRPIXEL`/`ENDPIXEL` 0x30–0x33 | **0x82–0x84 / 0x85–0x87**, 24-bit, **native 7200 dpi units**, independent of resolution | **Explains §18 #3** (X scan area ignored). The GL846-style writer in `plustek8200i-scan.c` (`plustek8200i_set_scan_parameters`) is dead code anyway. |
| Line period | `LPERIOD` 0x38–0x39 | **0x28–0x2A**, 24-bit, per-dpi table (`LPERIOD_BY_DPI`) | We never set it intentionally. Needed for exposure/ME and 7200 dpi. |
| Bit depth | (0x33 = `ENDPIXEL_L`) | **0x33 `DEPTH_A` + 0xAF `DEPTH_B`** (0x04/0x46 = 16-bit) | Don't write 0x33 as a pixel register. |
| 0x35–0x37 | "MAXWD" 24-bit, fixed 0x2F04C0 | 0x35/0x36 opaque; **0x37 = `REG_IR`, bit 2 = IR LED** | Harmless today (0xC0 has bit 2 clear and it's a replay of the macOS value), but **fragile**. If someone "fixes" MAXWD, the IR LED could switch on during colour scans. Rename it and stop treating it as a line width. |
| Status | 0x40/0x41 | **0x101** (same bit layout as GL845 0x41) | Check our status polling reads the register that GL128 updates. |
| AFE access | — | **0x51 (sub-address) + 0x5D/0x5E (16-bit value)**; GL845 uses 0x3A/0x3B | ✅ our `macos-init` already uses 0x51/0x5D/0x5E. Agreement. |
| Shading table | DVDSET mentioned, not implemented | **DVDSET = bit 0x20 of 0x01**, table uploaded to **AHB 0x10014000**; slope tables at 0x1000C000 / 0x10010000 | ✅ our motor-table DMA addresses match. The shading upload is the missing piece (§18 #5). |

**Independent confirmations of our own findings** (good news, worth recording in
`NEXT_SESSION.md`):
- **0x06 = 0xF0**: pyopticfilm's 1200 dpi setup golden has exactly this. That was our
  "REG06/SCANMOD = 0xF0" breakthrough.
- **DPISET = dpi/6** at ≥ 600 dpi. Same in both. pyopticfilm programs < 600 dpi as 600
  and downsamples on the host.
- **Vertical oversampling** (our "~2.2× vertical sampling, area-average resample"):
  pyopticfilm models it as `oversample = optical_resolution / asic_dpi`.
- **IR**: 0x37 bit 2 = IR LED; 0x03 `LAMPPWR`=0x10 off and `XPASEL`=0x20 kept for IR
  passes. That matches our IR implementation exactly.

### 20.2 Gap-by-gap: where to look in pyopticfilm

| §18 gap | pyopticfilm reference | What to take from it |
|---|---|---|
| #5 shading correction | `scan/calib_gl128.py` (1333 lines), `asic/gl128.py` shading-strip code (~L760–1060) | Dark strip (lamp off) and white strip (lamp on, `DVDSET\|MTRPWR\|AGOHOME`), the shading-table format and upload to `AHB_SHADING`, per-dpi dummy clocks (`SHADING_DARK_DUMMY_BY_DPI`), and the "median gain 1.02–3.0×" sanity band. Also the gotcha that IR passes must **clear DVDSET** (it clipped IR to full scale), and the cache key per resolution (`~/.cache/pyopticfilm/calib_v2.json`). |
| #6 calibration inside the driver | same + AFE search (`search_afe` / zero-collapse recovery) | AFE offset/gain search done at scan time, cached per dpi, replacing `tools/calibrate_afe.py` + the conf file, which goes stale with geometry changes. |
| #7 exposure / ME / AG | `scan/session_gl128.py` (`clamp_me_long`), `scan/exposure_merge.py`, `device/gl128_common.py` (`exposure_short/long`, `me_long_clamp_max`) | Write `REG_EXPOSURE` + per-channel AHB exposure tables. Limits: 14k–85k, 16-bit channel words at 7200 dpi (64k cap). Merge maths for short + long passes. Multi-pass stacking (`pass_align.py`). |
| #3 / #11 scan area, overscan | `scan/geometry.py`, `asic/gl128.py` STR/END handling, `scan/bringup.py` (`default_frame_crop_norm`, `PRESCAN_DPI`) | STRPIXEL/ENDPIXEL in native 7200 dpi clocks; the 96-clock `optical_end_inactive_native` shift; the ~24-column USB dummy suffix to drop; `feed_to_scan_steps` = 13128 as the true top of the scan window (CHANGELOG: 13704 overran it). |
| #12 7200 dpi | `gl128_common.py` per-dpi tables, `max_image_lincnt_by_feed2`, register 0x1D notes | Oversample = 1 at 7200; LPERIOD and dummy-clock values; 0x1D flips during 3600/7200 shading and the 7200 image pass; line-aligned "quiet USB drain" paced to LPERIOD (`DEFAULT_IMAGE_USB_PACE_S`). Our 7200 attempt dropped the device off the bus after the first bulk read, plausibly a drain/pacing or LINCNT/0x1D issue. |
| #4 progress and cancel | `asic/gl128.py` / `scanner.py` cancel path; first commit "Fix GL128 mid-scan cancel wedging USB by completing bulk announces before abort" | **Cancel mid-scan wedges USB unless pending bulk transfers are completed first.** Also the capture-confirmed cancel-during-preview park recipe (lamp-strobe sequence on 0x03, then 0x01=0x22). |
| #10 lamp warm-up | `Scanner.warmup()`, `_strike_lamp_on` | The lamp-on sequence (`AVEENB`=0x40 held during lamp-on). |
| parking / homing safety | `docs/register-reference.md` behavioural notes | **GL128 has no capture-proven standalone home seek**: the carriage only returns via `AGOHOME` on the image pass. "A standalone home-seek recipe previously caused grinding when invented." Also a motor-ramp selection that caused a real mechanical fault on the 8100 V2. Check our motor code against both before experimenting. |
| #14 buttons, #15 holder | — | Not covered by pyopticfilm. Still open. |

### 20.3 Suggested method

1. **Offline diff first (no hardware risk).** pyopticfilm ships GL128 register
   "goldens" for 1200 / 1800 / 7200 dpi
   (`tests/traces/python/8200i_se/*_rgb16_setup.json`, init + configure). Dump our
   driver's register state at scan start (we already have `PLUSTEK8200I_DUMP_RAW` and
   `tools/recon/regdiff.py`) and diff against them. Every difference is either a bug or
   a documented model divergence.
2. **Live differential capture.** Run pyopticfilm on the scanner (detach from the VM;
   udev rule; `pip install pyopticfilm`), capture with usbmon exactly as for the Linux
   and Windows runs in `harness/captures/plustek/`, and diff against our driver and
   SilverFast with `regdiff.py`. That gives three implementations on one device,
   including one that already does shading, exposure and 7200 dpi.
3. **Use its Scan Lab** (`tools/scanlab/`, a Qt app with a forensic USB timeline,
   register catalog and run comparison) for side-by-side comparison of scans.
4. **Port behaviour, not code** (see 20.4), one gap at a time, in the §18.4 order:
   auto-WB off → shading → exposure → area → progress/cancel → 7200.

### 20.4 Licence: read it, don't copy it

- pyopticfilm: **GPL-3.0-or-later**. Our backend: **GPL-2.0-or-later** (the SANE
  header), like upstream sane-backends.
- Copying pyopticfilm code (or close translations of it) into `plustek8200i.c` would
  make that file GPL-3-only in effect. That's incompatible with keeping the backend
  GPL-2+, and would likely block upstreaming to SANE.
- **What is fine:** using it as documentation: register addresses and meanings,
  sequence order, timing constraints, and the behavioural lessons above, then writing
  our own C implementation. Captured register values and tables are hardware facts
  observed on the wire, and our project already derives the same kind of data from its
  own captures. I'm not a lawyer; if large tables are taken verbatim, ask first.
- **Best option: talk to the maintainer (`jboneng`).** Ask whether the GL128 parts can
  be dual-licensed GPL-2.0-or-later for a SANE backend, and offer our macOS-decompile
  findings in return. Their docs list several GL128 registers as "unknown" or
  "suspected", and our `docs/decompiled_macos_driver/ScanApi.c` analysis may answer
  some of them. A joint GL128 SANE backend would help both projects and every 8200i
  owner on Linux.

---

## 21. Driver status after applying the GL128 lessons (2026-10-06)

Branch `gl128-lessons` in `~/projects/scanner` (8 commits; details in
`NEXT_SESSION.md`). Lessons came from pyopticfilm, but no code was taken from it.
Every change was verified with test scans of the tulip frame, and a regression sweep
covered all resolutions.

| §18 gap | Status |
|---|---|
| #1 forced auto white balance | ✅ `--auto-wb`, default off; highlight clipping gone |
| #2 film-type no-op | ✅ removed: the SilverFast recordings show it has no hardware meaning |
| #3 scan area | ✅ exact software crop of the full window; correct pre-scan size estimate |
| #4 progress / cancel | ✅ lines stream from `sane_read` while the carriage moves (real progress in frontends); cancel works mid-scan |
| #5 shading correction | ✅ hardware shading (DVDSET): ~19% edge falloff → flat to 0.3% |
| #6 calibration in driver | ✅ per-scan, ~2 s, stationary: AFE gains set against the open light (90% FS, SilverFast's reference), frames repeat to 0.1%; works after power cycles since the driver sets SilverFast's lamp level |
| #7 exposure / ME / AG | ✅ `--exposure 1.0–4.0`, linear and geometry-preserving, so multi-exposure is now possible in the app |
| #8 IR + colour | ✅ IR detection flat-fielded; IR level normalised per scan; mode **Color+Infrared** delivers the IR plane as a second, pixel-aligned SANE frame (for dust removal later or an HDRi-RAW-style file) |
| #10 lamp warm-up | ✅ each scan waits until two white strips agree to 0.2% |
| scan time | ✅ 38.7 s → ~20–25 s per scan: the carriage now returns home on its own, as with SilverFast |
| #12 7200 dpi | ✅ works with SilverFast's 7200 line timing. A fast positioning feed (SilverFast's method) replaced the 57 s scan-speed lead-in: 51 s per 7200 dpi scan, 29 s at 3600, 16.5 s at 600 |

**Found and fixed along the way** (not in §18):
- **Lamp level:** SilverFast sets the lamp and IR LED drive with two writes on a
  vendor path the driver never used. Without them the power-on lamp is ~25% brighter,
  and that differed depending on whether SilverFast had run. The driver now sends them.
- **3600 dpi was 1.2 s from failing:** the first read waited out a 29 s lead-in feed
  against a 30 s timeout. Now 180 s.
- **Frame position was not repeatable.** It landed 11 rows apart depending on a
  timing race in the homing move; now consistent across back-to-back and idle starts.
- **Colour channels were misregistered by dpi/300 rows** (12 px at 3600 dpi). This
  caused colour fringes and smeared IR specks into triples; now aligned to <0.2 px.
