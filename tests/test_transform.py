import itertools
import math
import random
import unittest

from cad2solidcraft.solidcraft.transform import axis_angle, rotation_from_axis_angle


class AxisAngleTests(unittest.TestCase):
    def assert_same(self, a, b, tol=1e-9):
        for x, y in zip(a, b):
            self.assertAlmostEqual(x, y, delta=tol)

    def test_identity(self):
        self.assertEqual(axis_angle((1, 0, 0, 0, 1, 0, 0, 0, 1)), ((0.0, 0.0, 1.0), 0.0))

    def test_round_trip_random(self):
        rng = random.Random(7)
        for _ in range(500):
            v = [rng.uniform(-1, 1) for _ in range(3)]
            n = math.sqrt(sum(c * c for c in v))
            axis = tuple(c / n for c in v)
            angle = rng.uniform(1e-4, math.pi - 1e-4)
            m = rotation_from_axis_angle(axis, angle)
            a2, ang2 = axis_angle(m)
            self.assert_same(rotation_from_axis_angle(a2, ang2), m)

    def test_axis_aligned_quarter_and_half_turns(self):
        # Every signed permutation matrix with determinant +1 (the 24 cube rotations).
        for perm in itertools.permutations(range(3)):
            for signs in itertools.product((1, -1), repeat=3):
                m = [0.0] * 9
                for r, c in enumerate(perm):
                    m[3 * r + c] = float(signs[r])
                det = (m[0] * (m[4] * m[8] - m[5] * m[7]) - m[1] * (m[3] * m[8] - m[5] * m[6])
                       + m[2] * (m[3] * m[7] - m[4] * m[6]))
                if det < 0:
                    continue
                axis, angle = axis_angle(tuple(m))
                self.assert_same(rotation_from_axis_angle(axis, angle), m)


if __name__ == "__main__":
    unittest.main()
