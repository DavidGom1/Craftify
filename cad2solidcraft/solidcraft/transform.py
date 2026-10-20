"""Placement maths shared by every source CAD."""

from __future__ import annotations

import math

Matrix = tuple[float, ...]  # 3x3, row-major


def transpose(m: Matrix) -> Matrix:
    return (m[0], m[3], m[6], m[1], m[4], m[7], m[2], m[5], m[8])


def apply(m: Matrix, p: tuple[float, float, float]) -> tuple[float, float, float]:
    """m · p, with p a column vector."""
    return (
        m[0] * p[0] + m[1] * p[1] + m[2] * p[2],
        m[3] * p[0] + m[4] * p[1] + m[5] * p[2],
        m[6] * p[0] + m[7] * p[1] + m[8] * p[2],
    )


def axis_angle(m: Matrix) -> tuple[tuple[float, float, float], float]:
    """Axis (unit) and angle (radians) of a proper rotation acting on column vectors.

    Identity gives ((0, 0, 1), 0.0).
    """
    trace = m[0] + m[4] + m[8]
    cos_a = max(-1.0, min(1.0, (trace - 1.0) / 2.0))
    angle = math.acos(cos_a)
    if angle < 1e-12:
        return (0.0, 0.0, 1.0), 0.0
    if math.pi - angle > 1e-6:
        x, y, z = m[7] - m[5], m[2] - m[6], m[3] - m[1]
        n = math.sqrt(x * x + y * y + z * z)
        return (x / n, y / n, z / n), angle
    # Half turn: the axis comes from the symmetric part, R = 2·a·aᵀ − I.
    xx, yy, zz = (m[0] + 1) / 2, (m[4] + 1) / 2, (m[8] + 1) / 2
    if xx >= yy and xx >= zz:
        x = math.sqrt(max(xx, 0.0))
        y, z = (m[1] + m[3]) / (4 * x), (m[2] + m[6]) / (4 * x)
    elif yy >= zz:
        y = math.sqrt(max(yy, 0.0))
        x, z = (m[1] + m[3]) / (4 * y), (m[5] + m[7]) / (4 * y)
    else:
        z = math.sqrt(max(zz, 0.0))
        x, y = (m[2] + m[6]) / (4 * z), (m[5] + m[7]) / (4 * z)
    n = math.sqrt(x * x + y * y + z * z)
    return (x / n, y / n, z / n), math.pi


def rotation_from_axis_angle(axis: tuple[float, float, float], angle: float) -> Matrix:
    x, y, z = axis
    c, s, t = math.cos(angle), math.sin(angle), 1 - math.cos(angle)
    return (
        t * x * x + c, t * x * y - s * z, t * x * z + s * y,
        t * x * y + s * z, t * y * y + c, t * y * z - s * x,
        t * x * z - s * y, t * y * z + s * x, t * z * z + c,
    )
