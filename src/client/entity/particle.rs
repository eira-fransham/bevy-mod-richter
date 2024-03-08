// Copyright © 2020 Cormac O'Brien
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

use std::ops::RangeInclusive;

use crate::{
    client::render::ParticleAssets,
    common::{
        engine,
        math::{self, VERTEX_NORMAL_COUNT},
    },
};

use bevy::{
    ecs::system::EntityCommand, hierarchy::BuildWorldChildren as _, prelude::*,
    render::extract_component::ExtractComponent,
};
use cgmath::InnerSpace as _;
use chrono::Duration;
use lazy_static::lazy_static;
use rand::distributions::{Distribution as _, Uniform};

lazy_static! {
    static ref COLOR_RAMP_EXPLOSION_FAST: ColorRamp = ColorRamp {
        ramp: vec![0x6F, 0x6D, 0x6B, 0x69, 0x67, 0x65, 0x63, 0x61],
        fps: 10.0,
    };
    static ref COLOR_RAMP_EXPLOSION_SLOW: ColorRamp = ColorRamp {
        ramp: vec![0x6F, 0x6E, 0x6D, 0x6C, 0x6B, 0x6A, 0x68, 0x66],
        fps: 5.0,
    };
    static ref COLOR_RAMP_FIRE: ColorRamp = ColorRamp {
        ramp: vec![0x6D, 0x6B, 0x06, 0x05, 0x04, 0x03],
        fps: 15.0,
    };
    static ref EXPLOSION_SCATTER_DISTRIBUTION: Uniform<f32> = Uniform::new(-16.0, 16.0);
    static ref EXPLOSION_VELOCITY_DISTRIBUTION: Uniform<f32> = Uniform::new(-256.0, 256.0);
}

// TODO: make max configurable
pub const MIN_PARTICLES: usize = 512;

// should be possible to get the whole particle list in cache at once
pub const MAX_PARTICLES: usize = 16384;

/// An animated color ramp.
///
/// Colors are specified using 8-bit indexed values, which should be translated
/// using the palette.
#[derive(Debug)]
pub struct ColorRamp {
    // TODO: arrayvec, tinyvec, or array once const generics are stable
    ramp: Vec<u8>,

    // frames per second of the animation
    fps: f32,
}

impl ColorRamp {
    /// Returns the frame corresponding to the given time.
    ///
    /// If the animation has already completed by `elapsed`, returns `None`.
    pub fn color(&self, elapsed: Duration, frame_skip: usize) -> Option<u8> {
        let frame = (engine::duration_to_f32(elapsed) * self.fps) as usize + frame_skip;
        self.ramp.get(frame).map(|c| *c)
    }
}

/// Dictates the behavior of a particular particle.
///
/// Particles which are animated with a color ramp are despawned automatically
/// when the animation is complete.
#[derive(Copy, Clone, Debug)]
pub enum ParticleKind {
    /// Normal particle, unaffected by gravity.
    Static,

    /// Normal particle, affected by gravity.
    Grav,

    /// Fire and smoke particles. Animated using `COLOR_RAMP_FIRE`. Inversely
    /// affected by gravity, rising instead of falling.
    Fire {
        /// Specifies the number of frames to skip.
        frame_skip: usize,
    },

    /// Explosion particles. May have `COLOR_RAMP_EXPLOSION_FAST` or
    /// `COLOR_RAMP_EXPLOSION_SLOW`. Affected by gravity.
    Explosion {
        /// Specifies the color ramp to use.
        ramp: &'static ColorRamp,

        /// Specifies the number of frames to skip.
        frame_skip: usize,
    },

    /// Spawn (enemy) death explosion particle. Accelerates at
    /// `v(t2) = v(t1) + 4 * (t2 - t1)`. May or may not have an intrinsic
    /// z-velocity.
    Blob {
        /// If false, particle only moves in the XY plane and is unaffected by
        /// gravity.
        has_z_velocity: bool,
    },
}

/// Factor at which particles are affected by gravity.
pub const PARTICLE_GRAVITY_FACTOR: f32 = 0.05;

/// A live particle.
#[derive(Copy, Clone, Debug, ExtractComponent, Component)]
pub struct Particle {
    pub kind: ParticleKind,
    pub velocity: Vec3,
    pub color: u8,
    pub spawned: Duration,
    pub expire: Duration,
}

