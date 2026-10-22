//! A component's timeline (see [`super::timeline`]) as SolidCraft commands, so the part keeps
//! its features instead of being one imported body.
//!
//! What Fusion selects by its topological naming is found again from the finished part, whose
//! exact geometry is known (the STEP file of the part):
//!
//! * A fillet's edges: each cylindrical face of the fillet's radius lies between two flat faces
//!   it is tangent to; the edge it replaced is where those two planes meet.
//! * Faces extruded right after a cut of a sketch's profiles are the floors that cut left: the
//!   same profiles, starting where the cut ended.
//!
//! The caller checks the result against the part as imported and keeps the import when they
//! differ.

use std::collections::HashMap;

use serde_json::json;

use super::sketch::{self, Sketch};
use super::timeline::{Feature, Operation, Param, Profile};
use crate::solidcraft::script::Script;
use crate::step::fix::{edges_of, face_points};
use crate::step::{Step, V, cross, dot, mean, norm, sub, unit};

/// The commands that make the features again (to run in a new design), and how many features.
pub struct Rebuilt {
    pub script: Script,
    pub features: usize,
}

/// A length expression without its sign ("-0.4 mm" -> "0.4 mm").
fn unsigned(p: &Param) -> String {
    p.expression.trim().trim_start_matches('-').trim().to_owned()
}

fn box_command(size: &[Param; 3], offset: [f64; 2], m: &[f64; 16]) -> Result<serde_json::Value, String> {
    let rot = |r: usize, c: usize| m[4 * r + c];
    // only boxes whose axes are the component's axes, in some order
    let mut axis_of_world = [0usize; 3];
    for (w, slot) in axis_of_world.iter_mut().enumerate() {
        let hits: Vec<usize> = (0..3).filter(|&c| rot(w, c).abs() > 1.0 - 1e-9).collect();
        if hits.len() != 1 || (0..3).any(|c| c != hits[0] && rot(w, c).abs() > 1e-9) {
            return Err("caja girada".into());
        }
        *slot = hits[0];
    }
    let local_min = [offset[0], offset[1], 0.0];
    let local_max = [offset[0] + size[0].value, offset[1] + size[1].value, size[2].value];
    let mut corner = [0.0; 3];
    for (w, c) in corner.iter_mut().enumerate() {
        let k = axis_of_world[w];
        let (a, b) = (rot(w, k) * local_min[k], rot(w, k) * local_max[k]);
        *c = a.min(b) + m[4 * w + 3];
    }
    let dim = |w: usize| {
        let p = &size[axis_of_world[w]];
        if p.value > 0.0 { p.expression.clone() } else { format!("{} mm", p.value.abs()) }
    };
    Ok(json!({"length": dim(0), "width": dim(1), "height": dim(2), "corner": corner}))
}

/// Points on the edges a fillet of this radius replaced, from the finished part.
pub fn fillet_edges(step: &Step, radius: f64) -> Vec<V> {
    let faces: Vec<i64> = step.ent.iter().filter(|(_, e)| e.ty == "ADVANCED_FACE").map(|(i, _)| *i).collect();
    let mut faces_of_edge: HashMap<i64, Vec<i64>> = HashMap::new();
    for &f in &faces {
        if let Ok(edges) = edges_of(step, &step.ent[&f]) {
            for e in edges {
                faces_of_edge.entry(e).or_default().push(f);
            }
        }
    }
    let plane = |f: i64| -> Option<(V, V)> {
        let surf = step.surface(&step.ent[&f]).ok()??;
        if surf.ty != "PLANE" {
            return None;
        }
        let (o, _, _, z) = step.placement(surf.a(1).ok()?.r().ok()?).ok()?;
        Some((o, z))
    };
    let mut out: Vec<V> = Vec::new();
    for &f in &faces {
        let face = &step.ent[&f];
        let Some(surf) = step.surface(face).ok().flatten() else { continue };
        if surf.ty != "CYLINDRICAL_SURFACE" || surf.a(2).and_then(|a| a.f()).map_or(true, |r| (r - radius).abs() > 1e-6) {
            continue;
        }
        let Some(Ok((_, _, _, axis))) = surf.a(1).ok().and_then(|a| a.r().ok()).map(|p| step.placement(p)) else { continue };
        // the flat faces along it: next to it and parallel to its axis
        let mut planes: Vec<(V, V)> = Vec::new();
        for e in edges_of(step, face).unwrap_or_default() {
            for &g in faces_of_edge.get(&e).map(Vec::as_slice).unwrap_or(&[]) {
                if let Some((o, n)) = plane(g).filter(|(_, n)| dot(*n, axis).abs() < 1e-6)
                    && !planes.iter().any(|(_, m)| norm(cross(*m, n)) < 1e-6)
                {
                    planes.push((o, n));
                }
            }
        }
        let [(o1, n1), (o2, n2)] = planes[..] else { continue };
        let Some(c) = mean(&face_points(step, face).unwrap_or_default()) else { continue };
        // the line where the two planes meet, at the face's middle along it
        let Some(d) = unit(cross(n1, n2)) else { continue };
        let Some(p) = solve([n1, n2, d], [dot(n1, o1), dot(n2, o2), dot(d, c)]) else { continue };
        if !out.iter().any(|q| norm(sub(*q, p)) < 1e-6) {
            out.push(p);
        }
    }
    out
}

