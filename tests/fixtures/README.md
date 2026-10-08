# Test fixtures

`plustek8200i/` holds real scans from the reference scanner (OpticFilm 8200i).
Tests that read them are in `crates/nv-process/tests/`.

| File | What |
|---|---|
| `tulip-600.pnm` | Colour negative, 864 × 645, 16-bit linear RGB, 600 dpi |
| `tulip-600-ir.pgm` | Its infrared plane |
| `tulip-600-repeat.pnm` | The same frame scanned again |

## Adding a B&W negative or a slide

The B&W-negative and positive pipelines are tested on synthetic ramps only. Real
scans make those tests honest. Scan one at 600 dpi with the same settings as the
tulip and save it under these names:

```sh
nv-cli scan --dpi 600 -o tests/fixtures/plustek8200i/bw-negative-NAME.pnm   # B&W negative
nv-cli scan --dpi 600 -o tests/fixtures/plustek8200i/slide-NAME.pnm         # slide / positive
```

Then generate their golden images and look at them before committing:

```sh
NV_UPDATE_GOLDENS=1 cargo test -p nv-process --test fixtures
```

Keep each file under a few MB (600 dpi of one frame is about 3 MB). Use frames you
are happy to publish: the repository is GPL-3.0-or-later and fixtures are part of it.