pub enum CreateParticle {
    EntityField,
    RandomCloud {
        count: usize,
        colors: RangeInclusive<u8>,
        kind: ParticleKind,
        ttl: Duration,
        scatter_distr: Uniform<f32>,
        velocity_distr: Uniform<f32>,
    },
    Explosion,
    ColorExplosion {
        colors: RangeInclusive<u8>,
    },
    /// Creates a death explosion for the Spawn.
    SpawnExplosion,
    ProjectileImpact {
        direction: Vec3,
        color: u8,
        count: usize,
    },
    /// Creates a lava splash effect.
    LavaSplash,
    /// Creates a teleporter warp effect.
    TeleporterWarp,
    Trail {
        end: Vec3,
        kind: TrailKind,
        sparse: bool,
    },
}

fn random_vector3(velocity_distr: &Uniform<f32>) -> Vec3 {
    let mut rng = rand::thread_rng();
    Vec3::new(
        velocity_distr.sample(&mut rng),
        velocity_distr.sample(&mut rng),
        velocity_distr.sample(&mut rng),
    )
}

fn spawn_random_cloud(
    e: &mut WorldChildBuilder,
    mesh: &Handle<Mesh>,
    materials: &[Handle<StandardMaterial>],
    time: Duration,
    count: usize,
    color_start: usize,
    color_end: usize,
    kind: ParticleKind,
    ttl: Duration,
    scatter_distr: &Uniform<f32>,
    velocity_distr: &Uniform<f32>,
) {
    for i in 0..count {
        let origin = random_vector3(scatter_distr);
        let velocity = random_vector3(velocity_distr);
        let color = (color_start + i % (color_end - color_start + 1)) as u8;
        e.spawn((
            MaterialMeshBundle {
                mesh: mesh.clone(),
                material: materials[color as usize].clone(),
                transform: Transform::from_translation(origin),
                ..default()
            },
            Particle {
                kind,
                velocity,
                color,
                spawned: time,
                expire: time + ttl,
            },
        ));
    }
}

