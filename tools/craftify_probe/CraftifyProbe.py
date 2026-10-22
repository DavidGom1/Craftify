"""CraftifyProbe: research add-in for Craftify's offline Fusion reader.

Watches work/jobs next to this file. A job is a folder with job.json ({"source": "<.f3d>"});
the add-in imports the design into a new document, writes probe.json (everything Fusion knows:
bodies, timeline, sketches, features, parameters) and closes the document without saving.
It only reads designs; it never saves or uploads anything.
"""

import json
import math
import os
import threading
import time
import traceback

import adsk.core
import adsk.fusion

EVENT_ID = "craftify_probe_job"
ROOT_DIR = os.path.join(os.path.dirname(os.path.abspath(__file__)), "work")
JOBS = os.path.join(ROOT_DIR, "jobs")
ALIVE = os.path.join(ROOT_DIR, "alive.json")

_app = None
_event = None
_handlers = []
_stop = None
_busy = False


def _write_json(path, data):
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(data, f, indent=1, ensure_ascii=False, default=str)
    os.replace(tmp, path)


def safe(f, default=None):
    try:
        return f()
    except Exception:
        return default


def pt(p):
    return None if p is None else [round(p.x * 10, 6), round(p.y * 10, 6), round(p.z * 10, 6)]


def vec(v):
    return None if v is None else [round(v.x, 9), round(v.y, 9), round(v.z, 9)]


def mat(m):
    return None if m is None else [round(x, 9) for x in m.asArray()]


def bbox(b):
    bb = safe(lambda: b.boundingBox)
    return None if bb is None else [pt(bb.minPoint), pt(bb.maxPoint)]


def body_info(b, i):
    return {
        "index": i,
        "name": b.name,
        "token": safe(lambda: b.entityToken),
        "solid": safe(lambda: b.isSolid),
        "visible": safe(lambda: b.isVisible),
        "volume_mm3": safe(lambda: b.volume * 1000.0),
        "area_mm2": safe(lambda: b.area * 100.0),
        "faces": safe(lambda: b.faces.count),
        "edges": safe(lambda: b.edges.count),
        "bbox_mm": bbox(b),
        "material": safe(lambda: b.material.name),
        "appearance": safe(lambda: b.appearance.name),
        "revision": safe(lambda: b.revisionId),
    }


def curve_info(c):
    t = c.objectType.split("::")[-1]
    d = {"type": t, "construction": safe(lambda: c.isConstruction), "fixed": safe(lambda: c.isFixed),
         "reference": safe(lambda: c.isReference), "token": safe(lambda: c.entityToken)}
    if t == "SketchLine":
        d.update(start=pt(c.startSketchPoint.geometry), end=pt(c.endSketchPoint.geometry))
    elif t == "SketchCircle":
        d.update(center=pt(c.centerSketchPoint.geometry), radius=c.radius * 10)
    elif t == "SketchArc":
        g = c.geometry
        d.update(center=pt(c.centerSketchPoint.geometry), radius=c.radius * 10, start=pt(c.startSketchPoint.geometry),
                 end=pt(c.endSketchPoint.geometry), start_angle=safe(lambda: g.startAngle), end_angle=safe(lambda: g.endAngle),
                 normal=safe(lambda: vec(g.normal)))
    elif t == "SketchEllipse":
        d.update(center=pt(c.centerSketchPoint.geometry), major=safe(lambda: c.majorAxisRadius * 10),
                 minor=safe(lambda: c.minorAxisRadius * 10), major_axis=safe(lambda: vec(c.majorAxis)))
    elif t in ("SketchFittedSpline", "SketchFixedSpline", "SketchControlPointSpline"):
        g = safe(lambda: c.geometry)
        if t == "SketchFittedSpline":
            d["fit_points"] = safe(lambda: [pt(p.geometry) for p in c.fitPoints])
        if g is not None:
            r = safe(lambda: g.getData())
            if r:
                ok, ctrl, degree, knots, rational, weights, periodic = r
                d.update(degree=degree, control=[pt(p) for p in ctrl], knots=list(knots), rational=rational,
                         weights=list(weights), periodic=periodic)
    else:
        d["start"] = safe(lambda: pt(c.startSketchPoint.geometry))
        d["end"] = safe(lambda: pt(c.endSketchPoint.geometry))
    return d


