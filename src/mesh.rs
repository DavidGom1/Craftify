//! Triangle meshes for checking a converted part against the source CAD's own display mesh.
//!
//! The source mesh is the ground truth for a conversion: its volume and box say whether the
//! result matches, and the distance from a point to it says which side of a circle an arc of
//! the source runs along. Port of `mesh.py` (the parts the converter uses).

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::step::{V, add, cross, dot, mul, sub};

/// Closest point to p on triangle abc (Ericson, Real-Time Collision Detection 5.1.5).
fn closest_on_triangle(p: V, a: V, b: V, c: V) -> V {
    let (ab, ac, ap) = (sub(b, a), sub(c, a), sub(p, a));
    let (d1, d2) = (dot(ab, ap), dot(ac, ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = sub(p, b);
    let (d3, d4) = (dot(ab, bp), dot(ac, bp));
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return add(a, mul(ab, d1 / (d1 - d3)));
    }
    let cp = sub(p, c);
    let (d5, d6) = (dot(ab, cp), dot(ac, cp));
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return add(a, mul(ac, d2 / (d2 - d6)));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        return add(b, mul(sub(c, b), (d4 - d3) / ((d4 - d3) + (d5 - d6))));
    }
    let denom = 1.0 / (va + vb + vc);
    add(a, add(mul(ab, vb * denom), mul(ac, vc * denom)))
}

const CELL: f64 = 5.0;

type Grid = HashMap<(i64, i64, i64), Vec<usize>>;

/// Triangles wound counter-clockwise seen from outside (outward normals).
#[derive(Default)]
pub struct Mesh {
    pub triangles: Vec<[V; 3]>,
    grid: OnceLock<Grid>,
}

impl Clone for Mesh {
    fn clone(&self) -> Self {
        Mesh { triangles: self.triangles.clone(), grid: OnceLock::new() }
    }
}

fn cell(x: f64) -> i64 {
    (x / CELL).floor() as i64
}

impl Mesh {
    pub fn add(&mut self, a: V, mut b: V, mut c: V, outward: Option<V>) {
        let g = cross(sub(b, a), sub(c, a));
        if dot(g, g) < 1e-24 {
            return; // degenerate
        }
        if let Some(o) = outward
            && dot(g, o) < 0.0 {
                std::mem::swap(&mut b, &mut c);
            }
        self.triangles.push([a, b, c]);
        self.grid = OnceLock::new();
    }

    /// The mesh with every point multiplied by the 3x3 matrix m (row-major, column vectors).
    pub fn transformed(&self, m: &[f64; 9]) -> Mesh {
        let t = |p: V| {
            [m[0] * p[0] + m[1] * p[1] + m[2] * p[2], m[3] * p[0] + m[4] * p[1] + m[5] * p[2], m[6] * p[0] + m[7] * p[1] + m[8] * p[2]]
        };
        Mesh { triangles: self.triangles.iter().map(|[a, b, c]| [t(*a), t(*b), t(*c)]).collect(), grid: OnceLock::new() }
    }

    pub fn volume(&self) -> f64 {
        crate::util::py_sum(self.triangles.iter().map(|[a, b, c]| dot(*a, cross(*b, *c)))) / 6.0
    }

    /// (min, max); None for an empty mesh (Python raises there).
    pub fn bbox(&self) -> Option<(V, V)> {
        let mut pts = self.triangles.iter().flatten();
        let first = *pts.next()?;
        let (mut lo, mut hi) = (first, first);
        for p in pts {
            for k in 0..3 {
                if p[k] < lo[k] {
                    lo[k] = p[k];
                }
                if p[k] > hi[k] {
                    hi[k] = p[k];
                }
            }
        }
        Some((lo, hi))
    }

    fn grid(&self) -> &Grid {
        self.grid.get_or_init(|| {
            let mut grid: Grid = HashMap::new();
            for (i, [a, b, c]) in self.triangles.iter().enumerate() {
                let lo: Vec<i64> = (0..3).map(|k| cell(a[k].min(b[k]).min(c[k]))).collect();
                let hi: Vec<i64> = (0..3).map(|k| cell(a[k].max(b[k]).max(c[k]))).collect();
                for x in lo[0]..=hi[0] {
                    for y in lo[1]..=hi[1] {
                        for z in lo[2]..=hi[2] {
                            grid.entry((x, y, z)).or_default().push(i);
                        }
                    }
                }
            }
            grid
        })
    }

