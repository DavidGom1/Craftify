import math
import unittest

from cad2solidcraft.step_clean import _Curve, _Edit, _piece, _ref, clean


def _cylinder(base: int, r: float, h: float) -> tuple[str, int]:
    """A closed cylinder the way cadmpeg writes it: the side face has a loop at each end and no
    seam. Returns the entities and the id of its CLOSED_SHELL."""
    i = lambda k: f"#{base + k}"  # noqa: E731
    lines = [
        f"{i(1)} = CARTESIAN_POINT('',(0.,0.,0.));",
        f"{i(2)} = DIRECTION('',(0.,0.,1.));",
        f"{i(3)} = DIRECTION('',(1.,0.,0.));",
        f"{i(4)} = AXIS2_PLACEMENT_3D('',{i(1)},{i(2)},{i(3)});",
        f"{i(5)} = CARTESIAN_POINT('',(0.,0.,{h!r}));",
        f"{i(6)} = AXIS2_PLACEMENT_3D('',{i(5)},{i(2)},{i(3)});",
        f"{i(10)} = CIRCLE('',{i(4)},{r!r});",
        f"{i(11)} = CIRCLE('',{i(6)},{r!r});",
        f"{i(12)} = CARTESIAN_POINT('',({r!r},0.,0.));",
        f"{i(13)} = CARTESIAN_POINT('',({r!r},0.,{h!r}));",
        f"{i(14)} = VERTEX_POINT('',{i(12)});",
        f"{i(15)} = VERTEX_POINT('',{i(13)});",
        f"{i(16)} = EDGE_CURVE('',{i(14)},{i(14)},{i(10)},.T.);",
        f"{i(17)} = EDGE_CURVE('',{i(15)},{i(15)},{i(11)},.T.);",
        f"{i(20)} = CYLINDRICAL_SURFACE('',{i(4)},{r!r});",
        f"{i(21)} = ORIENTED_EDGE('',*,*,{i(16)},.T.);",
        f"{i(22)} = EDGE_LOOP('',({i(21)}));",
        f"{i(23)} = FACE_BOUND('',{i(22)},.T.);",
        f"{i(24)} = ORIENTED_EDGE('',*,*,{i(17)},.F.);",
        f"{i(25)} = EDGE_LOOP('',({i(24)}));",
        f"{i(26)} = FACE_BOUND('',{i(25)},.T.);",
        f"{i(27)} = ADVANCED_FACE('',({i(23)},{i(26)}),{i(20)},.T.);",
        f"{i(30)} = PLANE('',{i(4)});",
        f"{i(31)} = ORIENTED_EDGE('',*,*,{i(16)},.F.);",
        f"{i(32)} = EDGE_LOOP('',({i(31)}));",
        f"{i(33)} = FACE_BOUND('',{i(32)},.T.);",
        f"{i(34)} = ADVANCED_FACE('',({i(33)}),{i(30)},.F.);",
        f"{i(40)} = PLANE('',{i(6)});",
        f"{i(41)} = ORIENTED_EDGE('',*,*,{i(17)},.T.);",
        f"{i(42)} = EDGE_LOOP('',({i(41)}));",
        f"{i(43)} = FACE_BOUND('',{i(42)},.T.);",
        f"{i(44)} = ADVANCED_FACE('',({i(43)}),{i(40)},.T.);",
        f"{i(50)} = CLOSED_SHELL('',({i(27)},{i(34)},{i(44)}));",
    ]
    return "\n".join(lines), base + 50


def _file(body: str) -> str:
    return ("ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\n"
            "#900 = UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(1.E-05),#901,'distance_accuracy_value','');\n"
            f"{body}\nENDSEC;\nEND-ISO-10303-21;\n")


def _uses(st: _Edit, shell: int) -> dict[int, list[tuple[int, bool]]]:
    """Edge -> (face, travelled forward) for every use in the shell."""
    out: dict[int, list[tuple[int, bool]]] = {}
    for f in st.ent[shell].args[1]:
        for b in st.ent[_ref(f)].args[1]:
            bound = st.ent[_ref(b)]
            for oe in st.ent[_ref(bound.args[1])].args[1]:
                o = st.ent[_ref(oe)]
                out.setdefault(_ref(o.args[3]), []).append((_ref(f), (o.args[4] == ".T.") == (bound.args[2] == ".T.")))
    return out


