//! The shape of the launcher tiles. The recommended picture used to be 5:4
//! (480×384) while the tile was square, so every picture had bands or was
//! cropped. The tile now takes the shape the user's pictures have: 5:4 for
//! the classic launcher pictures, 1:1 for round wheels (PinUP Popper), 3:4
//! for upright playfield videos, 4:3 and 16:9 for landscape art.

/// A tile shape, width:height.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileShape {
    R5x4,
    R1x1,
    R4x3,
    R16x9,
    R3x4,
}

impl TileShape {
    pub const ALL: [Self; 5] = [Self::R5x4, Self::R1x1, Self::R4x3, Self::R16x9, Self::R3x4];

    /// The value stored in the settings, also the label ("5:4").
    pub fn key(self) -> &'static str {
        match self {
            Self::R5x4 => "5:4",
            Self::R1x1 => "1:1",
            Self::R4x3 => "4:3",
            Self::R16x9 => "16:9",
            Self::R3x4 => "3:4",
        }
    }

    /// The stored value back; anything unknown is the default 5:4.
    pub fn from_key(k: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|s| s.key() == k)
            .unwrap_or(Self::R5x4)
    }

    /// Width over height.
    pub fn ratio(self) -> f32 {
        let (w, h) = self.picture_size();
        w as f32 / h as f32
    }

    /// The picture size that fills this tile exactly, within the 480×384
    /// thumbnails the launcher keeps.
    pub fn picture_size(self) -> (u32, u32) {
        match self {
            Self::R5x4 => (480, 384),
            Self::R1x1 => (384, 384),
            Self::R4x3 => (480, 360),
            Self::R16x9 => (480, 270),
            Self::R3x4 => (288, 384),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_round_trip_and_unknown_is_the_default() {
        for s in TileShape::ALL {
            assert_eq!(TileShape::from_key(s.key()), s);
        }
        assert_eq!(TileShape::from_key(""), TileShape::R5x4);
        assert_eq!(TileShape::from_key("7:2"), TileShape::R5x4);
    }

    #[test]
    fn picture_sizes_have_the_shape_and_fit_the_thumbnails() {
        for s in TileShape::ALL {
            let (w, h) = s.picture_size();
            assert!(w <= crate::assets::THUMB_WIDTH && h <= crate::assets::THUMB_HEIGHT);
            let (a, b) = s.key().split_once(':').unwrap();
            let want = a.parse::<f32>().unwrap() / b.parse::<f32>().unwrap();
            assert!((s.ratio() - want).abs() < 0.01, "{}", s.key());
        }
    }
}