    /// Distance from p to the surface, positive outside, negative inside.
    ///
    /// Only triangles within `reach` are looked at; None when there is none.
    pub fn signed_distance(&self, p: V, reach: f64) -> Option<f64> {
        let grid = self.grid();
        let lo: Vec<i64> = (0..3).map(|k| cell(p[k] - reach)).collect();
        let hi: Vec<i64> = (0..3).map(|k| cell(p[k] + reach)).collect();
        let mut seen: Vec<usize> = Vec::new();
        let mut seen_set = std::collections::HashSet::new();
        let mut best: Option<V> = None;
        let mut best_d2 = reach * reach;
        for x in lo[0]..=hi[0] {
            for y in lo[1]..=hi[1] {
                for z in lo[2]..=hi[2] {
                    for &i in grid.get(&(x, y, z)).map(Vec::as_slice).unwrap_or(&[]) {
                        if !seen_set.insert(i) {
                            continue;
                        }
                        seen.push(i);
                        let [a, b, c] = self.triangles[i];
                        let q = closest_on_triangle(p, a, b, c);
                        let d = sub(p, q);
                        let d2 = dot(d, d);
                        if d2 <= best_d2 {
                            best_d2 = d2;
                            best = Some(q);
                        }
                    }
                }
            }
        }
        let q = best?;
        let dist = best_d2.sqrt();
        if dist < 1e-9 {
            return Some(0.0);
        }
        // Sign: the side of the nearest triangles. Near an edge or corner several triangles are
        // equally near; their summed normals give the correct side for a closed mesh.
        let mut normal = [0.0; 3];
        for &j in &seen {
            let [a, b, c] = self.triangles[j];
            let qq = closest_on_triangle(p, a, b, c);
            let dd = sub(p, qq);
            if (dot(dd, dd).sqrt() - dist).abs() < 1e-6 * dist.max(1.0) {
                let g = cross(sub(b, a), sub(c, a));
                let n = dot(g, g).sqrt();
                if n > 0.0 {
                    normal = add(normal, mul(g, 1.0 / n));
                }
            }
        }
        let side = dot(sub(p, q), normal);
        Some(if side >= 0.0 { dist } else { -dist })
    }
}

#[cfg(test)]
mod tests {
    use super::Mesh;

    fn cube() -> Mesh {
        let mut m = Mesh::default();
        let c = |x: f64, y: f64, z: f64| [x, y, z];
        let quads = [
            [c(0., 0., 0.), c(0., 1., 0.), c(1., 1., 0.), c(1., 0., 0.)],
            [c(0., 0., 1.), c(1., 0., 1.), c(1., 1., 1.), c(0., 1., 1.)],
            [c(0., 0., 0.), c(1., 0., 0.), c(1., 0., 1.), c(0., 0., 1.)],
            [c(0., 1., 0.), c(0., 1., 1.), c(1., 1., 1.), c(1., 1., 0.)],
            [c(0., 0., 0.), c(0., 0., 1.), c(0., 1., 1.), c(0., 1., 0.)],
            [c(1., 0., 0.), c(1., 1., 0.), c(1., 1., 1.), c(1., 0., 1.)],
        ];
        for q in quads {
            m.add(q[0], q[1], q[2], None);
            m.add(q[0], q[2], q[3], None);
        }
        m
    }

    #[test]
    fn unit_cube() {
        let m = cube();
        assert!((m.volume() - 1.0).abs() < 1e-12);
        assert_eq!(m.bbox(), Some(([0.0; 3], [1.0; 3])));
        assert!((m.signed_distance([0.5, 0.5, 2.0], 3.0).unwrap() - 1.0).abs() < 1e-12);
        assert!((m.signed_distance([0.5, 0.5, 0.75], 3.0).unwrap() + 0.25).abs() < 1e-12);
        assert_eq!(m.signed_distance([50.0, 0.0, 0.0], 1.0), None);
    }
}
