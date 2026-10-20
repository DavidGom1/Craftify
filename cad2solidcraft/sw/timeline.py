"""Rebuild a SolidWorks part's feature tree as a SolidCraft timeline.

Supported now: sketches (lines, arcs, circles), extrusions (add or cut; blind or through all)
and chamfers and fillets. Chamfer and fillet edges are stored by SolidWorks as internal
names, so they are found geometrically: the model is rebuilt up to the step, and an edge is
taken when the SolidWorks part's own display mesh shows it cut away by about the depth that
chamfer or fillet removes.

The rebuilt part is accepted only when its volume and box match SolidWorks' mesh; otherwise
the caller keeps the imported exact geometry.
"""

from __future__ import annotations

import math
import re
import unicodedata
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable

from ..mesh import Mesh
from ..solidcraft import script as sc
from ..solidcraft.transform import Matrix
from . import profiles
from .ir import PartIR

Vec = tuple[float, float, float]
Log = Callable[[str], None]

VOLUME_TOLERANCE = 0.005  # relative
BOX_TOLERANCE = 0.1  # mm


class Unsupported(Exception):
    """A step this module cannot rebuild yet."""


@dataclass
class TimelineResult:
    ok: bool = False
    design: str | None = None
    steps: int = 0
    rebuilt: int = 0
    stopped_at: str | None = None  # the SolidWorks feature it could not rebuild
    reason: str | None = None
    volume_mm3: float | None = None
    truth_volume_mm3: float | None = None
    parameters: int = 0
    notes: list[str] = field(default_factory=list)


# --------------------------------------------------------------------------- helpers

def _apply(m: Matrix, p: Vec) -> Vec:
    return (m[0] * p[0] + m[1] * p[1] + m[2] * p[2], m[3] * p[0] + m[4] * p[1] + m[5] * p[2],
            m[6] * p[0] + m[7] * p[1] + m[8] * p[2])


def _cross(a: Vec, b: Vec) -> Vec:
    return (a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0])


def _vec(d: dict) -> Vec:
    return (float(d["x"]), float(d["y"]), float(d["z"]))


def _uv(d: dict) -> tuple[float, float]:
    return (float(d["u"]), float(d["v"]))


def param_name(feature: str, label: str) -> str:
    """A SolidCraft parameter name from a SolidWorks feature and dimension name."""
    text = unicodedata.normalize("NFKD", f"{feature}_{label}").encode("ascii", "ignore").decode()
    text = re.sub(r"[^A-Za-z0-9]+", "_", text).strip("_")
    return text if text and not text[0].isdigit() else f"p_{text}"


def _fmt(x: float) -> str:
    return f"{x:.6g}"


# --------------------------------------------------------------------------- sketches

def _arc_points(e: dict, loop_centre: tuple[float, float] | None) -> tuple[list, list, list]:
    """start, through, end of an arc. The stored angles carry no direction: the shorter arc is
    taken, and for a half circle the one bulging away from the rest of its sketch."""
    g = e["geometry"]
    (cu, cv), r = _uv(g["center"]), float(g["radius"])
    a0, a1 = float(g["start_angle"]), float(g["end_angle"])
    span = (a1 - a0) % (2 * math.pi)
    p0 = [cu + r * math.cos(a0), cv + r * math.sin(a0)]
    p1 = [cu + r * math.cos(a1), cv + r * math.sin(a1)]
    ccw_mid = a0 + span / 2
    cw_mid = ccw_mid + math.pi
    if abs(span - math.pi) < 1e-6 and loop_centre is not None:
        def far(a: float) -> float:
            return math.hypot(cu + r * math.cos(a) - loop_centre[0], cv + r * math.sin(a) - loop_centre[1])
        mid = ccw_mid if far(ccw_mid) >= far(cw_mid) else cw_mid
    else:
        mid = ccw_mid if span <= math.pi else cw_mid
    return p0, [cu + r * math.cos(mid), cv + r * math.sin(mid)], p1


