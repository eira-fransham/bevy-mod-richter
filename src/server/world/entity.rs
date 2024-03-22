// Copyright © 2018 Cormac O'Brien
//
// Permission is hereby granted, free of charge, to any person obtaining a copy of this software
// and associated documentation files (the "Software"), to deal in the Software without
// restriction, including without limitation the rights to use, copy, modify, merge, publish,
// distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the
// Software is furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all copies or
// substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING
// BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
// NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM,
// DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

use std::{cell::RefCell, error::Error, fmt, iter, rc::Rc, sync::Arc};

use crate::{
    common::{engine::duration_to_f32, net::EntityState},
    server::{
        progs::{EntityId, FieldDef, FunctionId, ProgsError, StringId, StringTable, Type},
        world::phys::MoveKind,
    },
};

use arrayvec::ArrayString;
use bevy::prelude::*;
use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use cgmath::Vector3;
use chrono::Duration;
use num::FromPrimitive;
use num_derive::FromPrimitive;
use parking_lot::Mutex;
use uluru::LRUCache;

pub const MAX_ENT_LEAVES: usize = 16;

pub const STATIC_ADDRESS_COUNT: usize = 105;

#[derive(Debug)]
pub enum EntityError {
    Io(::std::io::Error),
    Address(isize),
    Other(String),
    NoVacantSlots,
}

impl EntityError {
    pub fn with_msg<S>(msg: S) -> Self
    where
        S: AsRef<str>,
    {
        EntityError::Other(msg.as_ref().to_owned())
    }
}

impl fmt::Display for EntityError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            EntityError::Io(ref err) => {
                write!(f, "I/O error: ")?;
                err.fmt(f)
            }
            EntityError::Address(val) => write!(f, "Invalid address ({})", val),
            EntityError::Other(ref msg) => write!(f, "{}", msg),
            EntityError::NoVacantSlots => write!(f, "No vacant slots"),
        }
    }
}

impl Error for EntityError {}

impl From<::std::io::Error> for EntityError {
    fn from(error: ::std::io::Error) -> Self {
        EntityError::Io(error)
    }
}

/// A trait which covers addresses of typed values.
pub trait FieldAddr {
    /// The type of value referenced by this address.
    type Value;

    /// Loads the value at this address.
    fn load(&self, type_def: &EntityTypeDef, ent: &Entity) -> Result<Self::Value, EntityError>;

