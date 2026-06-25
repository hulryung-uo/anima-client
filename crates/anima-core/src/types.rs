//! Common primitive types shared across the core.

/// A UO entity identifier ("serial"). `0` and `0xFFFF_FFFF` carry special
/// meaning in the protocol; the `0x4000_0000` boundary splits mobiles from items.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Serial(pub u32);

impl Serial {
    pub const ZERO: Serial = Serial(0);
    pub const INVALID: Serial = Serial(0xFFFF_FFFF);

    pub const fn is_mobile(self) -> bool {
        self.0 > 0 && self.0 < 0x4000_0000
    }

    pub const fn is_item(self) -> bool {
        self.0 >= 0x4000_0000 && self.0 != 0xFFFF_FFFF
    }
}

/// A world position. UO uses unsigned 16-bit X/Y and signed 8-bit Z.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Position {
    pub x: u16,
    pub y: u16,
    pub z: i8,
}

/// Facing direction. In walk packets the low 3 bits are the direction and
/// bit `0x80` is the "running" flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Direction {
    North = 0,
    Right = 1,
    East = 2,
    Down = 3,
    South = 4,
    Left = 5,
    West = 6,
    Up = 7,
}

impl Direction {
    pub const MASK: u8 = 0x07;
    pub const RUNNING: u8 = 0x80;

    pub fn from_bits(b: u8) -> Direction {
        match b & Self::MASK {
            0 => Direction::North,
            1 => Direction::Right,
            2 => Direction::East,
            3 => Direction::Down,
            4 => Direction::South,
            5 => Direction::Left,
            6 => Direction::West,
            _ => Direction::Up,
        }
    }
}
