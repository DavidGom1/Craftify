"""Fusion add-in: exports designs for cad2solidcraft.

It watches %LOCALAPPDATA%/cad2solidcraft/fusion/jobs. A job is a folder with job.json
({"source": "<.f3d or .f3z>"}); the add-in opens the design in a new document, writes into the
job folder one STEP per component that has bodies (its own bodies, in its own coordinates) and
result.json with every component and every occurrence (placement in the root's space, mm),
then closes the document without saving.

Fusion's API only runs on its main thread, so a background thread finds new jobs and fires a
custom event that does the work.
"""

import json
import os
import threading
import time
import traceback

import adsk.core
import adsk.fusion

EVENT_ID = "cad2solidcraft_export_job"
ROOT_DIR = os.path.join(os.environ.get("LOCALAPPDATA", os.path.expanduser("~")), "cad2solidcraft", "fusion")
JOBS = os.path.join(ROOT_DIR, "jobs")
ALIVE = os.path.join(ROOT_DIR, "alive.json")
VERSION = "0.1.0"

_app = None
_event = None
_handlers = []
_stop = None
_busy = False


def _write_json(path, data):
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(data, f, indent=1, ensure_ascii=False)
    os.replace(tmp, path)


class _Log:
    def __init__(self, folder):
        self.path = os.path.join(folder, "log.txt")

    def __call__(self, text):
        with open(self.path, "a", encoding="utf-8") as f:
            f.write(time.strftime("%H:%M:%S ") + text + "\n")


def _bbox_mm(bodies):
    lo, hi = [float("inf")] * 3, [float("-inf")] * 3
    for b in bodies:
        bb = b.boundingBox
        for k, (a, c) in enumerate(zip(bb.minPoint.asArray(), bb.maxPoint.asArray())):
            lo[k] = min(lo[k], a * 10.0)
            hi[k] = max(hi[k], c * 10.0)
    return [lo, hi] if bodies else None


def _export_own_bodies(app, bodies, path, log):
    """STEP of some bodies only: copies of them in the root of a temporary direct design."""
    tdoc = app.documents.add(adsk.core.DocumentTypes.FusionDesignDocumentType)
    try:
        tdesign = adsk.fusion.Design.cast(tdoc.products.itemByProductType("DesignProductType"))
        tdesign.designType = adsk.fusion.DesignTypes.DirectDesignType
        tbm = adsk.fusion.TemporaryBRepManager.get()
        for b in bodies:
            tdesign.rootComponent.bRepBodies.add(tbm.copy(b))
        opts = tdesign.exportManager.createSTEPExportOptions(path, tdesign.rootComponent)
        tdesign.exportManager.execute(opts)
    finally:
        tdoc.close(False)


def _run_job(folder):
    log = _Log(folder)
    result = {"exporter": VERSION, "ok": False}
    doc = None
    try:
        with open(os.path.join(folder, "job.json"), encoding="utf-8") as f:
            job = json.load(f)
        src = job["source"]
        log("opening " + src)
        app = adsk.core.Application.get()
        opts = app.importManager.createFusionArchiveImportOptions(src)
        doc = app.importManager.importToNewDocument(opts)
        design = adsk.fusion.Design.cast(doc.products.itemByProductType("DesignProductType"))
        root = design.rootComponent
        try:
            up = app.preferences.generalPreferences.defaultModelingOrientation
            result["up"] = "z" if up == adsk.core.DefaultModelingOrientations.ZUpModelingOrientation else "y"
        except Exception:
            result["up"] = "y"
        result["name"] = root.name
        comps = {}

        def entry(comp):
            key = comp.id if comp != root else "root"
            if key in comps:
                return comps[key]
            bodies = [b for b in comp.bRepBodies if b.isSolid]
            e = {
                "id": key,
                "name": comp.name,
                "bodies": len(bodies),
                "volume_mm3": sum(b.volume for b in bodies) * 1000.0,
                "area_mm2": sum(b.area for b in bodies) * 100.0,
                "bbox_mm": _bbox_mm(bodies),
                "materials": sorted({b.material.name for b in bodies if b.material}),
                "step": None,
            }
            comps[key] = e
            if bodies:
                name = "c%03d.step" % len([c for c in comps.values() if c.get("step")])
                path = os.path.join(folder, name)
                if comp.occurrences.count == 0 and comp != root:
                    o = design.exportManager.createSTEPExportOptions(path, comp)
                    design.exportManager.execute(o)
                else:
                    _export_own_bodies(app, bodies, path, log)
                e["step"] = name if os.path.isfile(path) else None
                if e["step"] is None:
                    log("STEP export failed for " + comp.name)
            log("component %s: %d bodies" % (comp.name, len(bodies)))
            return e

        entry(root)
        occs = []
        for occ in root.allOccurrences:
            e = entry(occ.component)
            m = occ.transform2
            occs.append({
                "path": occ.fullPathName,
                "component": e["id"],
                "rotation": [[m.getCell(r, c) for c in range(3)] for r in range(3)],
                "translation_mm": [m.getCell(r, 3) * 10.0 for r in range(3)],
                "visible": bool(occ.isLightBulbOn),
                "referenced": bool(occ.isReferencedComponent),
            })
        result["components"] = list(comps.values())
        result["occurrences"] = occs
        result["ok"] = True
        log("exported %d components, %d occurrences" % (len(comps), len(occs)))
    except Exception:
        result["error"] = traceback.format_exc()
        log("ERROR\n" + result["error"])
    finally:
        if doc is not None:
            try:
                doc.close(False)
            except Exception:
                pass
        _write_json(os.path.join(folder, "result.json"), result)


class _JobHandler(adsk.core.CustomEventHandler):
    def notify(self, args):
        global _busy
        try:
            _run_job(json.loads(args.additionalInfo)["dir"])
        except Exception:
            pass
        finally:
            _busy = False


def _poll():
    global _busy
    while not _stop.is_set():
        try:
            _write_json(ALIVE, {"time": time.time(), "pid": os.getpid(), "version": VERSION})
            if not _busy and os.path.isdir(JOBS):
                for d in sorted(os.listdir(JOBS)):
                    folder = os.path.join(JOBS, d)
                    if os.path.isfile(os.path.join(folder, "job.json")) and not os.path.exists(os.path.join(folder, "taken")):
                        with open(os.path.join(folder, "taken"), "w") as f:
                            f.write(str(time.time()))
                        _busy = True
                        _app.fireCustomEvent(EVENT_ID, json.dumps({"dir": folder}))
                        break
        except Exception:
            pass
        _stop.wait(1.0)


def run(context):
    global _app, _event, _stop
    try:
        _app = adsk.core.Application.get()
        os.makedirs(JOBS, exist_ok=True)
        _event = _app.registerCustomEvent(EVENT_ID)
        handler = _JobHandler()
        _event.add(handler)
        _handlers.append(handler)
        _stop = threading.Event()
        threading.Thread(target=_poll, daemon=True).start()
    except Exception:
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