    /// Stores a value at this address.
    fn store(
        &self,
        type_def: &EntityTypeDef,
        ent: &mut Entity,
        value: Self::Value,
    ) -> Result<(), EntityError>;
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, FromPrimitive)]
pub enum FieldAddrFloat {
    ModelIndex = 0,
    AbsMinX = 1,
    AbsMinY = 2,
    AbsMinZ = 3,
    AbsMaxX = 4,
    AbsMaxY = 5,
    AbsMaxZ = 6,
    /// Used by mobile level geometry such as moving platforms.
    LocalTime = 7,
    /// Determines the movement behavior of an entity. The value must be a variant of `MoveKind`.
    MoveKind = 8,
    Solid = 9,
    OriginX = 10,
    OriginY = 11,
    OriginZ = 12,
    OldOriginX = 13,
    OldOriginY = 14,
    OldOriginZ = 15,
    VelocityX = 16,
    VelocityY = 17,
    VelocityZ = 18,
    AnglesX = 19,
    AnglesY = 20,
    AnglesZ = 21,
    AngularVelocityX = 22,
    AngularVelocityY = 23,
    AngularVelocityZ = 24,
    PunchAngleX = 25,
    PunchAngleY = 26,
    PunchAngleZ = 27,
    /// The index of the entity's animation frame.
    FrameId = 30,
    /// The index of the entity's skin.
    SkinId = 31,
    /// Effects flags applied to the entity. See `EntityEffects`.
    Effects = 32,
    /// Minimum extent in local coordinates, X-coordinate.
    MinsX = 33,
    /// Minimum extent in local coordinates, Y-coordinate.
    MinsY = 34,
    /// Minimum extent in local coordinates, Z-coordinate.
    MinsZ = 35,
    /// Maximum extent in local coordinates, X-coordinate.
    MaxsX = 36,
    /// Maximum extent in local coordinates, Y-coordinate.
    MaxsY = 37,
    /// Maximum extent in local coordinates, Z-coordinate.
    MaxsZ = 38,
    SizeX = 39,
    SizeY = 40,
    SizeZ = 41,
    /// The next server time at which the entity should run its think function.
    NextThink = 46,
    /// The entity's remaining health.
    Health = 48,
    /// The number of kills scored by the entity.
    Frags = 49,
    Weapon = 50,
    WeaponFrame = 52,
    /// The entity's remaining ammunition for its selected weapon.
    CurrentAmmo = 53,
    /// The entity's remaining shotgun shells.
    AmmoShells = 54,
    /// The entity's remaining shotgun shells.
    AmmoNails = 55,
    /// The entity's remaining rockets/grenades.
    AmmoRockets = 56,
    AmmoCells = 57,
    Items = 58,
    TakeDamage = 59,
    DeadFlag = 61,
    ViewOffsetX = 62,
    ViewOffsetY = 63,
    ViewOffsetZ = 64,
    Button0 = 65,
    Button1 = 66,
    Button2 = 67,
    Impulse = 68,
    FixAngle = 69,
    ViewAngleX = 70,
    ViewAngleY = 71,
    ViewAngleZ = 72,
    IdealPitch = 73,
    Flags = 76,
    Colormap = 77,
    Team = 78,
    MaxHealth = 79,
    TeleportTime = 80,
    ArmorStrength = 81,
    ArmorValue = 82,
    WaterLevel = 83,
    Contents = 84,
    IdealYaw = 85,
    YawSpeed = 86,
    SpawnFlags = 89,
    DmgTake = 92,
    DmgSave = 93,
    MoveDirectionX = 96,
    MoveDirectionY = 97,
    MoveDirectionZ = 98,
    Sounds = 100,
}

impl FieldAddr for FieldAddrFloat {
    type Value = f32;

    #[inline]
    fn load(&self, type_def: &EntityTypeDef, ent: &Entity) -> Result<Self::Value, EntityError> {
        ent.get_float(type_def, *self as i16)
    }

    #[inline]
    fn store(
        &self,
        type_def: &EntityTypeDef,
        ent: &mut Entity,
        value: Self::Value,
    ) -> Result<(), EntityError> {
        ent.put_float(type_def, value, *self as i16)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, FromPrimitive)]
pub enum FieldAddrVector {
    AbsMin = 1,
    AbsMax = 4,
    Origin = 10,
    OldOrigin = 13,
    Velocity = 16,
    Angles = 19,
    AngularVelocity = 22,
    PunchAngle = 25,
    Mins = 33,
    Maxs = 36,
    Size = 39,
    ViewOffset = 62,
    ViewAngle = 70,
    MoveDirection = 96,
}

impl FieldAddr for FieldAddrVector {
    type Value = [f32; 3];

    #[inline]
    fn load(&self, type_def: &EntityTypeDef, ent: &Entity) -> Result<Self::Value, EntityError> {
        ent.get_vector(type_def, *self as i16)
    }

    #[inline]
    fn store(
        &self,
        type_def: &EntityTypeDef,
        ent: &mut Entity,
        value: Self::Value,
    ) -> Result<(), EntityError> {
        ent.put_vector(type_def, value, *self as i16)
    }
}

#[derive(Copy, Clone, Debug, FromPrimitive)]
pub enum FieldAddrStringId {
    ClassName = 28,
    ModelName = 29,
    WeaponModelName = 51,
    NetName = 74,
    Target = 90,
    TargetName = 91,
    Message = 99,
    Noise0Name = 101,
    Noise1Name = 102,
    Noise2Name = 103,
    Noise3Name = 104,
}

impl FieldAddr for FieldAddrStringId {
    type Value = StringId;

