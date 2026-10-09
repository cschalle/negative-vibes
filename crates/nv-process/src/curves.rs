//! Tone curves (DESIGN.md §7.1 stage 5, F§5.4): a master curve and one per
//! channel, each a set of control points joined by a monotone cubic, applied
//! in the perceptual encoding (gamma 2.2).

use serde::{Deserialize, Serialize};

/// Control points `[x, y]` in `[0, 1]`, ordered by `x`. The curve passes
/// through all of them. Fewer than two points mean the identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Curve(pub Vec<[f32; 2]>);

impl Default for Curve {
    fn default() -> Self {
        Curve(vec![[0.0, 0.0], [1.0, 1.0]])
    }
}

impl Curve {
    pub fn is_identity(&self) -> bool {
        self.0.len() < 2 || self.0.iter().all(|p| (p[0] - p[1]).abs() < 1e-6)
    }

    /// Sample the curve at `n` evenly spaced inputs from 0 to 1.
    ///
    /// Interpolation is monotone cubic Hermite (Fritsch–Carlson): it stays
    /// between its neighbouring points, so a steep edit cannot ring into
    /// values darker than black or brighter than white the way a spline can.
    pub fn lut(&self, n: usize) -> Vec<f32> {
        let pts = &self.0;
        if pts.len() < 2 {
            return (0..n).map(|i| i as f32 / (n - 1).max(1) as f32).collect();
        }
        let k = pts.len();
        let dx: Vec<f32> = (0..k - 1).map(|i| (pts[i + 1][0] - pts[i][0]).max(1e-6)).collect();
        let slope: Vec<f32> = (0..k - 1).map(|i| (pts[i + 1][1] - pts[i][1]) / dx[i]).collect();

        let mut m = vec![0.0f32; k];
        m[0] = slope[0];
        m[k - 1] = slope[k - 2];
        for i in 1..k - 1 {
            m[i] = if slope[i - 1] * slope[i] <= 0.0 { 0.0 } else { (slope[i - 1] + slope[i]) / 2.0 };
        }
        for i in 0..k - 1 {
            if slope[i].abs() < 1e-9 {
                m[i] = 0.0;
                m[i + 1] = 0.0;
                continue;
            }
            let (a, b) = (m[i] / slope[i], m[i + 1] / slope[i]);
            let s = a * a + b * b;
            if s > 9.0 {
                let t = 3.0 / s.sqrt();
                m[i] = t * a * slope[i];
                m[i + 1] = t * b * slope[i];
            }
        }

        let mut seg = 0;
        (0..n)
            .map(|j| {
                let x = j as f32 / (n - 1).max(1) as f32;
                if x <= pts[0][0] {
                    return pts[0][1].clamp(0.0, 1.0);
                }
                if x >= pts[k - 1][0] {
                    return pts[k - 1][1].clamp(0.0, 1.0);
                }
                while seg + 2 < k && x > pts[seg + 1][0] {
                    seg += 1;
                }
                let t = (x - pts[seg][0]) / dx[seg];
                let (t2, t3) = (t * t, t * t * t);
                let y = (2.0 * t3 - 3.0 * t2 + 1.0) * pts[seg][1]
                    + (t3 - 2.0 * t2 + t) * dx[seg] * m[seg]
                    + (-2.0 * t3 + 3.0 * t2) * pts[seg + 1][1]
                    + (t3 - t2) * dx[seg] * m[seg + 1];
                y.clamp(0.0, 1.0)
            })
            .collect()
    }
}

/// Master curve plus one per colour channel.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Curves {
    pub master: Curve,
    pub r: Curve,
    pub g: Curve,
    pub b: Curve,
}

impl Curves {
    pub fn is_identity(&self) -> bool {
        self.master.is_identity() && self.r.is_identity() && self.g.is_identity() && self.b.is_identity()
    }

    /// One lookup table per channel: the channel's curve, then the master.
    pub fn luts(&self, n: usize) -> [Vec<f32>; 3] {
        let master = self.master.lut(n);
        let through = |c: &Curve| -> Vec<f32> {
            c.lut(n)
                .into_iter()
                .map(|v| {
                    let f = v * (n - 1) as f32;
                    let (i, t) = (f.floor() as usize, f - f.floor());
                    let j = (i + 1).min(n - 1);
                    master[i.min(n - 1)] * (1.0 - t) + master[j] * t
                })
                .collect()
        };
        [through(&self.r), through(&self.g), through(&self.b)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_linear() {
        let lut = Curve::default().lut(5);
        for (i, v) in lut.iter().enumerate() {
            assert!((v - i as f32 / 4.0).abs() < 1e-6);
        }
        assert!(Curve::default().is_identity() && Curves::default().is_identity());
    }

    #[test]
    fn the_curve_passes_through_its_points() {
        let c = Curve(vec![[0.0, 0.0], [0.25, 0.4], [0.75, 0.7], [1.0, 1.0]]);
        let lut = c.lut(101);
        assert!((lut[25] - 0.4).abs() < 1e-5 && (lut[75] - 0.7).abs() < 1e-5, "{} {}", lut[25], lut[75]);
        assert!(!c.is_identity());
    }

    /// A steep edit between flat neighbours must not overshoot: a plain
    /// cubic spline here goes above 1 and below 0.
    #[test]
    fn monotone_data_gives_a_monotone_curve_without_overshoot() {
        let c = Curve(vec![[0.0, 0.0], [0.4, 0.05], [0.5, 0.95], [1.0, 1.0]]);
        let lut = c.lut(256);
        for w in lut.windows(2) {
            assert!(w[1] >= w[0] - 1e-6, "not monotone: {} then {}", w[0], w[1]);
        }
        assert!(lut.iter().all(|v| (0.0..=1.0).contains(v)));
    }

    #[test]
    fn a_lowered_midpoint_darkens_the_midtones() {
        let lut = Curve(vec![[0.0, 0.0], [0.5, 0.35], [1.0, 1.0]]).lut(101);
        assert!(lut[50] < 0.4 && lut[50] > 0.3, "{}", lut[50]);
    }

    #[test]
    fn channel_curve_goes_first_then_master() {
        let curves = Curves { master: Curve(vec![[0.0, 0.0], [1.0, 0.5]]), r: Curve(vec![[0.0, 0.0], [1.0, 0.8]]), ..Default::default() };
        let [r, g, _] = curves.luts(101);
        assert!((r[100] - 0.4).abs() < 1e-4, "{}", r[100]); // 1.0 -> 0.8 -> 0.4
        assert!((g[100] - 0.5).abs() < 1e-4, "{}", g[100]); // 1.0 -> 1.0 -> 0.5
    }
}
