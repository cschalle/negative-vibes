# Third-party test material

Everything else under `tests/fixtures/` is our own. These files are not.

## bw-negative-glass-with-light.dng (and `tests/goldens/bw-negative-glass-with-light.png`)

| | |
|---|---|
| Work | "[PlayRaw] Glass with light", file `28.dng` |
| Author | billznn (Massimo Bill) |
| Source | <https://discuss.pixls.us/t/playraw-glass-with-light/9392>, posted 2018-10-12 |
| Licence | Creative Commons Attribution-ShareAlike 4.0 International ([CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/)). The post says: "The files (raw, jpg and darktable sidecar file) are licensed under CC BY-SA 4.0." |
| What it is | A silver B&W negative (Ferrania P30 film), scanned on a Plustek OpticFilm 8200i with SilverFast 8.8: 2508 × 1680, 16 bit, one channel, linear, uncompressed |
| Changes | None to the `.dng`. SHA-256 `0f3edc89f0260a3e4ba66c82c5bbc3654f4b29ca3bdd21ef088fdf46abb77636`. The golden PNG is our pipeline's rendering of it, which makes it an adaptation under the same licence. |

The photograph and its golden rendering stay under CC BY-SA 4.0 whatever the licence of the code around
them (GPL-3.0-or-later): CC BY-SA 4.0 material may be combined with GPLv3 projects, and the
share-alike condition applies to the material, not to our source code. If you redistribute
the fixture, keep this notice with it.