    fn load(&self, type_def: &EntityTypeDef, ent: &Entity) -> Result<Self::Value, EntityError> {
        ent.get_int(*self as i16)
            .map(|val| StringId(val.try_into().unwrap()))
    }

    fn store(
        &self,
        type_def: &EntityTypeDef,
        ent: &mut Entity,
        value: Self::Value,
    ) -> Result<(), EntityError> {
        ent.put_int(value.0.try_into().unwrap(), *self as i16)
    }
}

#[derive(Copy, Clone, Debug, FromPrimitive)]
pub enum FieldAddrEntityId {
    /// The entity this entity is standing on.
    Ground = 47,
    Chain = 60,
    Enemy = 75,
    Aim = 87,
    Goal = 88,
    DmgInflictor = 94,
    Owner = 95,
}

impl FieldAddr for FieldAddrEntityId {
    type Value = EntityId;

    fn load(&self, type_def: &EntityTypeDef, ent: &Entity) -> Result<Self::Value, EntityError> {
        ent.entity_id(type_def, *self as i16)
    }

    fn store(
        &self,
        type_def: &EntityTypeDef,
        ent: &mut Entity,
        value: Self::Value,
    ) -> Result<(), EntityError> {
        ent.put_entity_id(type_def, value, *self as i16)
    }
}

#[derive(Copy, Clone, Debug, FromPrimitive)]
pub enum FieldAddrFunctionId {
    Touch = 42,
    Use = 43,
    Think = 44,
    Blocked = 45,
}

impl FieldAddr for FieldAddrFunctionId {
    type Value = FunctionId;

    #[inline]
    fn load(&self, type_def: &EntityTypeDef, ent: &Entity) -> Result<Self::Value, EntityError> {
        ent.function_id(type_def, *self as i16)
    }

    #[inline]
    fn store(
        &self,
        type_def: &EntityTypeDef,
        ent: &mut Entity,
        value: Self::Value,
    ) -> Result<(), EntityError> {
        ent.put_function_id(type_def, value, *self as i16)
    }
}

bitflags! {
    pub struct EntityFlags: u16 {
        const FLY            = 0b0000000000001;
        const SWIM           = 0b0000000000010;
        const CONVEYOR       = 0b0000000000100;
        const CLIENT         = 0b0000000001000;
        const IN_WATER       = 0b0000000010000;
        const MONSTER        = 0b0000000100000;
        const GOD_MODE       = 0b0000001000000;
        const NO_TARGET      = 0b0000010000000;
        const ITEM           = 0b0000100000000;
        const ON_GROUND      = 0b0001000000000;
        const PARTIAL_GROUND = 0b0010000000000;
        const WATER_JUMP     = 0b0100000000000;
        const JUMP_RELEASED  = 0b1000000000000;
    }
}

// TODO: if this never gets used, remove it
#[allow(dead_code)]
fn float_addr(addr: usize) -> Result<FieldAddrFloat, ProgsError> {
    match FieldAddrFloat::from_usize(addr) {
        Some(f) => Ok(f),
        None => Err(ProgsError::with_msg(format!(
            "float_addr: invalid address ({})",
            addr
        ))),
    }
}

// TODO: if this never gets used, remove it
#[allow(dead_code)]
fn vector_addr(addr: usize) -> Result<FieldAddrVector, ProgsError> {
    match FieldAddrVector::from_usize(addr) {
        Some(v) => Ok(v),
        None => Err(ProgsError::with_msg(format!(
            "vector_addr: invalid address ({})",
            addr
        ))),
    }
}

#[derive(Debug)]
struct FieldDefCacheEntry {
    name: ArrayString<64>,
    index: usize,
}

#[derive(Debug)]
pub struct EntityTypeDef {
    addr_count: usize,
    field_defs: Box<[FieldDef]>,

    name_cache: Mutex<LRUCache<FieldDefCacheEntry, 16>>,
}

