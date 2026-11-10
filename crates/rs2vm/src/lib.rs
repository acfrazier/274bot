#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! Bounded, symbolic execution of the Lost City RuneScript bytecode.
//!
//! This offline library has no bot state and no runtime host dependencies.

pub mod commands;
pub mod machine;
pub mod profile;
pub mod program;
pub mod world;

/// A script's index in the server script pack.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScriptId(pub u32);
/// A server loc type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocTypeId(pub i32);
/// A player's backing variable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VarpId(pub u16);
/// A packed engine coordinate: z | x << 14 | level << 28.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Coord(pub i32);
impl Coord {
    /// Pack parts with the masks used by engine/CoordGrid.ts:136-138.
    pub const fn new(x: i32, level: i32, z: i32) -> Self {
        Self((z & 0x3fff) | ((x & 0x3fff) << 14) | ((level & 3) << 28))
    }
    /// East-west coordinate.
    pub const fn x(self) -> i32 { (self.0 >> 14) & 0x3fff }
    /// Height plane.
    pub const fn level(self) -> i32 { (self.0 >> 28) & 3 }
    /// North-south coordinate.
    pub const fn z(self) -> i32 { self.0 & 0x3fff }
    /// Apply engine-masked offsets.
    pub const fn offset(self, dx: i32, dlevel: i32, dz: i32) -> Self {
        Self::new(self.x().wrapping_add(dx), self.level().wrapping_add(dlevel), self.z().wrapping_add(dz))
    }
}
impl std::fmt::Display for Coord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}_{}_{}", self.level(), self.x(), self.z())
    }
}
/// One dispatched instruction in one script.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Site {
    /// Script containing the instruction.
    pub script: ScriptId,
    /// Zero-based instruction index.
    pub pc: u32,
}
/// Inclusive bounds on elapsed game ticks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ticks {
    /// Least elapsed ticks.
    pub min: u32,
    /// Greatest elapsed ticks.
    pub max: u32,
}
/// Inclusive bounds on engine-dispatched instruction count.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OpCount {
    /// Least dispatched instructions.
    pub min: u32,
    /// Greatest dispatched instructions.
    pub max: u32,
}
