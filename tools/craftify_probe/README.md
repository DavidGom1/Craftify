# CraftifyProbe

A research add-in for Fusion, not part of Craftify: it dumps what Fusion's API knows about a
design (components, bodies and their names, timeline, sketches with every curve, features,
parameters) so the offline reader can be checked against it. It only reads: each design is
imported into a new document and closed without saving.

## Install and run

1. Copy this folder to `%APPDATA%\Autodesk\Autodesk Fusion 360\API\AddIns\CraftifyProbe`.
2. In Fusion: Shift+S (Utilities > Add-ins), tab Add-ins, CraftifyProbe, Run.
3. For each design, make a folder `work\jobs\<name>` next to `CraftifyProbe.py` with
   `job.json`: `{"source": "D:\\path\\to\\design.f3d"}` (write it with a JSON library: the
   backslashes must be escaped).
4. The add-in writes `probe.json` in that folder (`ok`, `components`, `timeline`, `parameters`).

The job folder must be next to the add-in: Fusion sees its own copy of `%LOCALAPPDATA%`, so jobs
left there are not found. `work/alive.json` is rewritten every second while the add-in runs;
`work/poll_error.txt` holds any error of its loop.