def _sketch_commands(s: sc.Script, ir: PartIR, sketch_id: str, name: str, u: Matrix) -> int:
    sk = ir.sketches.get(sketch_id)
    if sk is None:
        raise Unsupported(f"sketch {sketch_id} not decoded")
    pl = sk.get("placement") or {}
    frame = pl.get("frame")
    if pl.get("kind") != "resolved" or not frame:
        raise Unsupported(f"sketch {name}: plane not resolved")
    o, n, x = _vec(frame["origin"]), _vec(frame["normal"]), _vec(frame["u_axis"])
    y = _cross(n, x)
    s.add("sketch.create", plane={"origin": list(_apply(u, o)), "x_dir": list(_apply(u, x)), "y_dir": list(_apply(u, y))}, name=name)
    curves = [e for e in ir.entities.get(sketch_id, []) if (e.get("geometry") or {}).get("kind") in ("line", "arc", "circle")]
    if not curves:
        raise Unsupported(f"sketch {name}: no curves decoded")
    ends = []
    for e in curves:
        g = e["geometry"]
        if g["kind"] == "line":
            ends += [_uv(g["start"]), _uv(g["end"])]
        else:
            ends.append(_uv(g["center"]))
    centre = (sum(p[0] for p in ends) / len(ends), sum(p[1] for p in ends) / len(ends))
    for e in curves:
        g = e["geometry"]
        construction = bool(e.get("construction"))
        if g["kind"] == "line":
            s.add("sketch.line", points=[list(_uv(g["start"])), list(_uv(g["end"]))], construction=construction)
        elif g["kind"] == "circle":
            if construction:
                continue
            s.add("sketch.circle.center", center=list(_uv(g["center"])), radius=float(g["radius"]))
        else:
            if construction:
                continue
            p0, mid, p1 = _arc_points(e, centre)
            s.add("sketch.arc.three_point", start=p0, through=mid, end=p1)
    s.add("sketch.finish")
    return len(curves)


def _sketch_of_profile(ir: PartIR, profile: dict) -> tuple[str, str]:
    """(sketch id, sketch feature name) an extrusion uses."""
    kind, value = profile.get("kind"), profile.get("value")
    if kind == "feature":
        f = ir.feature_by_id.get(value)
        if f is None or (f.get("definition") or {}).get("definition") != "sketch":
            raise Unsupported("profile is not a sketch")
        return f["definition"]["sketch"]["sketch"], f.get("name") or value
    if kind == "sketch":
        for f in ir.features:
            d = f.get("definition") or {}
            if d.get("definition") == "sketch" and (d.get("sketch") or {}).get("sketch") == value:
                return value, f.get("name") or value
        return value, (ir.sketches.get(value) or {}).get("name") or value
    raise Unsupported(f"profile kind {kind}")


# --------------------------------------------------------------------------- edges of chamfers and fillets

def _expected_depth(kind: str, size: float) -> tuple[float, float, float]:
    """(low, typical, high) depth the step removes at a sharp edge, by how open the corner is."""
    if kind == "chamfer":  # d·cos(θ/2): θ the angle between the faces
        return 0.2 * size, size * math.cos(math.pi / 4), 1.05 * size
    return 0.08 * size, size * (math.sqrt(2) - 1), 1.6 * size  # fillet: r/sin(θ/2) − r


def _pick_edges(edges: list[dict], truth: Mesh, kind: str, size: float, wanted: int) -> list[list[float]]:
    low, typical, high = _expected_depth(kind, size)
    scored = []
    for e in edges:
        mid = tuple(e["mid"])
        d = truth.signed_distance(mid, reach=high * 1.5 + 0.5)
        if d is None or d < low or d > high:
            continue
        # the whole edge must be cut away, not only its middle (a later cut can reach one part)
        ends_ok = True
        for key in ("start", "end"):
            p = e.get(key)
            if p is None:
                continue
            q = tuple(p)
            dq = truth.signed_distance(q, reach=high * 1.5 + 0.5)
            if dq is None or dq < 0.5 * low:
                ends_ok = False
        if ends_ok:
            scored.append((abs(d - typical), list(mid)))
    scored.sort(key=lambda t: t[0])
    return [p for _s, p in scored[: wanted if wanted > 0 else len(scored)]]


def _chamfer_spec(f: dict, ir: PartIR) -> tuple[str, float, dict]:
    d = f["definition"]
    groups = d.get("groups") or [{}]
    spec = groups[0].get("spec") or {}
    kind = d["definition"]
    params = {p.get("name"): p for p in ir.parameters.get(f["id"], [])}
    if kind == "chamfer":
        if spec.get("kind") == "distance_angle":
            dist, ang = float(spec["distance"]), float(spec["angle"])
            if abs(ang - math.pi / 4) < 1e-6:
                return kind, dist, {"distance": dist}
            return kind, dist, {"distance": dist, "angle": f"{math.degrees(ang):.6g} deg"}
        if spec.get("kind") == "distance":
            dist = float(spec["distance"])
            return kind, dist, {"distance": dist}
        raise Unsupported(f"chamfer spec {spec.get('kind')}")
    radius = spec.get("radius")
    if radius is None:
        lengths = [p for p in params.values() if (p.get("value") or {}).get("kind") == "length"]
        if len(lengths) != 1:
            raise Unsupported("fillet radius unknown")
        radius = (lengths[0].get("value") or {}).get("value")
    return kind, float(radius), {"radius": float(radius)}


