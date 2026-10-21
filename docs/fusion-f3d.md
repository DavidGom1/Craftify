# Notes on Fusion's .f3d, for the offline reader

What `src/fusion/f3d.rs` and `src/fusion/sketch.rs` read from the Design segment
(`FusionDesignSegmentType1/MetaStream.dat` + `BulkStream.dat`), and how each rule was checked.
Record types are named by the first block of their GUID; ids are record (entity) ids.

Checked against `tools/craftify_probe` dumps of four designs: *Tarjetas para picar* (3
components, 46 bodies, 3 sketches), *Carteleria Electromagnetismo* (15 components, 203 bodies,
39 sketches), *Ikea Lack table joiner* and *monstera-earrings* (1 body each).

## Components

* A component record is type `E03784ED`, just before its naming space (base type
  `21F379C8`). Older files were found by the text "Part Number"; most records lack it.
* Its name: the length-prefixed UTF-16 string right after the record's last UUID string.

## Bodies

* Body maps (`74CA4562`) give (selector, entity suffix) pairs; the body belongs to the
  component whose record id is the nearest below the suffix.
* Cached box: records suffix+1..3, a byte 1 and six f64 (cm). Values such as -1e-14 are
  rounding noise, not garbage.
* Name, in the record at the suffix (`D3937028`): u32 number 16 bytes after the first UUID
  (default name "Cuerpo<n>", Fusion adds the word in its UI language); after the fourth UUID,
  4 bytes, the appearance's name, 23 bytes and the user's own name (empty for the default).
  48/48 names in three designs; 203/203 in Carteleria, whose body maps also list 107 bodies
  Fusion no longer shows (consumed ones?).

## Placements

* Occurrence placements (`CE2913AA`) may hold no matrix. Older files: the transform cache
  (`549FBB80`) of the transform object at placement id + 3. Newer files: a transform cache that
  names the placement id itself. Carteleria still has 11 placements with neither; one placement
  is named by two caches (the last one read wins).

## Sketches

* Sketch `8DA771B7`. Default name: after the bytes `ff ff ff ff`, UTF-16 word + u32 number
  ("Boceto" + 4). A name the user gave it is in its container (below), in the 70 bytes after
  the reference to the sketch. 42/42 names.
* Container `44A64366`: points back at exactly one sketch. Every curve and point points at its
  sketch's container with the last flagged reference (a byte 1, then the u64 id) of its record.
  Sketches also point at other sketches' containers (projections), so do not go sketch ->
  container.
* Component: the sketch's last flagged reference to a feature list (`3A6D1E62`), then the
  component by id range. By the sketch's own id it is wrong for sketches made later in the root.
* Plane: a rigid 4x4 (cm) in a `7B6C3C2D` record the sketch points at; none on the origin's XY
  plane (identity).
* Point `C2CEDAE7`: x, y, z (cm) at byte 141, in sketch coordinates. 1884/1884.
* Line `DCA267ED`: point ids at 282 and 293; construction when byte 310 is 1. 1676/1676 lines
  and 48/48 construction flags in two designs. (Fixed: 305; projected/reference: 307.)
* Arc/circle `F0130424`: radius at 257 (cm), sweep at 273 (2π: a circle); centre point id at
  282, end at 293, start at 304 (counter-clockwise start -> end).
* Spline `D82E012F`: degree (u32), tolerance (f64), then knots, weights and control points
  (x, y, z), each array after its count twice and the u32 8. All seen are cubic with 4 control
  points (Béziers); some have two equal control points in a row.
* Not read yet: sketch texts (with a frame of 4 lines), elliptical arcs, splines that are not
  single cubic spans, and projected edges kept in other record types (`00D157C2`, `16DEFC4D`?).

In SolidCraft the sketches match Fusion's curve and profile counts (49/30/98 in Tarjetas; 25 of
39 in Carteleria, the rest miss only what is not read yet).