impl EntityTypeDef {
    pub fn new(
        addr_count: usize,
        field_defs: Box<[FieldDef]>,
    ) -> Result<EntityTypeDef, EntityError> {
        if addr_count < STATIC_ADDRESS_COUNT {
            return Err(EntityError::with_msg(format!(
                "addr_count ({}) < STATIC_ADDRESS_COUNT ({})",
                addr_count, STATIC_ADDRESS_COUNT
            )));
        }

        Ok(EntityTypeDef {
            addr_count,
            field_defs,
            name_cache: default(),
        })
    }

    pub fn addr_count(&self) -> usize {
        self.addr_count
    }

    pub fn field_defs(&self) -> &[FieldDef] {
        self.field_defs.as_ref()
    }

    /// Locate a field definition given its name.
    pub fn find<S>(&self, string_table: &StringTable, name: S) -> Option<&FieldDef>
    where
        S: AsRef<str>,
    {
        let name = name.as_ref();

        let mut name_cache = self.name_cache.lock();

        if let Some(cached) = name_cache.find(|entry| &entry.name == name) {
            return Some(&self.field_defs[cached.index]);
        }

        let name_id = string_table.find(name)?;

        let (index, def) = self
            .field_defs
            .iter()
            .enumerate()
            .find(|(_, def)| def.name_id == name_id)?;

        name_cache.insert(FieldDefCacheEntry {
            name: ArrayString::from(name).unwrap(),
            index,
        });

        Some(def)
    }
}

#[derive(Debug, FromPrimitive, PartialEq)]
pub enum EntitySolid {
    Not = 0,
    Trigger = 1,
    BBox = 2,
    SlideBox = 3,
    Bsp = 4,
}

#[derive(Debug, Clone)]
pub struct Entity {
    addrs: im::Vector<[u8; 4]>,

    pub leaf_count: usize,
    pub leaf_ids: [usize; MAX_ENT_LEAVES],
    pub baseline: EntityState,
}

impl Entity {
    pub fn new(type_def: &EntityTypeDef) -> Entity {
        let addrs = iter::repeat_n([0; 4], type_def.addr_count).collect();

        Entity {
            addrs,
            leaf_count: 0,
            leaf_ids: [0; MAX_ENT_LEAVES],
            baseline: EntityState::uninitialized(),
        }
    }

    pub fn type_check(
        &self,
        type_def: &EntityTypeDef,
        addr: usize,
        type_: Type,
    ) -> Result<(), EntityError> {
        match type_def
            .field_defs
            .iter()
            .find(|def| def.type_ != Type::QVoid && def.offset as usize == addr)
        {
            Some(d) => {
                if type_ == d.type_ {
                    Ok(())
                } else if type_ == Type::QFloat && d.type_ == Type::QVector {
                    Ok(())
                } else if type_ == Type::QVector && d.type_ == Type::QFloat {
                    Ok(())
                } else {
                    Err(EntityError::with_msg(format!(
                        "type check failed: addr={} expected={:?} actual={:?}",
                        addr, type_, d.type_
                    )))
                }
            }
            None => Ok(()),
        }
    }

    /// Returns a reference to the memory at the given address.
    pub fn get_addr(&self, addr: i16) -> Result<&[u8], EntityError> {
        if addr < 0 {
            return Err(EntityError::Address(addr as isize));
        }

        let addr = addr as usize;

        if addr > self.addrs.len() {
            return Err(EntityError::Address(addr as isize));
        }

        Ok(&self.addrs[addr])
    }

    /// Returns a mutable reference to the memory at the given address.
    pub fn get_addr_mut(&mut self, addr: i16) -> Result<&mut [u8], EntityError> {
        if addr < 0 {
            return Err(EntityError::Address(addr as isize));
        }

        let addr = addr as usize;

        if addr > self.addrs.len() {
            return Err(EntityError::Address(addr as isize));
        }

        Ok(&mut self.addrs[addr])
    }

    /// Returns a copy of the memory at the given address.
    pub fn get_bytes(&self, addr: i16) -> Result<[u8; 4], EntityError> {
        if addr < 0 {
            return Err(EntityError::Address(addr as isize));
        }

        let addr = addr as usize;

        if addr > self.addrs.len() {
            return Err(EntityError::Address(addr as isize));
        }

        Ok(self.addrs[addr])
    }

