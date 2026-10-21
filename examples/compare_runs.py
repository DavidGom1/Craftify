"""Compare a Python and a Rust conversion of the same sources, folder by folder.

usage: python compare_runs.py <python out folder> <rust out folder> [design name ...]

For each converted design: the report (parts, repairs, volumes, checks, warnings), every STEP
file the conversion wrote (line endings aside) and the SolidCraft scripts (as JSON values).
"""
import json
import sys
from pathlib import Path

PART_FIELDS = ("ok", "volume_mm3", "bodies", "repair", "source_volume_mm3", "matches_source", "warnings")
py_root, rs_root = Path(sys.argv[1]), Path(sys.argv[2])
only = set(sys.argv[3:])
bad = 0
for py_dir in sorted(p for p in py_root.iterdir() if (p / "report.json").is_file() and (not only or p.name in only)):
    rs_dir = rs_root / py_dir.name
    a = json.loads((py_dir / "report.json").read_text(encoding="utf-8"))
    b = json.loads((rs_dir / "report.json").read_text(encoding="utf-8"))
    print(f"== {py_dir.name}: py {a['placed']}/{a['instances']} placed, rs {b['placed']}/{b['instances']}; "
          f"py {a['seconds']} s, rs {b['seconds']} s")
    for k in ("output", "instances", "placed", "warnings"):
        if (a[k] is None) != (b[k] is None) if k == "output" else a[k] != b[k]:
            print(f"   DIFF report {k}: {a[k]!r} vs {b[k]!r}")
            bad += 1
    pb = {p["stem"]: p for p in b["parts"]}
    if [p["stem"] for p in a["parts"]] != [p["stem"] for p in b["parts"]]:
        print("   DIFF part list:", [p["stem"] for p in a["parts"]], [p["stem"] for p in b["parts"]])
        bad += 1
    for p in a["parts"]:
        q = pb.get(p["stem"], {})
        diffs = [f for f in PART_FIELDS if p.get(f) != q.get(f)]
        print(f"   {p['stem'][:34]:34} {str(p['repair'])[:26]:26} {'IGUAL' if not diffs else 'DIFF ' + str(diffs)}")
        for f in diffs:
            print(f"      {f}: {p.get(f)!r}\n      {'':{len(f)}}  {q.get(f)!r}")
        bad += bool(diffs)
    for f in sorted((py_dir / "build").glob("*.step")):
        g = rs_dir / "build" / f.name
        same = g.is_file() and f.read_bytes().replace(b"\r\n", b"\n") == g.read_bytes().replace(b"\r\n", b"\n")
        if not same:
            print(f"   DIFF {f.name}")
            bad += 1
    for f in sorted((py_dir / "build").glob("*.json")):
        if f.name.endswith((".results.json", ".eval.json")):
            continue
        g = rs_dir / "build" / f.name
        # the two output folders differ: compare the scripts with Rust's folder in place of Python's
        mine = f.read_text(encoding="utf-8").replace(json.dumps(str(py_root.resolve()))[1:-1], json.dumps(str(rs_root.resolve()))[1:-1])
        if not g.is_file() or json.loads(mine) != json.loads(g.read_text(encoding="utf-8")):
            print(f"   DIFF {f.name}")
            bad += 1
    print(f"   {len(list((py_dir / 'build').glob('*.step')))} STEP files and the scripts compared")
print("TODO IGUAL" if not bad else f"{bad} diferencias")
