//! A small deterministic 2D rigid-body engine in the style of Box2D (soft-step solver, speculative
//! contacts, sleeping islands) with joints and springs. No dependencies; `f64`; single threaded;
//! every iteration order is fixed, so the same inputs always give bit-identical results.
//!
//! Units are metres, kilograms, seconds and radians. The y axis points down (screen space), so
//! gravity is `(0, +9.8)`. The language layer converts pixels with a scale (100 px/m by default).
mod collide;
mod joint;
mod math;
mod particles;
mod shape;
mod soft;
mod world;

pub use collide::{collide, Manifold, ManifoldPoint};
pub use joint::{Joint, JointDef, JointKind};
pub use math::{cross_sv, v2, Rot, Vec2, Xf};
pub use particles::{Emitter, EmitterDef, Particle};
pub use shape::{decompose, find_pairs, Aabb, MassData, Polygon, Shape};
pub use soft::{soft_step, SoftBody, SoftParams};
pub use world::{
    Body, BodyDef, BodyId, BodyKind, ContactEvent, ContactInfo, FixtureDef, JointId, World,
};

/// The boundary a 3D backend (a Jolt wrapper) will implement later. The language runtime only
/// needs stepping and contact events from a world; poses and creation stay dimension-specific.
pub trait PhysicsWorld {
    fn step(&mut self, dt: f64);
    fn body_count(&self) -> usize;
    fn take_events(&mut self) -> Vec<ContactEvent>;
}

impl PhysicsWorld for World {
    fn step(&mut self, dt: f64) {
        World::step(self, dt)
    }
    fn body_count(&self) -> usize {
        World::body_count(self)
    }
    fn take_events(&mut self) -> Vec<ContactEvent> {
        World::take_events(self)
    }
}