    /// Writes the provided data to the memory at the given address.
    ///
    /// This can be used to circumvent the type checker in cases where an operation is not dependent
    /// of the type of the data.
    pub fn put_bytes(&mut self, val: [u8; 4], addr: i16) -> Result<(), EntityError> {
        if addr < 0 {
            return Err(EntityError::Address(addr as isize));
        }

        let addr = addr as usize;

        if addr > self.addrs.len() {
            return Err(EntityError::Address(addr as isize));
        }

        self.addrs[addr] = val;
        Ok(())
    }

    #[inline]
    pub fn load<F>(&self, type_def: &EntityTypeDef, field: F) -> Result<F::Value, EntityError>
    where
        F: FieldAddr,
    {
        field.load(type_def, self)
    }

    #[inline]
    pub fn store<F>(
        &mut self,
        type_def: &EntityTypeDef,
        field: F,
        value: F::Value,
    ) -> Result<(), EntityError>
    where
        F: FieldAddr,
    {
        field.store(type_def, self, value)
    }

    /// Loads an `i32` from the given virtual address.
    pub fn get_int(&self, addr: i16) -> Result<i32, EntityError> {
        Ok(self.get_addr(addr)?.read_i32::<LittleEndian>()?)
    }

    /// Loads an `i32` from the given virtual address.
    pub fn put_int(&mut self, val: i32, addr: i16) -> Result<(), EntityError> {
        self.get_addr_mut(addr)?.write_i32::<LittleEndian>(val)?;
        Ok(())
    }

    /// Loads an `f32` from the given virtual address.
    pub fn get_float(&self, type_def: &EntityTypeDef, addr: i16) -> Result<f32, EntityError> {
        self.type_check(type_def, addr as usize, Type::QFloat)?;
        Ok(self.get_addr(addr)?.read_f32::<LittleEndian>()?)
    }

    /// Stores an `f32` at the given virtual address.
    pub fn put_float(
        &mut self,
        type_def: &EntityTypeDef,
        val: f32,
        addr: i16,
    ) -> Result<(), EntityError> {
        self.type_check(type_def, addr as usize, Type::QFloat)?;
        self.get_addr_mut(addr)?.write_f32::<LittleEndian>(val)?;
        Ok(())
    }

    /// Loads an `[f32; 3]` from the given virtual address.
    pub fn get_vector(&self, type_def: &EntityTypeDef, addr: i16) -> Result<[f32; 3], EntityError> {
        self.type_check(type_def, addr as usize, Type::QVector)?;

        let mut v = [0.0; 3];

        for i in 0..3 {
            v[i] = self.get_float(type_def, addr + i as i16)?;
        }

        Ok(v)
    }

    /// Stores an `[f32; 3]` at the given virtual address.
    pub fn put_vector(
        &mut self,
        type_def: &EntityTypeDef,
        val: [f32; 3],
        addr: i16,
    ) -> Result<(), EntityError> {
        self.type_check(type_def, addr as usize, Type::QVector)?;

        for i in 0..3 {
            self.put_float(type_def, val[i], addr + i as i16)?;
        }

        Ok(())
    }

    /// Loads a `StringId` from the given virtual address.
    pub fn string_id(&self, type_def: &EntityTypeDef, addr: i16) -> Result<StringId, EntityError> {
        self.type_check(type_def, addr as usize, Type::QString)?;

        Ok(StringId(
            self.get_addr(addr)?.read_i32::<LittleEndian>()? as usize
        ))
    }

    /// Stores a `StringId` at the given virtual address.
    pub fn put_string_id(
        &mut self,
        type_def: &EntityTypeDef,
        val: StringId,
        addr: i16,
    ) -> Result<(), EntityError> {
        self.type_check(type_def, addr as usize, Type::QString)?;

        self.get_addr_mut(addr)?
            .write_i32::<LittleEndian>(val.try_into().unwrap())?;
        Ok(())
    }