def _edge_count(f: dict) -> int:
    n = 0
    for g in f["definition"].get("groups") or []:
        edges = (g.get("edges") or {}).get("value") or {}
        n += len(edges.get("edges") or [])
    return n


# --------------------------------------------------------------------------- build

def _ordered(ir: PartIR) -> list[dict]:
    """Feature order for rebuilding: cadmpeg sometimes lists cuts before the first added
    body, which cannot be; those cuts (and the sketches they use) move to after it."""
    feats = [f for f in ir.features if not f.get("suppressed")]
    first_join = next((i for i, f in enumerate(feats)
                       if (f.get("definition") or {}).get("definition") == "extrude"
                       and (f.get("definition") or {}).get("op") == "join"), None)
    if first_join is None:
        return feats
    early = feats[:first_join]
    moved = [f for f in early if (f.get("definition") or {}).get("definition") in ("extrude", "sketch")]
    keep = [f for f in early if f not in moved]
    return keep + [feats[first_join]] + moved + feats[first_join + 1:]


def _given_frame(ir: PartIR, sketch_id: str) -> profiles.Frame:
    sk = ir.sketches.get(sketch_id) or {}
    frame = (sk.get("placement") or {}).get("frame")
    if not frame:
        raise Unsupported("sketch plane not decoded")
    o, n, x = _vec(frame["origin"]), _vec(frame["normal"]), _vec(frame["u_axis"])
    return profiles.Frame(o, x, _cross(n, x))


def _extrude(s: sc.Script, ir: PartIR, f: dict, truth_sw: Mesh, u: Matrix, body: str, has_body: bool, res: "TimelineResult") -> None:
    d = f["definition"]
    name = f.get("name") or f["id"]
    if (d.get("direction") or {}).get("kind") != "profile_normal" or (d.get("start") or {}).get("kind") != "profile_plane":
        raise Unsupported("extrusion direction or start")
    ext = d.get("extent") or {}
    if ext.get("kind") != "one_sided":
        raise Unsupported(f"extent {ext.get('kind')}")
    term = (ext.get("side") or {}).get("termination") or {}
    op = d.get("op")
    if op not in ("join", "cut"):
        raise Unsupported(f"operation {op}")
    if term.get("kind") == "blind":
        depth: float | None = float(term["length"])
    elif term.get("kind") == "through_all":
        depth = None
    else:
        raise Unsupported(f"extrusion to {term.get('kind')}")
    sketch_id, sketch_name = _sketch_of_profile(ir, d.get("profile") or {})
    curves = [e for e in ir.entities.get(sketch_id, []) if (e.get("geometry") or {}).get("kind") in ("line", "arc", "circle")]
    if not curves:
        raise Unsupported(f"sketch {sketch_name}: no curves decoded")
    centre = _curves_centre(curves)
    loops = profiles.loops_from_curves(curves, lambda e: _arc_points(e, centre))
    regions = profiles.regions_from_loops(loops)
    choice = profiles.choose(regions, _given_frame(ir, sketch_id), truth_sw, op, depth)
    if choice is None:
        raise Unsupported(f"no region of {sketch_name} matches the SolidWorks part")
    fr = choice.frame
    if fr is not None and (fr.x, fr.y) != (_given_frame(ir, sketch_id).x, _given_frame(ir, sketch_id).y):
        res.notes.append(f"{sketch_name}: plane orientation taken from the part's geometry")
    s.add("sketch.create", name=sketch_name,
          plane={"origin": list(_apply(u, fr.origin)), "x_dir": list(_apply(u, fr.x)), "y_dir": list(_apply(u, fr.y))})
    for e in curves:
        g = e["geometry"]
        if e.get("construction"):
            continue
        if g["kind"] == "line":
            s.add("sketch.line", points=[list(_uv(g["start"])), list(_uv(g["end"]))])
        elif g["kind"] == "circle":
            s.add("sketch.circle.center", center=list(_uv(g["center"])), radius=float(g["radius"]))
        else:
            p0, mid, p1 = _arc_points(e, centre)
            s.add("sketch.arc.three_point", start=p0, through=mid, end=p1)
    s.add("sketch.finish")
    params: dict = dict(sketch=sketch_name, name=name, profiles=[{"point": list(r.samples[0])} for r in choice.regions])
    if choice.direction < 0:
        params["direction"] = "negative"
    if op == "cut":
        params["operation"] = "cut"
    elif not has_body:
        params["operation"] = "new"
        params["body_name"] = body
    else:
        params["operation"] = "join"
    if depth is None:
        params["through_all"] = True
    else:
        pname = param_name(name, "D1")
        s.add("parameters.add", name=pname, expression=f"{_fmt(depth)} mm")
        res.parameters += 1
        params["distance"] = pname
    s.add("solid.extrude", **params)


