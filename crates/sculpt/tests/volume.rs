//! The sparse volume and its surface.
use animscript_sculpt::*;
use std::collections::BTreeSet;

fn sphere_vol(h: f64, r: f64, c: V3) -> Volume {
    let mut v = Volume::new(h, 3.0);
    let (lo, hi) = v.index_box(
        [c[0] - r - 4.0 * h, c[1] - r - 4.0 * h, c[2] - r - 4.0 * h],
        [c[0] + r + 4.0 * h, c[1] + r + 4.0 * h, c[2] + r + 4.0 * h],
    );
    v.edit(lo, hi, &mut |p, _| len(sub(p, c)) - r);
    v
}

#[test]
fn an_empty_volume_is_outside_everywhere() {
    let v = Volume::new(1.0, 3.0);
    assert_eq!(v.get([5, -3, 100]), 3.0);
    assert_eq!(v.block_count(), 0);
    assert!(v.mesh().tris.is_empty());
    assert_eq!(v.volume(), 0.0);
}

#[test]
fn set_get_and_the_band() {
    let mut v = Volume::new(0.5, 3.0); // band 1.5
    v.set([2, 3, 4], -0.25);
    assert_eq!(v.get([2, 3, 4]), -0.25);
    assert_eq!(v.get([2, 3, 5]), 1.5);
    v.set([-9, -9, -9], -100.0);
    assert_eq!(v.get([-9, -9, -9]), -1.5, "values are clamped to the band");
    // negative indices land in the right blocks
    v.set([-1, -1, -1], 0.5);
    assert_eq!(v.get([-1, -1, -1]), 0.5);
    assert_eq!(v.get([-2, -1, -1]), 1.5);
}

#[test]
fn blocks_that_become_uniform_collapse_and_empty_ones_vanish() {
    let mut v = Volume::new(1.0, 3.0);
    v.edit([0, 0, 0], [7, 7, 7], &mut |_, _| -3.0);
    assert_eq!((v.block_count(), v.dense_block_count()), (1, 0));
    assert_eq!(v.get([3, 3, 3]), -3.0);
    v.edit([0, 0, 0], [7, 7, 7], &mut |_, _| 3.0);
    assert_eq!(v.block_count(), 0);
    v.edit([0, 0, 0], [7, 7, 7], &mut |p, _| p[0] - 3.5);
    assert_eq!(v.dense_block_count(), 1);
}

#[test]
fn sampling_is_exact_on_a_linear_field_and_the_gradient_is_its_slope() {
    let mut v = Volume::new(1.0, 20.0);
    v.edit([-12, -12, -12], [12, 12, 12], &mut |p, _| {
        0.5 * p[0] - 0.25 * p[1] + 0.125 * p[2]
    });
    for p in [[0.3, -1.7, 2.2], [5.5, 5.5, -3.1], [-4.0, 0.0, 0.0]] {
        let exact = 0.5 * p[0] - 0.25 * p[1] + 0.125 * p[2];
        assert!((v.sample(p) - exact).abs() < 1e-5, "{p:?}");
    }
    let g = v.gradient([1.0, 1.0, 1.0]);
    assert!((g[0] - 0.5).abs() < 1e-5 && (g[1] + 0.25).abs() < 1e-5 && (g[2] - 0.125).abs() < 1e-5);
}

#[test]
fn a_sphere_meshes_to_a_closed_outward_surface_with_the_right_size() {
    let (r, c) = (20.0, [0.3, 0.2, -0.4]);
    let v = sphere_vol(1.0, r, c);
    let m = v.mesh();
    assert!(!m.tris.is_empty());
    assert!(m.is_closed(), "a sphere has a closed surface");
    let exact_v = 4.0 / 3.0 * std::f64::consts::PI * r.powi(3);
    let exact_a = 4.0 * std::f64::consts::PI * r * r;
    assert!(
        (m.enclosed_volume() / exact_v - 1.0).abs() < 0.03,
        "{} {}",
        m.enclosed_volume(),
        exact_v
    );
    assert!(
        (m.area() / exact_a - 1.0).abs() < 0.04,
        "{} {}",
        m.area(),
        exact_a
    );
    assert!(
        (v.volume() / exact_v - 1.0).abs() < 0.02,
        "{} {}",
        v.volume(),
        exact_v
    );
    // every vertex is close to the true surface and every triangle faces out
    for p in &m.verts {
        assert!((len(sub(*p, c)) - r).abs() < 0.8, "{p:?}");
    }
    for t in &m.tris {
        let (a, b, cc) = (
            m.verts[t[0] as usize],
            m.verts[t[1] as usize],
            m.verts[t[2] as usize],
        );
        let n = cross(sub(b, a), sub(cc, a));
        let mid = [
            (a[0] + b[0] + cc[0]) / 3.0,
            (a[1] + b[1] + cc[1]) / 3.0,
            (a[2] + b[2] + cc[2]) / 3.0,
        ];
        assert!(dot(n, sub(mid, c)) > 0.0);
    }
}