    /// Loads an `EntityId` from the given virtual address.
    pub fn entity_id(&self, type_def: &EntityTypeDef, addr: i16) -> Result<EntityId, EntityError> {
        self.type_check(type_def, addr as usize, Type::QEntity)?;

        match self.get_addr(addr)?.read_i32::<LittleEndian>()? {
            e if e < 0 => Err(EntityError::with_msg(format!("Negative entity ID ({})", e))),
            e => Ok(EntityId(e as usize)),
        }
    }

    /// Stores an `EntityId` at the given virtual address.
    pub fn put_entity_id(
        &mut self,
        type_def: &EntityTypeDef,
        val: EntityId,
        addr: i16,
    ) -> Result<(), EntityError> {
        self.type_check(type_def, addr as usize, Type::QEntity)?;

        self.get_addr_mut(addr)?
            .write_i32::<LittleEndian>(val.0 as i32)?;
        Ok(())
    }

    /// Loads a `FunctionId` from the given virtual address.
    pub fn function_id(
        &self,
        type_def: &EntityTypeDef,
        addr: i16,
    ) -> Result<FunctionId, EntityError> {
        self.type_check(type_def, addr as usize, Type::QFunction)?;
        Ok(FunctionId(
            self.get_addr(addr)?.read_i32::<LittleEndian>()? as usize
        ))
    }

    /// Stores a `FunctionId` at the given virtual address.
    pub fn put_function_id(
        &mut self,
        type_def: &EntityTypeDef,
        val: FunctionId,
        addr: i16,
    ) -> Result<(), EntityError> {
        self.type_check(type_def, addr as usize, Type::QFunction)?;
        self.get_addr_mut(addr)?
            .write_i32::<LittleEndian>(val.try_into().unwrap())?;
        Ok(())
    }

    /// Set this entity's minimum and maximum bounds and calculate its size.
    pub fn set_min_max_size<V>(
        &mut self,
        type_def: &EntityTypeDef,
        min: V,
        max: V,
    ) -> Result<(), EntityError>
    where
        V: Into<Vector3<f32>>,
    {
        let min = min.into();
        let max = max.into();
        let size = max - min;

        debug!("Setting entity min: {:?}", min);
        self.put_vector(type_def, min.into(), FieldAddrVector::Mins as i16)?;

        debug!("Setting entity max: {:?}", max);
        self.put_vector(type_def, max.into(), FieldAddrVector::Maxs as i16)?;

        debug!("Setting entity size: {:?}", size);
        self.put_vector(type_def, size.into(), FieldAddrVector::Size as i16)?;
        Ok(())
    }

    pub fn model_index(&self, type_def: &EntityTypeDef) -> Result<usize, EntityError> {
        let model_index = self.get_float(type_def, FieldAddrFloat::ModelIndex as i16)?;
        if model_index < 0.0 || model_index > ::std::usize::MAX as f32 {
            Err(EntityError::with_msg(format!(
                "Invalid value for entity.model_index ({})",
                model_index,
            )))
        } else {
            Ok(model_index as usize)
        }
    }

    pub fn abs_min(&self, type_def: &EntityTypeDef) -> Result<Vector3<f32>, EntityError> {
        Ok(self
            .get_vector(type_def, FieldAddrVector::AbsMin as i16)?
            .into())
    }

    pub fn abs_max(&self, type_def: &EntityTypeDef) -> Result<Vector3<f32>, EntityError> {
        Ok(self
            .get_vector(type_def, FieldAddrVector::AbsMax as i16)?
            .into())
    }

    pub fn solid(&self, type_def: &EntityTypeDef) -> Result<EntitySolid, EntityError> {
        let solid_i = self.get_float(type_def, FieldAddrFloat::Solid as i16)? as i32;
        match EntitySolid::from_i32(solid_i) {
            Some(s) => Ok(s),
            None => Err(EntityError::with_msg(format!(
                "Invalid value for entity.solid ({})",
                solid_i,
            ))),
        }
    }

    pub fn origin(&self, type_def: &EntityTypeDef) -> Result<Vector3<f32>, EntityError> {
        Ok(self
            .get_vector(type_def, FieldAddrVector::Origin as i16)?
            .into())
    }