/// The point x with rows · x = rhs (Cramer's rule); None when the rows are dependent.
fn solve(rows: [V; 3], rhs: [f64; 3]) -> Option<V> {
    let det = dot(rows[0], cross(rows[1], rows[2]));
    if det.abs() < 1e-12 {
        return None;
    }
    let col = |k: usize| [rows[0][k], rows[1][k], rows[2][k]];
    let (c0, c1, c2) = (col(0), col(1), col(2));
    let d = |a: V, b: V, c: V| dot(a, cross(b, c));
    Some([d(rhs, c1, c2) / det, d(c0, rhs, c2) / det, d(c0, c1, rhs) / det].map(|x| (x * 1e9).round() / 1e9))
}

/// The profiles as SolidCraft picks them: the curves of each outer loop.
fn pick(profiles: &[Profile], ids: &HashMap<u64, String>) -> Result<Vec<serde_json::Value>, String> {
    profiles
        .iter()
        .map(|p| {
            let curves: Option<Vec<&String>> = p.outer.iter().map(|k| ids.get(k)).collect();
            curves.map(|c| json!({ "curves": c })).ok_or_else(|| "un perfil con curvas que aún no se leen".to_owned())
        })
        .collect()
}

/// The commands for a timeline; Err says what could not be made.
pub fn rebuild(timeline: &[Feature], sketches: &[Sketch], step: &Step) -> Result<Rebuilt, String> {
    let mut script = Script::default();
    let mut ids: HashMap<u64, String> = HashMap::new();
    let mut sketch_name = None;
    // the last cut or new body from profiles: (profiles, its distance)
    let mut last: Option<(Vec<serde_json::Value>, Param)> = None;
    let mut features = 0;
    for f in timeline {
        match f {
            Feature::Box { size, offset, placement } => {
                let mut params = box_command(size, *offset, placement)?;
                params["operation"] = json!("new");
                script.add("solid.box", params);
            }
            Feature::Fillet { radius } => {
                let edges = fillet_edges(step, radius.value);
                if edges.is_empty() {
                    return Err(format!("no se encuentran las aristas del empalme de {}", radius.expression));
                }
                script.add("solid.fillet", json!({"edges": edges, "radius": radius.expression}));
            }
            Feature::Sketch { id } => {
                let sk = sketches.iter().find(|s| s.id == *id).ok_or("un boceto que no se ha podido leer")?;
                if sk.skipped > 0 {
                    return Err(format!("{}: {} curvas que aún no se leen", sk.name, sk.skipped));
                }
                sketch::add_to(&mut script, sk);
                ids = sk.keys.iter().copied().zip(sketch::curve_ids(sk)).collect();
                sketch_name = Some(sk.name.clone());
                last = None;
            }
            Feature::Extrude { operation, distance, profiles } => {
                let sketch = sketch_name.clone().ok_or("una extrusión sin boceto")?;
                let operation = match operation {
                    Operation::Cut => "cut",
                    Operation::NewBody => "new",
                };
                let direction = if distance.value < 0.0 { "negative" } else { "positive" };
                let mut params = json!({"sketch": sketch, "distance": unsigned(distance), "direction": direction, "operation": operation});
                match profiles {
                    Some(p) => {
                        let picked = pick(p, &ids)?;
                        params["profiles"] = json!(picked);
                        last = Some((picked, distance.clone()));
                    }
                    // faces: the floors the previous cut of these profiles left
                    None => {
                        let (picked, before) = last.clone().ok_or("una extrusión de caras que no siguen a un corte")?;
                        params["profiles"] = json!(picked);
                        params["start_offset"] = json!(before.expression);
                    }
                }
                script.add("solid.extrude", params);
            }
            Feature::Other { kind, name } => {
                return Err(format!("{} aún no se reproduce", if name.is_empty() { kind } else { name }));
            }
        }
        features += 1;
    }
    Ok(Rebuilt { script, features })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(name: &str, expression: &str, value: f64) -> Param {
        Param { name: name.into(), expression: expression.into(), value }
    }

    #[test]
    fn a_box_turned_a_quarter_round_z() {
        // Tarjetas para picar: the box's X runs along the component's Y
        let m = [0.0, -1.0, 0.0, 53.98, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];
        let size = [p("DistanceX", "85.6 mm", 85.6), p("DistanceY", "53.98 mm", 53.98), p("DistanceZ", "0.8 mm", 0.8)];
        let v = box_command(&size, [0.0, 0.0], &m).expect("a box");
        assert_eq!(v["length"], "53.98 mm");
        assert_eq!(v["width"], "85.6 mm");
        assert_eq!(v["height"], "0.8 mm");
        let c: Vec<f64> = v["corner"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
        assert!(c.iter().all(|x| x.abs() < 1e-9));
    }

    #[test]
    fn a_tilted_box_is_not_made() {
        let s = std::f64::consts::FRAC_1_SQRT_2;
        let m = [s, -s, 0.0, 0.0, s, s, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];
        let size = [p("DistanceX", "1 mm", 1.0), p("DistanceY", "1 mm", 1.0), p("DistanceZ", "1 mm", 1.0)];
        assert!(box_command(&size, [0.0, 0.0], &m).is_err());
    }

    #[test]
    fn two_planes_meet_on_a_line() {
        let x = solve([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], [53.98, 0.0, 0.4]).unwrap();
        assert_eq!(x, [53.98, 0.0, 0.4]);
    }
}