#[test]
fn a_finer_grid_is_more_accurate() {
    let r: f64 = 10.0;
    let exact = 4.0 / 3.0 * std::f64::consts::PI * r.powi(3);
    let coarse = sphere_vol(2.0, r, [0.0; 3]).mesh().enclosed_volume();
    let fine = sphere_vol(0.5, r, [0.0; 3]).mesh().enclosed_volume();
    assert!((fine - exact).abs() < (coarse - exact).abs());
    assert!((fine / exact - 1.0).abs() < 0.01);
}

#[test]
fn the_volume_is_sparse() {
    let v = sphere_vol(1.0, 48.0, [0.0; 3]);
    // a shell of dense blocks around the surface, deep inside collapsed to uniform blocks
    let box_blocks = (104.0f64 / 8.0).powi(3) as usize;
    assert!(
        v.dense_block_count() < box_blocks / 2,
        "{} of {}",
        v.dense_block_count(),
        box_blocks
    );
    assert!(v.block_count() > v.dense_block_count());
}

#[test]
fn two_spheres_make_two_closed_pieces_and_a_torus_has_a_hole() {
    let mut v = Volume::new(1.0, 3.0);
    v.edit([-30, -12, -12], [30, 12, 12], &mut |p, _| {
        let a = len(sub(p, [-15.0, 0.0, 0.0])) - 8.0;
        let b = len(sub(p, [15.0, 0.0, 0.0])) - 8.0;
        a.min(b)
    });
    let m = v.mesh();
    assert!(m.is_closed());
    // count connected components by union-find over triangles
    let mut parent: Vec<usize> = (0..m.verts.len()).collect();
    fn find(p: &mut [usize], x: usize) -> usize {
        let mut r = x;
        while p[r] != r {
            r = p[r];
        }
        let mut c = x;
        while p[c] != r {
            let n = p[c];
            p[c] = r;
            c = n;
        }
        r
    }
    for t in &m.tris {
        let (a, b) = (
            find(&mut parent, t[0] as usize),
            find(&mut parent, t[1] as usize),
        );
        parent[a] = b;
        let (a, b) = (
            find(&mut parent, t[1] as usize),
            find(&mut parent, t[2] as usize),
        );
        parent[a] = b;
    }
    let used: BTreeSet<u32> = m.tris.iter().flatten().copied().collect();
    let roots: BTreeSet<usize> = used
        .iter()
        .map(|v| find(&mut parent, *v as usize))
        .collect();
    assert_eq!(roots.len(), 2);
    // a torus: Euler characteristic V - E + F = 0
    let mut t = Volume::new(1.0, 3.0);
    t.edit([-22, -22, -10], [22, 22, 10], &mut |p, _| {
        let q = ((p[0] * p[0] + p[1] * p[1]).sqrt() - 14.0, p[2]);
        (q.0 * q.0 + q.1 * q.1).sqrt() - 5.0
    });
    let m = t.mesh();
    assert!(m.is_closed());
    let used: BTreeSet<u32> = m.tris.iter().flatten().copied().collect();
    let euler = used.len() as i64 - (3 * m.tris.len() / 2) as i64 + m.tris.len() as i64;
    assert_eq!(euler, 0, "a torus has genus 1");
}

#[test]
fn meshing_is_deterministic() {
    let a = sphere_vol(1.0, 12.0, [0.1, 0.2, 0.3]).mesh();
    let b = sphere_vol(1.0, 12.0, [0.1, 0.2, 0.3]).mesh();
    assert_eq!(a.tris, b.tris);
    assert!(a
        .verts
        .iter()
        .zip(&b.verts)
        .all(|(p, q)| p.iter().zip(q).all(|(x, y)| x.to_bits() == y.to_bits())));
}

#[test]
fn a_snapshot_holds_the_old_values() {
    let mut v = sphere_vol(1.0, 8.0, [0.0; 3]);
    let snap = v.snapshot([-10, -10, -10], [10, 10, 10]);
    v.edit([-10, -10, -10], [10, 10, 10], &mut |_, _| 3.0);
    assert!(snap.sample([0.0, 0.0, 0.0]) < 0.0);
    assert!(v.sample([0.0, 0.0, 0.0]) > 0.0);
}