def sketch_info(sk):
    d = {
        "name": sk.name,
        "token": safe(lambda: sk.entityToken),
        "plane": safe(lambda: sk.referencePlane.objectType + ":" + (getattr(sk.referencePlane, "name", "") or "")),
        "transform": safe(lambda: mat(sk.transform)),
        "origin": safe(lambda: pt(sk.origin)),
        "x_dir": safe(lambda: vec(sk.xDirection)),
        "y_dir": safe(lambda: vec(sk.yDirection)),
        "visible": safe(lambda: sk.isVisible),
        "fully_constrained": safe(lambda: sk.isFullyConstrained),
        "timeline_index": safe(lambda: sk.timelineObject.index),
        "profiles": safe(lambda: sk.profiles.count),
        "curves": [curve_info(c) for c in sk.sketchCurves],
        "points": safe(lambda: [pt(p.geometry) for p in sk.sketchPoints], []),
        "texts": [],
        "constraints": [],
        "dimensions": [],
    }
    for tx in safe(lambda: list(sk.sketchTexts), []):
        d["texts"].append({"text": safe(lambda: tx.text), "font": safe(lambda: tx.fontName), "height": safe(lambda: tx.height * 10),
                           "bbox": safe(lambda: [pt(tx.boundingBox.minPoint), pt(tx.boundingBox.maxPoint)]),
                           "bold": safe(lambda: tx.isBold), "italic": safe(lambda: tx.isItalic)})
    for c in safe(lambda: list(sk.geometricConstraints), []):
        d["constraints"].append(c.objectType.split("::")[-1])
    for dim in safe(lambda: list(sk.sketchDimensions), []):
        d["dimensions"].append({"type": dim.objectType.split("::")[-1], "parameter": safe(lambda: dim.parameter.name),
                                "expression": safe(lambda: dim.parameter.expression), "value": safe(lambda: dim.parameter.value),
                                "driving": safe(lambda: dim.isDriving)})
    return d


def extent_info(e):
    if e is None:
        return None
    t = e.objectType.split("::")[-1]
    d = {"type": t}
    d["distance"] = safe(lambda: {"name": e.distance.name, "expression": e.distance.expression, "value": e.distance.value})
    d["flipped"] = safe(lambda: e.isPositiveDirection)
    return d


def feature_info(f):
    t = f.objectType.split("::")[-1]
    d = {"type": t, "name": safe(lambda: f.name), "token": safe(lambda: f.entityToken),
         "operation": safe(lambda: int(f.operation)), "suppressed": safe(lambda: f.isSuppressed),
         "health": safe(lambda: int(f.healthState)), "timeline_index": safe(lambda: f.timelineObject.index),
         "bodies": safe(lambda: [b.name for b in f.bodies], []),
         "participants": safe(lambda: [b.name for b in f.participantBodies], [])}
    if t == "ExtrudeFeature":
        d["profiles"] = safe(lambda: [profile_info(p) for p in _profiles(f.profile)])
        d["start"] = safe(lambda: f.startExtent.objectType.split("::")[-1])
        d["start_offset"] = safe(lambda: f.startExtent.offset.expression)
        d["extent_type"] = safe(lambda: int(f.extentType))
        d["one"] = safe(lambda: extent_info(f.extentOne))
        d["two"] = safe(lambda: extent_info(f.extentTwo))
        d["taper_one"] = safe(lambda: f.taperAngleOne.expression)
        d["solid"] = safe(lambda: f.isSolid)
    if t == "FilletFeature":
        d["edge_sets"] = []
        for es in safe(lambda: list(f.edgeSets), []):
            d["edge_sets"].append({"type": es.objectType.split("::")[-1], "radius": safe(lambda: es.radius.expression),
                                   "tangent_chain": safe(lambda: es.isTangentChain),
                                   "edges": safe(lambda: [edge_info(es.edges.item(i)) for i in range(es.edges.count)], [])})
    # Every parameter and matrix the feature exposes (a primitive's sizes and placement...).
    d["parameters"] = {}
    d["matrices"] = {}
    for attr in dir(f):
        if attr.startswith("_") or attr in ("parentComponent", "timelineObject", "nativeObject", "assemblyContext"):
            continue
        v = safe(lambda: getattr(f, attr))
        if v is None or callable(v):
            continue
        if hasattr(v, "expression") and hasattr(v, "value"):
            d["parameters"][attr] = {"name": safe(lambda: v.name), "expression": safe(lambda: v.expression), "value": safe(lambda: v.value)}
        elif hasattr(v, "asArray") and safe(lambda: len(v.asArray())) == 16:
            d["matrices"][attr] = mat(v)
        elif hasattr(v, "x") and hasattr(v, "y") and hasattr(v, "z") and not hasattr(v, "count"):
            d["matrices"][attr] = [safe(lambda: v.x), safe(lambda: v.y), safe(lambda: v.z)]
    return d


def edge_info(e):
    g = safe(lambda: e.geometry)
    ev = safe(lambda: e.evaluator)
    mid = None
    if ev is not None:
        r = safe(lambda: ev.getParameterExtents())
        if r and r[0]:
            p = safe(lambda: ev.getPointAtParameter((r[1] + r[2]) / 2))
            if p and p[0]:
                mid = pt(p[1])
    return {"type": safe(lambda: g.objectType.split("::")[-1]), "body": safe(lambda: e.body.name),
            "start": safe(lambda: pt(e.startVertex.geometry)), "end": safe(lambda: pt(e.endVertex.geometry)),
            "mid": mid, "length_mm": safe(lambda: e.length * 10)}


def profile_info(p):
    a = safe(lambda: p.areaProperties())
    sk = safe(lambda: p.parentSketch)
    idx = None
    if sk is not None:
        for i in range(safe(lambda: sk.profiles.count, 0)):
            if safe(lambda: sk.profiles.item(i) == p):
                idx = i
                break
    return {"sketch": safe(lambda: sk.name), "index": idx, "area_mm2": safe(lambda: a.area * 100.0),
            "centroid": safe(lambda: pt(a.centroid)), "loops": safe(lambda: p.profileLoops.count)}


