//! What screen a PinMAME game has, from the ROM name.
//!
//! `data/rom_displays.csv` was made by running every game of the PinMAME 3.7
//! collection far enough for the library to announce its displays: a dot
//! matrix (128x32, 128x16, 192x64...), a video screen (Mr. Game's rotated CRT
//! is 256x248, nearly square) or only segment displays. The B2S fix uses it to
//! leave alone the tables that have no dot matrix, and to look for a hole of
//! the shape the game's screen really has.

use std::collections::HashMap;
use std::sync::OnceLock;

/// The screen a game draws its scores on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    /// A dot matrix display of `width` x `height` dots.
    Dmd { width: u32, height: u32 },
    /// A CRT or LCD video screen of `width` x `height` pixels.
    Video { width: u32, height: u32 },
    /// Segment displays only: no dot matrix to show.
    Segments,
    /// A ROM the list does not know (a recent or custom one).
    Unknown,
}

impl Screen {
    /// Width over height of the screen, `None` when there is no such screen.
    pub fn aspect(self) -> Option<f32> {
        match self {
            Screen::Dmd { width, height } | Screen::Video { width, height } if height > 0 => {
                Some(width as f32 / height as f32)
            }
            _ => None,
        }
    }
}

fn table() -> &'static HashMap<&'static str, Screen> {
    static TABLE: OnceLock<HashMap<&'static str, Screen>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut map = HashMap::new();
        for line in include_str!("../data/rom_displays.csv").lines() {
            if line.starts_with('#') {
                continue;
            }
            let mut f = line.split(',');
            let (Some(name), Some(kind), Some(w), Some(h)) =
                (f.next(), f.next(), f.next(), f.next())
            else {
                continue;
            };
            let (Ok(width), Ok(height)) = (w.trim().parse(), h.trim().parse()) else {
                continue;
            };
            let screen = match kind {
                "dmd" => Screen::Dmd { width, height },
                "video" => Screen::Video { width, height },
                "none" => Screen::Segments,
                _ => continue,
            };
            map.insert(name, screen);
        }
        map
    })
}

/// The screen of the game `rom` (case does not matter).
pub fn screen_of(rom: &str) -> Screen {
    let rom = rom.trim().to_ascii_lowercase();
    table()
        .get(rom.as_str())
        .copied()
        .unwrap_or(Screen::Unknown)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_games() {
        assert_eq!(
            screen_of("mm_109c"),
            Screen::Dmd {
                width: 128,
                height: 32
            }
        );
        assert_eq!(
            screen_of("MOTRSHOW"),
            Screen::Video {
                width: 256,
                height: 248
            }
        );
        assert_eq!(screen_of("bk2k_l4"), Screen::Segments);
        assert_eq!(screen_of("no_such_rom"), Screen::Unknown);
    }

    #[test]
    fn aspect_of_the_screens() {
        assert_eq!(screen_of("mm_109c").aspect(), Some(4.0));
        let video = screen_of("motrshow").aspect().unwrap();
        assert!((video - 256.0 / 248.0).abs() < 1e-6);
        assert_eq!(screen_of("bk2k_l4").aspect(), None);
    }
}