def _curves_centre(curves: list[dict]) -> tuple[float, float]:
    ends = []
    for e in curves:
        g = e["geometry"]
        if g["kind"] == "line":
            ends += [_uv(g["start"]), _uv(g["end"])]
        else:
            ends.append(_uv(g["center"]))
    return (sum(p[0] for p in ends) / len(ends), sum(p[1] for p in ends) / len(ends))


def build(ir: PartIR, stem: str, out: Path, cli: Path, u: Matrix, log: Log = print) -> TimelineResult:
    res = TimelineResult(steps=len(ir.features))
    truth_sw = ir.mesh()
    if not truth_sw.triangles:
        res.reason = "the part has no display mesh to check against"
        return res
    truth = truth_sw.transformed(u)
    res.truth_volume_mm3 = truth.volume()
    build_dir = out / "build"
    build_dir.mkdir(parents=True, exist_ok=True)
    design = out / "parts" / f"{stem}.solidcraft"
    design.parent.mkdir(parents=True, exist_ok=True)
    body = re.sub(r"[^A-Za-z0-9]+", "_", unicodedata.normalize("NFKD", stem).encode("ascii", "ignore").decode()).strip("_") or "Body"

    s = sc.Script()
    s.add("file.new", name=stem)
    has_body = False
    for f in _ordered(ir):
        d = f.get("definition") or {}
        kind = d.get("definition")
        name = f.get("name") or f.get("id")
        try:
            if kind == "sketch":
                pass  # drawn by the extrusion that uses it, on the plane the part confirms
            elif kind == "extrude":
                _extrude(s, ir, f, truth_sw, u, body, has_body, res)
                has_body = True
            elif kind in ("chamfer", "fillet"):
                ckind, size, spec = _chamfer_spec(f, ir)
                probe = sc.Script(list(s.commands))
                probe.add("model.edges", body=body)
                results = sc.run(cli, probe.write(build_dir / f"{stem}.probe.json"))
                edges = (results[-1] or {}).get("edges") or []
                picked = _pick_edges(edges, truth, ckind, size, _edge_count(f))
                if not picked:
                    raise Unsupported(f"{ckind}: its edges were not found")
                pname = param_name(name, "D1")
                value = spec.pop("distance", None) if ckind == "chamfer" else spec.pop("radius")
                s.add("parameters.add", name=pname, expression=f"{_fmt(value)} mm")
                res.parameters += 1
                if ckind == "chamfer":
                    s.add("solid.chamfer", edges=picked, distance=pname, body=body, **spec)
                else:
                    s.add("solid.fillet", edges=picked, radius=pname, body=body)
                if len(picked) != _edge_count(f):
                    res.notes.append(f"{name}: {len(picked)} edges found, SolidWorks lists {_edge_count(f)}")
            else:
                raise Unsupported(f"feature type {kind}")
            res.rebuilt += 1
        except Unsupported as e:
            res.stopped_at, res.reason = name, str(e)
            return res
        except sc.SolidCraftError as e:
            res.stopped_at, res.reason = name, f"SolidCraft: {e}"
            return res

    s.add("document.metadata.set", key=sc.METADATA_KEY, value={"source": str(ir.path), "source_cad": "SolidWorks", "timeline": True})
    s.add("file.save_as", path=str(design))
    try:
        sc.run(cli, s.write(build_dir / f"{stem}.timeline.json"))
        measured = sc.evaluate(cli, design)
    except sc.SolidCraftError as e:
        res.stopped_at, res.reason = "(rebuild)", f"SolidCraft: {e}"
        return res
    res.volume_mm3 = (measured.get("total") or {}).get("volume_mm3")
    res.design = str(design)
    lo, hi = truth.bbox()
    boxes = [b.get("bbox") for b in measured.get("bodies") or [] if b.get("bbox")]
    if not boxes or res.volume_mm3 is None:
        res.reason = "the rebuilt part has no bodies"
        return res
    blo = [min(b["min"][k] for b in boxes) for k in range(3)]
    bhi = [max(b["max"][k] for b in boxes) for k in range(3)]
    box_err = max(max(abs(blo[k] - lo[k]), abs(bhi[k] - hi[k])) for k in range(3))
    vol_err = abs(res.volume_mm3 - res.truth_volume_mm3) / max(abs(res.truth_volume_mm3), 1e-9)
    if vol_err > VOLUME_TOLERANCE or box_err > BOX_TOLERANCE:
        res.reason = f"does not match SolidWorks: volume off by {vol_err * 100:.2f} %, box by {box_err:.2f} mm"
        return res
    res.ok = True
    return res
