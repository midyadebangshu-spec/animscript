//! A small deterministic 3D rigid-body engine: spheres, capsules, convex hulls (boxes are hulls),
//! static planes and triangle meshes, sequential-impulse contacts with friction, sleeping islands,
//! and ball, hinge, fixed and distance joints. No dependencies; `f64`; single threaded; every
//! iteration order is fixed, so the same inputs give bit-identical results.
//!
//! Units are metres, kilograms, seconds and radians. Axes follow the language: x right, y down,
//! z away from the viewer, so gravity is `(0, +9.8, 0)`.
#![allow(clippy::needless_range_loop, clippy::should_implement_trait)]
mod collide;
mod fluid;
mod hull;
mod joint;
mod math;
mod par;
mod shape;
mod soft;
mod tet;
mod world;

pub use collide::{collide, Manifold};
pub use fluid::{Fluid3, FluidParams3, Foam3};
pub use hull::{Edge, Face, Hull, MassProps};
pub use joint::{Joint, JointDef, JointKind};
pub use math::{v3, Mat3, Quat, V3};
pub use shape::{MassData, Shape, WShape};
pub use soft::{
    coupled_step3, is_closed, soft_step, Pin, Point, SoftBody3, SoftParams, SurfaceSpec,
};
pub use tet::{point_in_mesh, tetrahedralize, Tets};
pub use world::{
    hull_centered, Body, BodyDef, BodyId, BodyKind, ContactEvent, ContactInfo, JointId, SoftHit,
    World,
};