def _profiles(p):
    if p is None:
        return []
    if hasattr(p, "count") and hasattr(p, "item"):
        return [p.item(i) for i in range(p.count)]
    return [p]


def comp_info(comp):
    d = {"name": comp.name, "id": safe(lambda: comp.id), "token": safe(lambda: comp.entityToken),
         "part_number": safe(lambda: comp.partNumber), "description": safe(lambda: comp.description),
         "bodies": [body_info(b, i) for i, b in enumerate(comp.bRepBodies)],
         "sketches": [sketch_info(s) for s in comp.sketches],
         "features": [feature_info(f) for f in safe(lambda: list(comp.features), [])],
         "construction_planes": safe(lambda: [p.name for p in comp.constructionPlanes], []),
         "occurrences": []}
    for o in comp.occurrences:
        d["occurrences"].append({"name": o.name, "component": o.component.name, "token": safe(lambda: o.entityToken),
                                 "transform": safe(lambda: mat(o.transform2)), "grounded": safe(lambda: o.isGrounded),
                                 "visible": safe(lambda: o.isLightBulbOn)})
    return d


def run_job(folder):
    result = {"ok": False}
    doc = None
    try:
        with open(os.path.join(folder, "job.json"), encoding="utf-8") as f:
            job = json.load(f)
        app = adsk.core.Application.get()
        opts = app.importManager.createFusionArchiveImportOptions(job["source"])
        doc = app.importManager.importToNewDocument(opts)
        design = adsk.fusion.Design.cast(doc.products.itemByProductType("DesignProductType"))
        result["design_type"] = safe(lambda: int(design.designType))
        result["units"] = safe(lambda: design.unitsManager.defaultLengthUnits)
        result["components"] = [comp_info(c) for c in design.allComponents]
        result["root"] = design.rootComponent.name
        result["timeline"] = []
        for i in range(safe(lambda: design.timeline.count, 0)):
            t = design.timeline.item(i)
            e = safe(lambda: t.entity)
            result["timeline"].append({"index": i, "name": safe(lambda: t.name), "type": safe(lambda: e.objectType.split("::")[-1]),
                                       "suppressed": safe(lambda: t.isSuppressed), "group": safe(lambda: t.isGroup),
                                       "parent_group": safe(lambda: t.parentGroup.name),
                                       "component": safe(lambda: e.parentComponent.name),
                                       "token": safe(lambda: e.entityToken)})
        result["parameters"] = [{"name": safe(lambda: p.name), "expression": safe(lambda: p.expression),
                                 "value": safe(lambda: p.value), "unit": safe(lambda: p.unit),
                                 "user": p.objectType.endswith("UserParameter"),
                                 "role": safe(lambda: p.role), "created_by": safe(lambda: p.createdBy.name)}
                                for p in design.allParameters]
        result["ok"] = True
    except Exception:
        result["error"] = traceback.format_exc()
    finally:
        if doc is not None:
            safe(lambda: doc.close(False))
        _write_json(os.path.join(folder, "probe.json"), result)


class _Handler(adsk.core.CustomEventHandler):
    def notify(self, args):
        global _busy
        try:
            run_job(json.loads(args.additionalInfo)["dir"])
        finally:
            _busy = False


def _poll():
    global _busy
    while not _stop.is_set():
        try:
            _write_json(ALIVE, {"time": time.time(), "busy": _busy, "root": ROOT_DIR, "jobs": sorted(os.listdir(JOBS)) if os.path.isdir(JOBS) else None})
            if not _busy and os.path.isdir(JOBS):
                for d in sorted(os.listdir(JOBS)):
                    folder = os.path.join(JOBS, d)
                    if os.path.isfile(os.path.join(folder, "job.json")) and not os.path.exists(os.path.join(folder, "taken")):
                        open(os.path.join(folder, "taken"), "w").close()
                        _busy = True
                        _app.fireCustomEvent(EVENT_ID, json.dumps({"dir": folder}))
                        break
        except Exception:
            with open(os.path.join(ROOT_DIR, "poll_error.txt"), "a", encoding="utf-8") as f:
                f.write(traceback.format_exc())
        _stop.wait(1.0)


def run(context):
    global _app, _event, _stop
    try:
        _app = adsk.core.Application.get()
        os.makedirs(JOBS, exist_ok=True)
        _event = _app.registerCustomEvent(EVENT_ID)
        h = _Handler()
        _event.add(h)
        _handlers.append(h)
        _stop = threading.Event()
        threading.Thread(target=_poll, daemon=True).start()
    except Exception:
        os.makedirs(ROOT_DIR, exist_ok=True)
        with open(os.path.join(ROOT_DIR, "addin_error.txt"), "w", encoding="utf-8") as f:
            f.write(traceback.format_exc())


def stop(context):
    try:
        if _stop is not None:
            _stop.set()
        if _app is not None:
            _app.unregisterCustomEvent(EVENT_ID)
        if os.path.exists(ALIVE):
            os.remove(ALIVE)
    except Exception:
        pass
