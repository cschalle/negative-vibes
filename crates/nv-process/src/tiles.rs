//! Full-resolution rendering in tiles (DESIGN.md §7.2): for the 1:1 view and
//! for export, where the whole picture is too big to treat as one array.
//!
//! Almost every stage is local, so a tile renders on its own. The few things
//! the pipeline measures on the whole picture (film base, density range, the
//! colour axis, a positive's range) are measured once on the proxy and fixed
//! by [`freeze`]; after that every tile, and the preview, convert with the
//! same numbers and the pieces join without seams.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};

use nv_image::FrameBuf;

use crate::params::{ImageType, ProcessParams};
use crate::dust::{DustMode, TILE_MARGIN};
use crate::{conversion, encode_output, estimate_film_base_proxy, render_toned, Orientation, Proxy};

/// Tile edge in pixels.
pub const TILE: usize = 512;

/// `params` with everything stage 4 measures on the whole picture fixed from
/// `proxy`, except what `params` already fixes (a pipette, roll analysis).
pub fn freeze(proxy: &Proxy, params: &ProcessParams) -> ProcessParams {
    let mut p = params.clone();
    if proxy.is_empty() {
        return p;
    }
    // A mark bigger than this is not dust, however it is cut into tiles.
    let full_pixels = proxy.width * proxy.height * proxy.factor * proxy.factor;
    p.dust.max_area.get_or_insert(((0.004 * full_pixels as f64) as usize).max(50));
    match p.image_type {
        ImageType::Positive => {
            if p.conversion.positive_range.is_none() {
                let (min, max) = conversion::positive_range(&proxy.pixels, &p);
                p.conversion.positive_range = Some([[min[0], max[0]], [min[1], max[1]], [min[2], max[2]]]);
            }
        }
        ImageType::ColourNegative | ImageType::BwNegative => {
            if p.conversion.film_base.is_none() {
                p.conversion.film_base = Some(estimate_film_base_proxy(proxy));
            }
            let m = conversion::freeze_negative(&proxy.pixels, proxy.width, proxy.height, &p);
            p.conversion.d_max.get_or_insert(m.d_max);
            p.conversion.cast_fit.get_or_insert(m.cast_fit);
        }
    }
    p
}

/// Render the `w × h` block at `(x0, y0)` of `raw` with frozen parameters:
/// 8-bit RGB, `w × h × 3`, clipped to the scan.
pub fn render_block(raw: &FrameBuf, frozen: &ProcessParams, x0: usize, y0: usize, w: usize, h: usize) -> Vec<u8> {
    encode_output(&tile_toned(raw, frozen, x0, y0, w, h), false)
}

/// The toned picture of one block of the scan. With dust removal on (and an
/// infrared plane in the scan), sharpening or noise reduction, the block is
/// rendered with a margin around it and trimmed, so what works on
/// neighbourhoods sees what lies beyond the block's edge.
fn tile_toned(raw: &FrameBuf, frozen: &ProcessParams, x0: usize, y0: usize, w: usize, h: usize) -> Vec<f32> {
    let dust_possible = raw.channels == 4 || frozen.dust.method == crate::dust::DustMethod::Software || !frozen.dust.include.is_empty();
    let dust = if frozen.dust.mode != DustMode::Off && dust_possible { TILE_MARGIN } else { 0 };
    let margin = dust.max(crate::detail::margin(&frozen.detail, 1));
    if margin == 0 {
        return render_toned(&Proxy::from_raw_region(raw, x0, y0, w, h), frozen);
    }
    let (sw, sh) = (raw.width as usize, raw.effective_height() as usize);
    let (mx0, my0) = (x0.saturating_sub(margin), y0.saturating_sub(margin));
    let (mx1, my1) = ((x0 + w + margin).min(sw), (y0 + h + margin).min(sh));
    let big = render_toned(&Proxy::from_raw_region(raw, mx0, my0, mx1 - mx0, my1 - my0), frozen);
    let (bw, cw, ch) = (mx1 - mx0, (x0 + w).min(sw) - x0, (y0 + h).min(sh) - y0);
    let mut out = Vec::with_capacity(cw * ch * 3);
    for y in 0..ch {
        let row = ((y0 - my0 + y) * bw + (x0 - mx0)) * 3;
        out.extend_from_slice(&big[row..row + cw * 3]);
    }
    out
}