    pub fn min(&self, type_def: &EntityTypeDef) -> Result<Vector3<f32>, EntityError> {
        Ok(self
            .get_vector(type_def, FieldAddrVector::Mins as i16)?
            .into())
    }

    pub fn max(&self, type_def: &EntityTypeDef) -> Result<Vector3<f32>, EntityError> {
        Ok(self
            .get_vector(type_def, FieldAddrVector::Maxs as i16)?
            .into())
    }

    pub fn size(&self, type_def: &EntityTypeDef) -> Result<Vector3<f32>, EntityError> {
        Ok(self
            .get_vector(type_def, FieldAddrVector::Size as i16)?
            .into())
    }

    pub fn velocity(&self, type_def: &EntityTypeDef) -> Result<Vector3<f32>, EntityError> {
        Ok(self
            .get_vector(type_def, FieldAddrVector::Velocity as i16)?
            .into())
    }

    pub fn set_velocity(
        &mut self,
        type_def: &EntityTypeDef,
        velocity: Vector3<f32>,
    ) -> Result<(), EntityError> {
        Ok(self.store(type_def, FieldAddrVector::Velocity, velocity.into())?)
    }

    /// Applies gravity to the entity.
    ///
    /// The effect depends on the provided value of the `sv_gravity` cvar, the
    /// amount of time being simulated, and the entity's own `gravity` field
    /// value.
    pub fn apply_gravity(
        &mut self,
        type_def: &EntityTypeDef,
        string_table: &StringTable,
        sv_gravity: f32,
        frame_time: Duration,
    ) -> Result<(), EntityError> {
        let ent_gravity = match type_def.find(string_table, "gravity") {
            Some(def) => self.get_float(type_def, def.offset as i16)?,
            None => 1.0,
        };

        let mut vel = self.velocity(type_def)?;
        vel.z -= ent_gravity * sv_gravity * duration_to_f32(frame_time);
        self.store(type_def, FieldAddrVector::Velocity, vel.into())?;

        Ok(())
    }

    /// Limits the entity's velocity by clamping each component (not the
    /// magnitude!) to an absolute value of `sv_maxvelocity`.
    pub fn limit_velocity(
        &mut self,
        type_def: &EntityTypeDef,
        sv_maxvelocity: f32,
    ) -> Result<(), EntityError> {
        let mut vel = self.velocity(type_def)?;
        for c in &mut vel[..] {
            *c = c.clamp(-sv_maxvelocity, sv_maxvelocity);
        }
        self.put_vector(type_def, vel.into(), FieldAddrVector::Velocity as i16)?;

        Ok(())
    }

    pub fn move_kind(&self, type_def: &EntityTypeDef) -> Result<MoveKind, EntityError> {
        let move_kind_f = self.get_float(type_def, FieldAddrFloat::MoveKind as i16)?;
        let move_kind_i = move_kind_f as i32;
        match MoveKind::from_i32(move_kind_i) {
            Some(m) => Ok(m),
            None => Err(EntityError::with_msg(format!(
                "Invalid value for entity.move_kind ({})",
                move_kind_f,
            ))),
        }
    }

    pub fn flags(&self, type_def: &EntityTypeDef) -> Result<EntityFlags, EntityError> {
        let flags_i = self.get_float(type_def, FieldAddrFloat::Flags as i16)? as u16;
        match EntityFlags::from_bits(flags_i) {
            Some(f) => Ok(f),
            None => Err(EntityError::with_msg(format!(
                "Invalid internal flags value ({})",
                flags_i
            ))),
        }
    }

    pub fn add_flags(
        &mut self,
        type_def: &EntityTypeDef,
        flags: EntityFlags,
    ) -> Result<(), EntityError> {
        let result = self.flags(type_def)? | flags;
        self.put_float(type_def, result.bits() as f32, FieldAddrFloat::Flags as i16)?;
        Ok(())
    }

    pub fn owner(&self, type_def: &EntityTypeDef) -> Result<EntityId, EntityError> {
        Ok(self.entity_id(type_def, FieldAddrEntityId::Owner as i16)?)
    }
}