impl EntityCommand for CreateParticle {
    fn apply(self, id: Entity, world: &mut bevy::prelude::World) {
        use CreateParticle::*;

        lazy_static! {
            // angular velocities initialized with (rand() & 255) * 0.01;
            static ref VELOCITY_DISTRIBUTION: Uniform<f32> = Uniform::new(0.0, 2.56);
            static ref ANGLE_VELOCITIES: [Vec3; VERTEX_NORMAL_COUNT] = {
                let mut angle_velocities = [Vec3::ZERO; VERTEX_NORMAL_COUNT];

                for i in 0..angle_velocities.len() {
                    angle_velocities[i] = random_vector3(&VELOCITY_DISTRIBUTION);
                }

                angle_velocities
            };
        }

        let time = Duration::from_std(world.resource::<Time<Virtual>>().elapsed()).unwrap();

        let ParticleAssets {
            mesh, materials, ..
        } = world.resource::<ParticleAssets>().clone();

        world.entity_mut(id).with_children(|e| {
            match self {
                EntityField => {
                    let beam_length = 16.0;
                    let dist = 64.0;

                    for i in 0..VERTEX_NORMAL_COUNT {
                        let float_time = engine::duration_to_f32(time);

                        let angles = float_time * ANGLE_VELOCITIES[i];

                        let sin_yaw = angles[0].sin();
                        let cos_yaw = angles[0].cos();
                        let sin_pitch = angles[1].sin();
                        let cos_pitch = angles[1].cos();

                        let forward =
                            Vec3::new(cos_pitch * cos_yaw, cos_pitch * sin_yaw, -sin_pitch);
                        let ttl = Duration::try_milliseconds(10).unwrap();

                        let vert_normal = math::VERTEX_NORMALS[i];
                        let vert_normal = Vec3::new(vert_normal.x, vert_normal.y, vert_normal.z);

                        let origin = dist * vert_normal + beam_length * forward;

                        e.spawn((
                            Transform::from_translation(origin),
                            Particle {
                                kind: ParticleKind::Explosion {
                                    ramp: &COLOR_RAMP_EXPLOSION_FAST,
                                    frame_skip: 0,
                                },
                                velocity: Vec3::ZERO,
                                color: COLOR_RAMP_EXPLOSION_FAST.ramp[0],
                                spawned: time,
                                expire: time + ttl,
                            },
                        ));
                    }
                }
                RandomCloud {
                    count,
                    colors,
                    kind,
                    ttl,
                    scatter_distr,
                    velocity_distr,
                } => {
                    let color_start = *colors.start() as usize;
                    let color_end = *colors.end() as usize;
                    spawn_random_cloud(
                        e,
                        &mesh,
                        &materials,
                        time,
                        count,
                        color_start,
                        color_end,
                        kind,
                        ttl,
                        &scatter_distr,
                        &velocity_distr,
                    )
                }
                Explosion => {
                    lazy_static! {
                        static ref FRAME_SKIP_DISTRIBUTION: Uniform<usize> = Uniform::new(0, 4);
                    }

                    let mut rng = rand::thread_rng();

                    // spawn 512 particles each for both color ramps
                    for ramp in [&*COLOR_RAMP_EXPLOSION_FAST, &*COLOR_RAMP_EXPLOSION_SLOW].iter() {
                        let frame_skip = FRAME_SKIP_DISTRIBUTION.sample(&mut rng);
                        spawn_random_cloud(
                            e,
                            &mesh,
                            &materials,
                            time,
                            512,
                            ramp.ramp[frame_skip] as _,
                            ramp.ramp[frame_skip] as _,
                            ParticleKind::Explosion { ramp, frame_skip },
                            Duration::try_seconds(5).unwrap(),
                            &*EXPLOSION_SCATTER_DISTRIBUTION,
                            &*EXPLOSION_VELOCITY_DISTRIBUTION,
                        );
                    }
                }
                ColorExplosion { colors } => {
                    spawn_random_cloud(
                        e,
                        &mesh,
                        &materials,
                        time,
                        512,
                        *colors.start() as usize,
                        *colors.end() as usize,
                        ParticleKind::Blob {
                            has_z_velocity: true,
                        },
                        Duration::try_seconds(5).unwrap(),
                        &*EXPLOSION_SCATTER_DISTRIBUTION,
                        &*EXPLOSION_VELOCITY_DISTRIBUTION,
                    );
                }
                SpawnExplosion => {
                    // R_BlobExplosion picks a random ttl with 1 + (rand() & 8) * 0.05
                    // which gives a value of either 1 or 1.4 seconds.
                    // (it's possible it was supposed to be 1 + (rand() & 7) * 0.05, which
                    // would yield between 1 and 1.35 seconds in increments of 50ms.)
                    let ttls = [
                        Duration::try_seconds(1).unwrap(),
                        Duration::try_milliseconds(1400).unwrap(),
                    ];

                    for ttl in ttls.iter().cloned() {
                        spawn_random_cloud(
                            e,
                            &mesh,
                            &materials,
                            time,
                            256,
                            66,
                            71,
                            ParticleKind::Blob {
                                has_z_velocity: true,
                            },
                            ttl,
                            &EXPLOSION_SCATTER_DISTRIBUTION,
                            &EXPLOSION_VELOCITY_DISTRIBUTION,
                        );
                        spawn_random_cloud(
                            e,
                            &mesh,
                            &materials,
                            time,
                            256,
                            150,
                            155,
                            ParticleKind::Blob {
                                has_z_velocity: false,
                            },
                            ttl,
                            &EXPLOSION_SCATTER_DISTRIBUTION,
                            &EXPLOSION_VELOCITY_DISTRIBUTION,
                        );
                    }
                }

                ProjectileImpact {
                    direction,
                    color,
                    count,
                } => {
                    lazy_static! {
                        static ref SCATTER_DISTRIBUTION: Uniform<f32> = Uniform::new(-8.0, 8.0);
                        // any color in block of 8 (see below)
                        static ref COLOR_DISTRIBUTION: Uniform<u8> = Uniform::new(0, 8);
                        // ttl between 0.1 and 0.5 seconds
                        static ref TTL_DISTRIBUTION: Uniform<i64> = Uniform::new(100, 500);
                    }

                    let mut rng = rand::thread_rng();

                    for _ in 0..count {
                        let scatter = random_vector3(&SCATTER_DISTRIBUTION);

                        // picks any color in the block of 8 the original color belongs to.
                        // e.g., if the color argument is 17, picks randomly in [16, 23]
                        let color = (color & !7) + COLOR_DISTRIBUTION.sample(&mut rng);

                        let ttl =
                            Duration::try_milliseconds(TTL_DISTRIBUTION.sample(&mut rng)).unwrap();

                        e.spawn((
                            MaterialMeshBundle {
                                mesh: mesh.clone(),
                                material: materials[color as usize].clone(),
                                transform: Transform::from_translation(scatter),
                                ..default()
                            },
                            Particle {
                                kind: ParticleKind::Grav,
                                velocity: 15.0 * direction,
                                color,
                                spawned: time,
                                expire: time + ttl,
                            },
                        ));
                    }
                }
                LavaSplash => {
                    lazy_static! {
                        // ttl between 2 and 2.64 seconds
                        static ref TTL_DISTRIBUTION: Uniform<i64> = Uniform::new(2000, 2640);

                        // any color on row 14
                        static ref COLOR_DISTRIBUTION: Uniform<u8> = Uniform::new(224, 232);

                        static ref DIR_OFFSET_DISTRIBUTION: Uniform<f32> = Uniform::new(0.0, 8.0);
                        static ref SCATTER_Z_DISTRIBUTION: Uniform<f32> = Uniform::new(0.0, 64.0);
                        static ref VELOCITY_DISTRIBUTION: Uniform<f32> = Uniform::new(50.0, 114.0);
                    }

                    let mut rng = rand::thread_rng();

                    for i in -16..16 {
                        for j in -16..16 {
                            let direction = Vec3::new(
                                8.0 * i as f32 + DIR_OFFSET_DISTRIBUTION.sample(&mut rng),
                                8.0 * j as f32 + DIR_OFFSET_DISTRIBUTION.sample(&mut rng),
                                256.0,
                            );

                            let scatter = Vec3::new(
                                direction.x,
                                direction.y,
                                SCATTER_Z_DISTRIBUTION.sample(&mut rng),
                            );

                            let velocity = VELOCITY_DISTRIBUTION.sample(&mut rng);

                            let color = COLOR_DISTRIBUTION.sample(&mut rng);
                            let ttl = Duration::try_milliseconds(TTL_DISTRIBUTION.sample(&mut rng))
                                .unwrap();

                            e.spawn((
                                MaterialMeshBundle {
                                    mesh: mesh.clone(),
                                    material: materials[color as usize].clone(),
                                    transform: Transform::from_translation(scatter),
                                    ..default()
                                },
                                Particle {
                                    kind: ParticleKind::Grav,
                                    velocity: direction.normalize() * velocity,
                                    color,
                                    spawned: time,
                                    expire: time + ttl,
                                },
                            ));
                        }
                    }
                }
                TeleporterWarp => {
                    lazy_static! {
                        // ttl between 0.2 and 0.34 seconds
                        static ref TTL_DISTRIBUTION: Uniform<i64> = Uniform::new(200, 340);

                        // random grey particles
                        static ref COLOR_DISTRIBUTION: Uniform<u8> = Uniform::new(7, 14);

                        static ref SCATTER_DISTRIBUTION: Uniform<f32> = Uniform::new(0.0, 4.0);
                        static ref VELOCITY_DISTRIBUTION: Uniform<f32> = Uniform::new(50.0, 114.0);
                    }

                    let mut rng = rand::thread_rng();

                    for i in (-16..16).step_by(4) {
                        for j in (-16..16).step_by(4) {
                            for k in (-24..32).step_by(4) {
                                let direction = Vec3::new(j as f32, i as f32, k as f32) * 8.0;
                                let scatter = Vec3::new(i as f32, j as f32, k as f32)
                                    + random_vector3(&SCATTER_DISTRIBUTION);
                                let velocity = VELOCITY_DISTRIBUTION.sample(&mut rng);
                                let color = COLOR_DISTRIBUTION.sample(&mut rng);
                                let ttl =
                                    Duration::try_milliseconds(TTL_DISTRIBUTION.sample(&mut rng))
                                        .unwrap();

                                e.spawn((
                                    MaterialMeshBundle {
                                        mesh: mesh.clone(),
                                        material: materials[color as usize].clone(),
                                        transform: Transform::from_translation(scatter),
                                        ..default()
                                    },
                                    Particle {
                                        kind: ParticleKind::Grav,
                                        velocity: direction.normalize() * velocity,
                                        color,
                                        spawned: time,
                                        expire: time + ttl,
                                    },
                                ));
                            }
                        }
                    }
                }
                Trail { end, kind, sparse } => {
                    use TrailKind::*;

                    lazy_static! {
                        static ref SCATTER_DISTRIBUTION: Uniform<f32> = Uniform::new(-3.0, 3.0);
                        static ref FRAME_SKIP_DISTRIBUTION: Uniform<usize> = Uniform::new(0, 4);
                        static ref BLOOD_COLOR_DISTRIBUTION: Uniform<u8> = Uniform::new(67, 71);
                        static ref VORE_COLOR_DISTRIBUTION: Uniform<u8> = Uniform::new(152, 156);
                    }

                    let mut rng = rand::thread_rng();

                    let distance = end.length();
                    let direction = end.normalize();

                    // particle interval in units
                    let interval = if sparse { 3.0 } else { 1.0 }
                        + match kind {
                            BloodSlight => 3.0,
                            _ => 0.0,
                        };

                    let ttl = Duration::try_seconds(2).unwrap();

                    for step in 0..(distance / interval) as i32 {
                        let frame_skip = FRAME_SKIP_DISTRIBUTION.sample(&mut rng);
                        let particle_kind = match kind {
                            Rocket => ParticleKind::Fire { frame_skip },
                            Smoke => ParticleKind::Fire {
                                frame_skip: frame_skip + 2,
                            },
                            Blood | BloodSlight => ParticleKind::Grav,
                            TracerGreen | TracerRed | Vore => ParticleKind::Static,
                        };

                        let scatter = random_vector3(&SCATTER_DISTRIBUTION);

                        let origin = direction * interval
                            + match kind {
                                // vore scatter is [-16, 15] in original
                                // this gives range of ~[-16, 16]
                                Vore => scatter * 5.33,
                                _ => scatter,
                            };

                        let velocity = match kind {
                            TracerGreen | TracerRed => {
                                30.0 * if step & 1 == 1 {
                                    Vec3::new(direction.y, -direction.x, 0.0)
                                } else {
                                    Vec3::new(-direction.y, direction.x, 0.0)
                                }
                            }

                            _ => Vec3::ZERO,
                        };

                        let color = match kind {
                            Rocket => COLOR_RAMP_FIRE.ramp[frame_skip],
                            Smoke => COLOR_RAMP_FIRE.ramp[frame_skip + 2],
                            Blood | BloodSlight => BLOOD_COLOR_DISTRIBUTION.sample(&mut rng),
                            TracerGreen => 52 + 2 * (step & 4) as u8,
                            TracerRed => 230 + 2 * (step & 4) as u8,
                            Vore => VORE_COLOR_DISTRIBUTION.sample(&mut rng),
                        };

                        e.spawn((
                            MaterialMeshBundle {
                                mesh: mesh.clone(),
                                material: materials[color as usize].clone(),
                                transform: Transform::from_translation(scatter),
                                ..default()
                            },
                            Particle {
                                kind: particle_kind,
                                velocity,
                                color,
                                spawned: time,
                                expire: time + ttl,
                            },
                        ));
                    }
                }
            }
        });
    }
}