/// The toned picture (linear light, `w × h × 3` floats) of the `w × h` block
/// at `(x0, y0)`, clipped to the scan, rendered tile by tile on `threads`
/// threads. This is what exporters encode, band by band, so memory stays at
/// one band however large the frame.
pub fn render_toned_region(raw: &FrameBuf, frozen: &ProcessParams, x0: usize, y0: usize, w: usize, h: usize, threads: usize) -> Vec<f32> {
    let (rw, rh) = (raw.width as usize, raw.effective_height() as usize);
    let (x1, y1) = ((x0 + w).min(rw), (y0 + h).min(rh));
    if x0 >= x1 || y0 >= y1 {
        return Vec::new();
    }
    let (w, h) = (x1 - x0, y1 - y0);
    let mut out = vec![0f32; w * h * 3];

    // Tiles are aligned to the scan, not to the block, so neighbouring bands
    // cut the same tiles and the result is identical however it is split.
    let (tx0, tx1) = (x0 / TILE, (x1 - 1) / TILE + 1);
    let (ty0, ty1) = (y0 / TILE, (y1 - 1) / TILE + 1);
    let n = (tx1 - tx0) * (ty1 - ty0);
    let next = AtomicUsize::new(0);
    let (send, recv) = mpsc::channel::<(usize, Vec<f32>)>();

    std::thread::scope(|scope| {
        for _ in 0..threads.clamp(1, n) {
            let (send, next) = (send.clone(), &next);
            scope.spawn(move || loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= n {
                    break;
                }
                let (tx, ty) = (tx0 + i % (tx1 - tx0), ty0 + i / (tx1 - tx0));
                let tile = tile_toned(raw, frozen, tx * TILE, ty * TILE, TILE, TILE);
                if send.send((i, tile)).is_err() {
                    break;
                }
            });
        }
        drop(send);
        for (i, tile) in recv {
            let (tx, ty) = (tx0 + i % (tx1 - tx0), ty0 + i / (tx1 - tx0));
            let (px0, py0) = (tx * TILE, ty * TILE);
            let tw = TILE.min(rw - px0);
            let (cx0, cx1) = (x0.max(px0), x1.min(px0 + tw));
            for y in y0.max(py0)..y1.min(py0 + TILE) {
                let src = ((y - py0) * tw + (cx0 - px0)) * 3;
                let dst = ((y - y0) * w + (cx0 - x0)) * 3;
                out[dst..dst + (cx1 - cx0) * 3].copy_from_slice(&tile[src..src + (cx1 - cx0) * 3]);
            }
        }
    });
    out
}

/// [`render_toned_region`] for the picture an [`Orientation`] makes of the
/// scan: the `w × h` block at `(x0, y0)` *of the picture*. Only the part of
/// the scan that block comes from is rendered, so an exporter can write a
/// rotated frame band by band as cheaply as an upright one.
pub fn render_toned_region_oriented(
    raw: &FrameBuf,
    frozen: &ProcessParams,
    orientation: Orientation,
    x0: usize,
    y0: usize,
    w: usize,
    h: usize,
    threads: usize,
) -> Vec<f32> {
    if orientation.is_identity() {
        return render_toned_region(raw, frozen, x0, y0, w, h, threads);
    }
    let (sw, sh) = (raw.width as usize, raw.effective_height() as usize);
    let (ow, oh) = orientation.output_size(sw, sh);
    let (x1, y1) = ((x0 + w).min(ow), (y0 + h).min(oh));
    if x0 >= x1 || y0 >= y1 {
        return Vec::new();
    }
    let (w, h) = (x1 - x0, y1 - y0);

    // The scan pixels the block comes from form a rectangle: the corners bound it.
    let corners = [(x0, y0), (x1 - 1, y0), (x0, y1 - 1), (x1 - 1, y1 - 1)].map(|(x, y)| orientation.source_of(x, y, sw, sh));
    let (bx0, bx1) = (corners.iter().map(|c| c.0).min().unwrap_or(0), corners.iter().map(|c| c.0).max().unwrap_or(0) + 1);
    let (by0, by1) = (corners.iter().map(|c| c.1).min().unwrap_or(0), corners.iter().map(|c| c.1).max().unwrap_or(0) + 1);
    let (bw, block) = (bx1 - bx0, render_toned_region(raw, frozen, bx0, by0, bx1 - bx0, by1 - by0, threads));

    let mut out = Vec::with_capacity(w * h * 3);
    for oy in y0..y1 {
        for ox in x0..x1 {
            let (sx, sy) = orientation.source_of(ox, oy, sw, sh);
            out.extend_from_slice(&block[((sy - by0) * bw + (sx - bx0)) * 3..][..3]);
        }
    }
    out
}

