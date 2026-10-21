//! Placement maths shared by every source CAD. Port of `solidcraft/transform.py`.

use std::f64::consts::PI;

/// 3x3, row-major.
pub type Matrix = [f64; 9];
pub type Vec3 = [f64; 3];

pub const IDENTITY: Matrix = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
/// +90 degrees about X: the SolidWorks "up" (+Y) becomes SolidCraft's "up" (+Z).
pub const Y_UP_TO_Z_UP: Matrix = [1.0, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0, 1.0, 0.0];

pub fn transpose(m: &Matrix) -> Matrix {
    [m[0], m[3], m[6], m[1], m[4], m[7], m[2], m[5], m[8]]
}

/// m · p, with p a column vector.
pub fn apply(m: &Matrix, p: Vec3) -> Vec3 {
    [m[0] * p[0] + m[1] * p[1] + m[2] * p[2], m[3] * p[0] + m[4] * p[1] + m[5] * p[2], m[6] * p[0] + m[7] * p[1] + m[8] * p[2]]
}

pub fn matmul(a: &Matrix, b: &Matrix) -> Matrix {
    let mut out = [0.0; 9];
    for r in 0..3 {
        for c in 0..3 {
            out[3 * r + c] = crate::util::py_sum((0..3).map(|k| a[3 * r + k] * b[3 * k + c]));
        }
    }
    out
}

/// U·M·Uᵀ: a rotation seen in a frame turned by U.
pub fn conjugate(u: &Matrix, m: &Matrix) -> Matrix {
    matmul(&matmul(u, m), &transpose(u))
}

/// Axis (unit) and angle (radians) of a proper rotation acting on column vectors.
///
/// Identity gives ((0, 0, 1), 0.0).
pub fn axis_angle(m: &Matrix) -> (Vec3, f64) {
    let trace = m[0] + m[4] + m[8];
    let cos_a = ((trace - 1.0) / 2.0).clamp(-1.0, 1.0);
    let angle = cos_a.acos();
    if angle < 1e-12 {
        return ([0.0, 0.0, 1.0], 0.0);
    }
    if PI - angle > 1e-6 {
        let (x, y, z) = (m[7] - m[5], m[2] - m[6], m[3] - m[1]);
        let n = (x * x + y * y + z * z).sqrt();
        return ([x / n, y / n, z / n], angle);
    }
    // Half turn: the axis comes from the symmetric part, R = 2·a·aᵀ − I.
    let (xx, yy, zz) = ((m[0] + 1.0) / 2.0, (m[4] + 1.0) / 2.0, (m[8] + 1.0) / 2.0);
    let (x, y, z);
    if xx >= yy && xx >= zz {
        x = xx.max(0.0).sqrt();
        y = (m[1] + m[3]) / (4.0 * x);
        z = (m[2] + m[6]) / (4.0 * x);
    } else if yy >= zz {
        y = yy.max(0.0).sqrt();
        x = (m[1] + m[3]) / (4.0 * y);
        z = (m[5] + m[7]) / (4.0 * y);
    } else {
        z = zz.max(0.0).sqrt();
        x = (m[2] + m[6]) / (4.0 * z);
        y = (m[5] + m[7]) / (4.0 * z);
    }
    let n = (x * x + y * y + z * z).sqrt();
    ([x / n, y / n, z / n], PI)
}

pub fn rotation_from_axis_angle(axis: Vec3, angle: f64) -> Matrix {
    let [x, y, z] = axis;
    let (c, s, t) = (angle.cos(), angle.sin(), 1.0 - angle.cos());
    [
        t * x * x + c,
        t * x * y - s * z,
        t * x * z + s * y,
        t * x * y + s * z,
        t * y * y + c,
        t * y * z - s * x,
        t * x * z - s * y,
        t * y * z + s * x,
        t * z * z + c,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: &Matrix, b: &Matrix) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9)
    }

    #[test]
    fn axis_angle_round_trips() {
        for (axis, angle) in [([0.0, 0.0, 1.0], 0.7), ([1.0, 0.0, 0.0], PI), ([0.0, 0.6, 0.8], 2.0), ([0.6, 0.0, -0.8], PI)] {
            let m = rotation_from_axis_angle(axis, angle);
            let (a, t) = axis_angle(&m);
            assert!(close(&rotation_from_axis_angle(a, t), &m));
        }
        assert_eq!(axis_angle(&IDENTITY), ([0.0, 0.0, 1.0], 0.0));
    }

    #[test]
    fn y_up_turns_y_into_z() {
        assert_eq!(apply(&Y_UP_TO_Z_UP, [0.0, 1.0, 0.0]), [0.0, 0.0, 1.0]);
    }
}