impl Particle {
    /// Particle update function.
    ///
    /// The return value indicates whether the particle should be retained after this
    /// frame.
    ///
    /// For details on how individual particles behave, see the documentation for
    /// [`ParticleKind`](ParticleKind).
    pub fn update(
        &mut self,
        mut transform: Mut<Transform>,
        time: Duration,
        frame_time: Duration,
        sv_gravity: f32,
    ) -> bool {
        use ParticleKind::*;

        let velocity_factor = engine::duration_to_f32(frame_time);
        let gravity = velocity_factor * sv_gravity * PARTICLE_GRAVITY_FACTOR;

        // don't bother updating expired particles
        if time >= self.expire {
            return false;
        }

        let velocity = self.velocity * velocity_factor;

        match self.kind {
            Static => true,

            Grav => {
                transform.translation += velocity;
                self.velocity.z -= gravity;
                true
            }

            Fire { frame_skip } => match COLOR_RAMP_FIRE.color(time - self.spawned, frame_skip) {
                Some(c) => {
                    transform.translation += velocity;
                    // rises instead of falling
                    self.velocity.z += gravity;
                    self.color = c;
                    true
                }
                None => false,
            },

            Explosion { ramp, frame_skip } => match ramp.color(time - self.spawned, frame_skip) {
                Some(c) => {
                    transform.translation += velocity;
                    self.velocity.z -= gravity;
                    self.color = c;
                    true
                }
                None => false,
            },

            Blob { has_z_velocity } => {
                if !has_z_velocity {
                    let velocity = Vec3::new(1., 1., 0.) * velocity;
                    transform.translation += velocity;
                } else {
                    transform.translation += velocity;
                    self.velocity.z -= gravity;
                }

                true
            }
        }
    }

    pub fn color(&self) -> u8 {
        self.color
    }
}

pub enum TrailKind {
    Rocket = 0,
    Smoke = 1,
    Blood = 2,
    TracerGreen = 3,
    BloodSlight = 4,
    TracerRed = 5,
    Vore = 6,
}