/// The whole of `raw`, rendered tile by tile on `threads` threads.
pub fn render_full(raw: &FrameBuf, frozen: &ProcessParams, threads: usize) -> Vec<u8> {
    let (w, h) = (raw.width as usize, raw.effective_height() as usize);
    let mut out = vec![0u8; w * h * 3];
    if w == 0 || h == 0 {
        return out;
    }
    let (tx_n, ty_n) = (w.div_ceil(TILE), h.div_ceil(TILE));
    let next = AtomicUsize::new(0);
    let (send, recv) = mpsc::channel::<(usize, Vec<u8>)>();

    std::thread::scope(|scope| {
        for _ in 0..threads.clamp(1, tx_n * ty_n) {
            let (send, next) = (send.clone(), &next);
            scope.spawn(move || loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= tx_n * ty_n {
                    break;
                }
                let (tx, ty) = (i % tx_n, i / tx_n);
                let tile = render_block(raw, frozen, tx * TILE, ty * TILE, TILE, TILE);
                if send.send((i, tile)).is_err() {
                    break;
                }
            });
        }
        drop(send);
        for (i, tile) in recv {
            let (x0, y0) = ((i % tx_n) * TILE, (i / tx_n) * TILE);
            let tw = TILE.min(w - x0);
            for (row, line) in tile.chunks_exact(tw * 3).enumerate() {
                let at = ((y0 + row) * w + x0) * 3;
                out[at..at + tw * 3].copy_from_slice(line);
            }
        }
    });
    out
}

/// Finished tiles kept for the 1:1 view, within a memory budget. Panning
/// back over ground already drawn costs nothing; changing the parameters
/// drops them all, since every tile depends on them.
pub struct TileCache {
    budget: usize,
    params: Option<ProcessParams>,
    tiles: HashMap<(usize, usize), Arc<Vec<u8>>>,
    /// Least recently used first.
    order: VecDeque<(usize, usize)>,
    bytes: usize,
    renders: usize,
}

impl TileCache {
    pub fn new(budget_bytes: usize) -> Self {
        Self { budget: budget_bytes, params: None, tiles: HashMap::new(), order: VecDeque::new(), bytes: 0, renders: 0 }
    }