class SeamTest(unittest.TestCase):
    def setUp(self):
        body, self.shell = _cylinder(0, 5.0, 10.0)
        text, self.done = clean(_file(body + "\n#51 = MANIFOLD_SOLID_BREP('',#50);"))
        self.st = _Edit(text)

    def test_side_face_gets_one_loop_with_a_seam(self):
        self.assertEqual(self.done["seams"], 1)
        face = self.st.ent[27]
        self.assertEqual(len(face.args[1]), 1)
        bound = self.st.ent[_ref(face.args[1][0])]
        self.assertEqual(bound.type, "FACE_OUTER_BOUND")
        edges = [_ref(self.st.ent[_ref(oe)].args[3]) for oe in self.st.ent[_ref(bound.args[1])].args[1]]
        seams = [e for e in set(edges) if edges.count(e) == 2]
        self.assertEqual(len(seams), 1)
        seam = self.st.ent[seams[0]]
        for v in (seam.args[1], seam.args[2]):
            x, y, _z = self.st.vertex(_ref(v))
            self.assertAlmostEqual(x, -5.0, places=9)  # half a turn from the reference direction
            self.assertAlmostEqual(y, 0.0, places=9)

    def test_shell_stays_closed_and_consistent(self):
        for edge, uses in _uses(self.st, self.shell).items():
            self.assertEqual(len(uses), 2, edge)
            self.assertNotEqual(uses[0][1], uses[1][1], edge)  # once each way

    def test_split_circles_keep_their_vertices_on_the_circle(self):
        for e in self.st.ent.values():
            if e.type == "EDGE_CURVE" and self.st.ent[_ref(e.args[3])].type == "CIRCLE":
                for v in (e.args[1], e.args[2]):
                    x, y, _z = self.st.vertex(_ref(v))
                    self.assertAlmostEqual(math.hypot(x, y), 5.0, places=9)

    def test_tolerance_is_declared(self):
        text, _ = clean(_file(_cylinder(0, 5.0, 10.0)[0]))
        self.assertIn("LENGTH_MEASURE(0.001)", text)


class OuterShellTest(unittest.TestCase):
    def test_largest_shell_becomes_the_outer_one(self):
        big, big_shell = _cylinder(0, 5.0, 10.0)
        small, small_shell = _cylinder(100, 1.0, 2.0)
        body = (f"{big}\n{small}\n#300 = ORIENTED_CLOSED_SHELL('',*,#{big_shell},.F.);\n"
                f"#301 = BREP_WITH_VOIDS('',#{small_shell},(#300));")
        text, done = clean(_file(body))
        self.assertEqual(done["outer shells"], 1)
        st = _Edit(text)
        self.assertEqual(_ref(st.ent[301].args[1]), big_shell)
        self.assertEqual(_ref(st.ent[300].args[2]), small_shell)


class PieceTest(unittest.TestCase):
    def test_a_stretch_of_a_bspline_is_the_same_curve(self):
        pts = ["#%d" % (10 + k) for k in range(6)]
        coords = [(0, 0, 0), (1, 2, 0), (3, 3, 1), (5, 1, 2), (6, -1, 2), (8, 0, 3)]
        body = "\n".join(f"#{10 + k} = CARTESIAN_POINT('',({x!r},{y!r},{z!r}));" for k, (x, y, z) in enumerate(coords))
        body += f"\n#20 = B_SPLINE_CURVE_WITH_KNOTS('',3,({','.join(pts)}),.UNSPECIFIED.,.F.,.F.,(4,1,1,4),(0.,0.3,0.7,1.),.UNSPECIFIED.);"
        st = _Edit(_file(body))
        c = _Curve(st, 20)
        pw = [(x * w, y * w, z * w, w) for (x, y, z), w in zip(c.ctrl, c.w)]
        knots, ctrl = _piece(c.p, list(c.u), pw, 0.2, 0.85)
        piece = _Curve(st, 20)
        piece.u, piece.ctrl, piece.w = knots, [(q[0] / q[3], q[1] / q[3], q[2] / q[3]) for q in ctrl], [q[3] for q in ctrl]
        for k in range(11):
            t = 0.2 + 0.65 * k / 10
            for a, b in zip(c.at(t), piece.at(t)):
                self.assertAlmostEqual(a, b, places=9)
        self.assertEqual(knots[0], 0.2)
        self.assertEqual(knots[-1], 0.85)


if __name__ == "__main__":
    unittest.main()
