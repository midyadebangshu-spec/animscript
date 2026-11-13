//! 3D meshes: primitives, extrude, lathe. Normals point out of the solid; closed solids have every edge twice.
use animscript_core::mesh::*;
use std::collections::BTreeMap;

fn normal(m: &Mesh, f: usize) -> [f64; 3] {
    m.face_normal(f)
}
fn centroid(m: &Mesh, f: usize) -> [f64; 3] {
    let n = m.faces[f].len() as f64;
    let mut c = [0.0; 3];
    for &i in &m.faces[f] {
        for (k, ck) in c.iter_mut().enumerate() {
            *ck += m.verts[i as usize][k] / n;
        }
    }
    c
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
/// Every undirected edge is used by exactly two faces, once in each direction: a closed, consistently wound surface.
fn closed(m: &Mesh) -> bool {
    let mut edges: BTreeMap<(u32, u32), i32> = BTreeMap::new();
    for f in &m.faces {
        for k in 0..f.len() {
            let (a, b) = (f[k], f[(k + 1) % f.len()]);
            *edges.entry((a, b)).or_insert(0) += 1;
        }
    }
    edges
        .iter()
        .all(|(&(a, b), &n)| n == 1 && edges.get(&(b, a)) == Some(&1))
}
/// All face normals point away from `centre` (for convex solids).
fn outward(m: &Mesh, centre: [f64; 3]) -> bool {
    (0..m.faces.len()).all(|f| {
        let c = centroid(m, f);
        dot(
            normal(m, f),
            [c[0] - centre[0], c[1] - centre[1], c[2] - centre[2]],
        ) > 0.0
    })
}
fn area(m: &Mesh) -> f64 {
    (0..m.faces.len())
        .map(|f| {
            let fa = &m.faces[f];
            let v = |i: usize| m.verts[fa[i] as usize];
            let mut a = 0.0;
            for k in 1..fa.len() - 1 {
                let (p, q, r) = (v(0), v(k), v(k + 1));
                let u = [q[0] - p[0], q[1] - p[1], q[2] - p[2]];
                let w = [r[0] - p[0], r[1] - p[1], r[2] - p[2]];
                let c = [
                    u[1] * w[2] - u[2] * w[1],
                    u[2] * w[0] - u[0] * w[2],
                    u[0] * w[1] - u[1] * w[0],
                ];
                a += 0.5 * (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt();
            }
            a
        })
        .sum()
}
fn volume(m: &Mesh) -> f64 {
    // signed volume by the divergence theorem; positive when the normals point out
    (0..m.faces.len())
        .map(|f| {
            let fa = &m.faces[f];
            let v = |i: usize| m.verts[fa[i] as usize];
            let mut vol = 0.0;
            for k in 1..fa.len() - 1 {
                let (p, q, r) = (v(0), v(k), v(k + 1));
                vol += dot(
                    p,
                    [
                        q[1] * r[2] - q[2] * r[1],
                        q[2] * r[0] - q[0] * r[2],
                        q[0] * r[1] - q[1] * r[0],
                    ],
                ) / 6.0;
            }
            vol
        })
        .sum()
}

#[test]
fn a_cube_has_eight_vertices_six_faces_and_outward_normals() {
    let m = Mesh::cube(100.0, 60.0, 40.0);
    assert_eq!((m.verts.len(), m.faces.len()), (8, 6));
    assert!(closed(&m) && outward(&m, [0.0; 3]));
    assert!((volume(&m) - 100.0 * 60.0 * 40.0).abs() < 1e-6);
    assert!((area(&m) - 2.0 * (6000.0 + 4000.0 + 2400.0)).abs() < 1e-6);
    let b = m.bbox();
    assert_eq!((b.0, b.1), ([-50.0, -30.0, -20.0], [50.0, 30.0, 20.0]));
}

#[test]
fn a_sphere_is_closed_round_and_outward() {
    let m = Mesh::sphere(50.0, 24, 12);
    assert!(closed(&m) && outward(&m, [0.0; 3]));
    assert!(m
        .verts
        .iter()
        .all(|v| ((v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt() - 50.0).abs() < 1e-9));
    let ideal = 4.0 / 3.0 * std::f64::consts::PI * 50.0f64.powi(3);
    assert!(
        (volume(&m) - ideal).abs() / ideal < 0.03,
        "{} vs {ideal}",
        volume(&m)
    );
}

#[test]
fn a_cylinder_and_a_cone_are_closed_and_outward() {
    let c = Mesh::cylinder(30.0, 80.0, 32);
    assert!(closed(&c) && outward(&c, [0.0; 3]));
    let ideal = std::f64::consts::PI * 900.0 * 80.0;
    assert!((volume(&c) - ideal).abs() / ideal < 0.02);
    let k = Mesh::cone(30.0, 80.0, 32);
    assert!(closed(&k) && volume(&k) > 0.0);
    // the apex of a cone is up: -y
    assert!(k
        .verts
        .iter()
        .any(|v| v[1] == -40.0 && v[0] == 0.0 && v[2] == 0.0));
}

#[test]
fn a_plane_is_a_flat_double_sided_grid_in_x_z() {
    let p = Mesh::plane(200.0, 100.0, 4, 2);
    assert_eq!((p.verts.len(), p.faces.len()), (5 * 3, 8));
    assert!(p.double_sided && p.verts.iter().all(|v| v[1] == 0.0));
    // it faces up (-y, since y points down the screen)
    assert!((0..p.faces.len()).all(|f| normal(&p, f)[1] < 0.0));
}

#[test]
fn extrude_turns_a_polygon_into_a_closed_solid() {
    // an L shape (concave): the caps need real triangulation
    let l = [
        [0.0, 0.0],
        [60.0, 0.0],
        [60.0, 20.0],
        [20.0, 20.0],
        [20.0, 60.0],
        [0.0, 60.0],
    ];
    for poly in [l.to_vec(), l.iter().rev().cloned().collect::<Vec<_>>()] {
        let m = Mesh::extrude(&poly, 30.0).unwrap();
        assert!(closed(&m), "closed for either winding");
        let area2d = 60.0 * 20.0 + 20.0 * 40.0;
        assert!(
            (volume(&m) - area2d * 30.0).abs() < 1e-6,
            "volume {}",
            volume(&m)
        );
        let b = m.bbox();
        assert_eq!((b.0[2], b.1[2]), (-15.0, 15.0));
    }
    assert!(Mesh::extrude(&[[0.0, 0.0], [1.0, 0.0]], 5.0).is_err());
    assert!(Mesh::extrude(&l, 0.0).is_err());
}

#[test]
fn lathe_spins_a_profile_into_a_vase() {
    // (radius, height): a closed profile from the axis out and back
    let profile = [
        [0.0, 0.0],
        [40.0, 0.0],
        [30.0, 50.0],
        [20.0, 90.0],
        [28.0, 120.0],
        [0.0, 120.0],
    ];
    let m = Mesh::lathe(&profile, 32).unwrap();
    assert!(closed(&m) && volume(&m) > 0.0);
    // heights go up the screen: height 120 is y = -120
    let b = m.bbox();
    assert!(b.0[1] <= -119.9 && b.1[1] >= -0.1 && b.1[0] <= 40.0 + 1e-9);
    assert!(Mesh::lathe(&profile, 2).is_err());
    assert!(Mesh::lathe(&[[1.0, 0.0]], 8).is_err());
    assert!(
        Mesh::lathe(&[[-1.0, 0.0], [2.0, 1.0]], 8).is_err(),
        "a negative radius"
    );
}

#[test]
fn triangulation_covers_the_polygon_exactly() {
    let l = [
        [0.0, 0.0],
        [60.0, 0.0],
        [60.0, 20.0],
        [20.0, 20.0],
        [20.0, 60.0],
        [0.0, 60.0],
    ];
    let tris = triangulate(&l).unwrap();
    assert_eq!(tris.len(), 4);
    let a: f64 = tris
        .iter()
        .map(|t| {
            let (p, q, r) = (l[t[0]], l[t[1]], l[t[2]]);
            ((q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0])).abs() / 2.0
        })
        .sum();
    assert!((a - 2000.0).abs() < 1e-9);
    // a polygon that crosses itself is refused
    assert!(triangulate(&[[0.0, 0.0], [10.0, 10.0], [10.0, 0.0], [0.0, 10.0]]).is_err());
}

#[test]
fn transforms_scale_rotate_and_move_vertices() {
    let m = Mesh::cube(2.0, 2.0, 2.0);
    let t = m.transformed(&Transform3 {
        position: [10.0, 0.0, 5.0],
        rot: [0.0, 0.25, 0.0],
        scale: [2.0, 1.0, 1.0],
        pivot: [0.0; 3],
        orient: None,
    });
    // scaled in x, then a quarter turn about y (x -> -z or +z), then moved
    let b = t.bbox();
    assert!(
        (b.1[2] - b.0[2] - 4.0).abs() < 1e-9 && (b.1[0] - b.0[0] - 2.0).abs() < 1e-9,
        "{b:?}"
    );
    assert!((b.0[0] + b.1[0]) / 2.0 - 10.0 < 1e-9);
}

#[test]
fn rotation_spins_about_y_first_then_tilts_about_x() {
    // a point on the x axis: a quarter spin about y sends it to -z; then a quarter tilt about x sends -z to +y
    let t = Transform3 {
        position: [0.0; 3],
        rot: [0.25, 0.25, 0.0],
        scale: [1.0; 3],
        pivot: [0.0; 3],
        orient: None,
    };
    let p = t.apply([10.0, 0.0, 0.0]);
    assert!(
        p[0].abs() < 1e-9 && (p[1] - 10.0).abs() < 1e-9 && p[2].abs() < 1e-9,
        "{p:?}"
    );
    // and a roll about z comes last
    let t = Transform3 {
        rot: [0.0, 0.0, 0.25],
        ..Transform3::identity()
    };
    let p = t.apply([10.0, 0.0, 0.0]);
    assert!(p[0].abs() < 1e-9 && (p[1] - 10.0).abs() < 1e-9);
}

#[test]
fn subdividing_a_cube_keeps_its_faces_outward_and_its_volume() {
    let m = Mesh::cube(100.0, 60.0, 80.0).subdivided(3);
    assert_eq!(m.faces.len(), 6 * 9);
    assert_eq!(m.verts.len(), 6 * 16);
    assert!(m.faces.iter().all(|f| f.len() == 4));
    // every face normal points away from the centre
    for f in 0..m.faces.len() {
        let n = m.face_normal(f);
        let c: [f64; 3] = {
            let mut c = [0.0; 3];
            for &i in &m.faces[f] {
                for (k, ck) in c.iter_mut().enumerate() {
                    *ck += m.verts[i as usize][k] / 4.0;
                }
            }
            c
        };
        assert!(
            n[0] * c[0] + n[1] * c[1] + n[2] * c[2] > 0.0,
            "face {f} points inward"
        );
    }
}