    /// How many tiles were rendered (as opposed to found).
    pub fn renders(&self) -> usize {
        self.renders
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    fn tile(&mut self, raw: &FrameBuf, frozen: &ProcessParams, tx: usize, ty: usize) -> Arc<Vec<u8>> {
        if self.params.as_ref() != Some(frozen) {
            self.tiles.clear();
            self.order.clear();
            self.bytes = 0;
            self.params = Some(frozen.clone());
        }
        if let Some(t) = self.tiles.get(&(tx, ty)).cloned() {
            self.order.retain(|k| *k != (tx, ty));
            self.order.push_back((tx, ty));
            return t;
        }
        self.renders += 1;
        let t = Arc::new(render_block(raw, frozen, tx * TILE, ty * TILE, TILE, TILE));
        self.bytes += t.len();
        self.tiles.insert((tx, ty), t.clone());
        self.order.push_back((tx, ty));
        // Never evict the tile just made, even if one tile exceeds the budget.
        while self.bytes > self.budget && self.order.len() > 1 {
            if let Some(old) = self.order.pop_front() {
                if let Some(t) = self.tiles.remove(&old) {
                    self.bytes -= t.len();
                }
            }
        }
        t
    }

    /// The `w × h` block at `(x0, y0)`, assembled from tiles, clipped to the scan.
    pub fn region(&mut self, raw: &FrameBuf, frozen: &ProcessParams, x0: usize, y0: usize, w: usize, h: usize) -> Vec<u8> {
        let (rw, rh) = (raw.width as usize, raw.effective_height() as usize);
        let (x1, y1) = ((x0 + w).min(rw), (y0 + h).min(rh));
        if x0 >= x1 || y0 >= y1 {
            return Vec::new();
        }
        let (w, h) = (x1 - x0, y1 - y0);
        let mut out = vec![0u8; w * h * 3];
        for ty in y0 / TILE..=(y1 - 1) / TILE {
            for tx in x0 / TILE..=(x1 - 1) / TILE {
                let tile = self.tile(raw, frozen, tx, ty);
                let (tx0, ty0) = (tx * TILE, ty * TILE);
                let tw = TILE.min(rw - tx0);
                let (cx0, cx1) = (x0.max(tx0), x1.min(tx0 + tw));
                for y in y0.max(ty0)..y1.min(ty0 + TILE) {
                    let src = ((y - ty0) * tw + (cx0 - tx0)) * 3;
                    let dst = ((y - y0) * w + (cx0 - x0)) * 3;
                    out[dst..dst + (cx1 - cx0) * 3].copy_from_slice(&tile[src..src + (cx1 - cx0) * 3]);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{default_params_for, render};
    use nv_film::builtin_profiles;

    /// A synthetic negative, wider and taller than a tile so several tiles
    /// (and ragged edges) are involved: a diagonal ramp with a rebate border.
    fn scan(w: u32, h: u32) -> FrameBuf {
        let mut b = FrameBuf::new(w, Some(h), 16, 3);
        for y in 0..h {
            for x in 0..w {
                let border = x < w / 12 || x >= w - w / 12 || y < h / 12 || y >= h - h / 12;
                let t = if border { 0.0 } else { (x + y) as f32 / (w + h) as f32 };
                for (c, base) in [0.6f32, 0.3, 0.18].iter().enumerate() {
                    let d = if border { 0.0 } else { (0.1 + 1.4 * t) * (0.9 + 0.1 * c as f32) };
                    let v = (base * 10f32.powf(-d) * 65535.0) as u16;
                    b.append(&v.to_ne_bytes());
                }
            }
        }
        b
    }

    fn frozen_for(raw: &FrameBuf, profile: usize) -> ProcessParams {
        let proxy = Proxy::from_raw(raw, usize::MAX);
        freeze(&proxy, &default_params_for(&builtin_profiles()[profile]))
    }

    #[test]
    fn tiles_join_into_exactly_the_whole_picture_rendered_at_once() {
        let raw = scan(1300, 900); // 3 × 2 tiles, the last ones ragged
        let frozen = frozen_for(&raw, 0);
        let whole = render(&Proxy::from_raw(&raw, usize::MAX), &frozen);
        assert_eq!(render_full(&raw, &frozen, 3), whole);
        assert_eq!(render_full(&raw, &frozen, 1), whole);
    }

    #[test]
    fn toned_bands_equal_the_whole_picture_toned_at_once() {
        let raw = scan(1300, 900);
        let frozen = frozen_for(&raw, 0);
        let whole = render_toned(&Proxy::from_raw(&raw, usize::MAX), &frozen);
        // Bands that do not line up with tiles, as an exporter's strips will not.
        let mut joined = Vec::new();
        for (y, h) in [(0, 333), (333, 333), (666, 234)] {
            joined.extend(render_toned_region(&raw, &frozen, 0, y, 1300, h, 3));
        }
        assert_eq!(joined, whole);
        let part = render_toned_region(&raw, &frozen, 200, 100, 700, 500, 2);
        assert_eq!(part.len(), 700 * 500 * 3);
        assert_eq!(&part[..3], &whole[((100 * 1300) + 200) * 3..][..3]);
    }

    #[test]
    fn an_oriented_region_is_the_same_block_of_the_oriented_whole() {
        let raw = scan(1300, 900);
        let frozen = frozen_for(&raw, 0);
        let whole = render_toned(&Proxy::from_raw(&raw, usize::MAX), &frozen);
        for rotation in 0..4u8 {
            for (flip_h, flip_v) in [(false, false), (true, false), (false, true)] {
                let o = Orientation { rotation, flip_h, flip_v };
                let (ow, oh, upright) = o.apply(&whole, 1300, 900, 3);
                // A band across tile edges, and a block with ragged ends.
                for (x0, y0, w, h) in [(0, 0, ow, 512.min(oh)), (300, 400, 700.min(ow - 300), 300.min(oh - 400)), (0, oh - 100, ow, 100)] {
                    let got = render_toned_region_oriented(&raw, &frozen, o, x0, y0, w, h, 2);
                    let mut want = Vec::new();
                    for y in y0..y0 + h {
                        want.extend_from_slice(&upright[(y * ow + x0) * 3..(y * ow + x0 + w) * 3]);
                    }
                    assert_eq!(got, want, "{o:?} block {x0},{y0} {w}x{h}");
                }
            }
        }
    }

    /// Sharpening and noise reduction look at neighbours: with a margin, a
    /// tile gives what the whole picture gives.
    #[test]
    fn detail_filters_tile_without_seams() {
        let raw = scan(1300, 900);
        let mut frozen = frozen_for(&raw, 0);
        frozen.dust.mode = DustMode::Off;
        frozen.detail.usm = crate::detail::UsmParams { enabled: true, amount: 200.0, radius: 2.0, threshold: 0 };
        frozen.detail.noise = crate::detail::NoiseLevel::Medium;
        let whole = render_toned(&Proxy::from_raw(&raw, usize::MAX), &frozen);
        let tiled = render_toned_region(&raw, &frozen, 0, 0, 1300, 900, 3);
        let worst = whole.iter().zip(&tiled).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        assert!(worst < 1e-4, "a seam of {worst}");
        // And it changed something, or the test proves nothing.
        let mut plain = frozen.clone();
        plain.detail = Default::default();
        assert_ne!(render_toned(&Proxy::from_raw(&raw, usize::MAX), &plain), whole);
    }

    #[test]
    fn positives_tile_without_seams_too() {
        let raw = scan(1100, 600);
        let frozen = frozen_for(&raw, 2);
        assert!(frozen.conversion.positive_range.is_some());
        assert_eq!(render_full(&raw, &frozen, 2), render(&Proxy::from_raw(&raw, usize::MAX), &frozen));
    }

    #[test]
    fn without_freezing_the_tiles_would_not_agree() {
        // The reason freeze exists: each tile would measure its own range.
        let raw = scan(1300, 900);
        let loose = default_params_for(&builtin_profiles()[0]);
        let whole = render(&Proxy::from_raw(&raw, usize::MAX), &loose);
        let tiled = render_full(&raw, &loose, 2);
        assert_ne!(tiled, whole);
    }

    #[test]
    fn freezing_from_a_reduced_proxy_stays_close_to_freezing_from_the_full_scan() {
        let raw = scan(1300, 900);
        let p = default_params_for(&builtin_profiles()[0]);
        let full = freeze(&Proxy::from_raw(&raw, usize::MAX), &p);
        let small = freeze(&Proxy::from_raw(&raw, 100_000), &p);
        for c in 0..3 {
            let (a, b) = (full.conversion.d_max.unwrap()[c], small.conversion.d_max.unwrap()[c]);
            assert!((a - b).abs() < 0.08, "channel {c}: {a} vs {b}");
        }
    }

    #[test]
    fn what_the_params_already_fix_is_not_remeasured() {
        let raw = scan(600, 400);
        let mut p = default_params_for(&builtin_profiles()[0]);
        p.conversion.d_max = Some([1.0, 1.1, 1.2]);
        let f = freeze(&Proxy::from_raw(&raw, usize::MAX), &p);
        assert_eq!(f.conversion.d_max, Some([1.0, 1.1, 1.2]));
        assert!(f.conversion.film_base.is_some() && f.conversion.cast_fit.is_some());
    }

    #[test]
    fn a_region_equals_the_same_block_of_the_whole_render() {
        let raw = scan(1300, 900);
        let frozen = frozen_for(&raw, 0);
        let whole = render(&Proxy::from_raw(&raw, usize::MAX), &frozen);
        let mut cache = TileCache::new(usize::MAX);
        // Straddles four tiles and runs off the right edge.
        let (x0, y0, w, h) = (400, 300, 1000, 500);
        let block = cache.region(&raw, &frozen, x0, y0, w, h);
        let (cw, ch) = (1300 - x0, h);
        assert_eq!(block.len(), cw * ch * 3);
        for y in 0..ch {
            let a = &block[y * cw * 3..(y + 1) * cw * 3];
            let b = &whole[((y0 + y) * 1300 + x0) * 3..((y0 + y) * 1300 + 1300) * 3];
            assert_eq!(a, b, "row {y}");
        }
    }

    #[test]
    fn the_cache_reuses_tiles_and_drops_them_when_parameters_change() {
        let raw = scan(1300, 900);
        let frozen = frozen_for(&raw, 0);
        let mut cache = TileCache::new(usize::MAX);
        cache.region(&raw, &frozen, 0, 0, 600, 600);
        let first = cache.renders();
        assert_eq!(first, 4);
        cache.region(&raw, &frozen, 100, 100, 300, 300);
        assert_eq!(cache.renders(), first, "drew tiles it already had");

        let mut other = frozen.clone();
        other.tone.midtone = 1.4;
        cache.region(&raw, &other, 100, 100, 300, 300);
        assert_eq!(cache.renders(), first + 1);
        assert_eq!(cache.bytes(), TILE * TILE * 3, "old tiles were kept");
    }

    #[test]
    fn the_cache_stays_within_its_budget() {
        let raw = scan(1300, 900);
        let frozen = frozen_for(&raw, 0);
        let one = TILE * TILE * 3;
        let mut cache = TileCache::new(2 * one);
        cache.region(&raw, &frozen, 0, 0, 1300, 900);
        assert!(cache.bytes() <= 2 * one, "{}", cache.bytes());
        // The most recently used tiles are the ones kept.
        let before = cache.renders();
        cache.region(&raw, &frozen, 1024, 512, 276, 388);
        assert_eq!(cache.renders(), before);
    }
}
