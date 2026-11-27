//! Similar photos and bursts: photos taken close together that look alike,
//! suggested as groups to choose from, and the stacks the photo grid can
//! show them in. Nothing here deletes, moves, or stars a photo; the groups
//! are only shown.
//!
//! How a photo looks is a 64-bit difference hash of its thumbnail: the
//! thumbnail in grey at 9 × 8, each bit saying whether a pixel is brighter
//! than the one to its right. Photos that look alike differ in few bits.

use std::ops::Range;

use image::{imageops, DynamicImage, RgbaImage};

/// A photo as the grouping sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shot {
    pub id: i64,
    /// When it was taken, in seconds; 0 when unknown.
    pub taken_at: i64,
    pub look: u64,
}

/// How far apart, in seconds, two shots of one group can be by default.
pub const DEFAULT_WINDOW: i64 = 10;
/// How many of the 64 bits two shots of one group can differ in by default.
pub const DEFAULT_DISTANCE: u32 = 12;
/// How many of the 64 bits two photos taken seconds apart can differ in
/// and still stack: more, and they are of different things, such as a
/// photo of a sign and then of the view. Unrelated pictures differ in
/// about half the bits.
pub const DIFFERENT_DISTANCE: u32 = 24;

/// The difference hash of a thumbnail.
pub fn look(thumb: &RgbaImage) -> u64 {
    let grey = DynamicImage::ImageRgba8(thumb.clone()).into_luma8();
    let small = imageops::resize(&grey, 9, 8, imageops::FilterType::Triangle);
    let mut bits = 0u64;
    for y in 0..8 {
        for x in 0..8 {
            bits <<= 1;
            if small.get_pixel(x, y)[0] > small.get_pixel(x + 1, y)[0] {
                bits |= 1;
            }
        }
    }
    bits
}

/// How many bits two looks differ in, from 0 (alike) to 64.
pub fn distance(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// Groups of two or more shots, in the order taken: each shot joins the
/// group of the one before it when it was taken within `window` seconds of
/// it and looks within `max_distance` of it. Shots with no time are left
/// out, as a time is what makes a burst.
pub fn group(shots: &[Shot], window: i64, max_distance: u32) -> Vec<Vec<i64>> {
    let mut timed: Vec<&Shot> = shots.iter().filter(|shot| shot.taken_at > 0).collect();
    timed.sort_by_key(|shot| (shot.taken_at, shot.id));
    let mut groups: Vec<Vec<i64>> = Vec::new();
    let mut current: Vec<i64> = Vec::new();
    let mut previous: Option<&Shot> = None;
    for shot in timed {
        let joins = previous.is_some_and(|before| {
            shot.taken_at - before.taken_at <= window
                && distance(shot.look, before.look) <= max_distance
        });
        if !joins && current.len() > 1 {
            groups.push(std::mem::take(&mut current));
        }
        if !joins {
            current.clear();
        }
        current.push(shot.id);
        previous = Some(shot);
    }
    if current.len() > 1 {
        groups.push(current);
    }
    groups
}

/// Which photos the grid stacks: each rule joins a photo to the one before
/// it in the grid, and either is enough.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StackRules {
    /// Photos taken at most this many seconds apart, when set.
    pub within: Option<i64>,
    /// Photos taken the same day that look alike.
    pub alike: bool,
}

impl StackRules {
    pub fn any(self) -> bool {
        self.within.is_some() || self.alike
    }
}

/// A photo in the grid as stacking sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tile {
    /// When it was taken, in seconds; 0 or less when unknown.
    pub taken_at: i64,
    /// The local day it was taken, as the grid's headings group it.
    pub day: Option<i32>,
    /// How it looks, once that is known.
    pub look: Option<u64>,
    pub video: bool,
}

/// Runs of two or more tiles in a row, in grid order, that stack: each
/// tile after the first joins the one before it by one of `rules`. Videos
/// and photos with no time never stack, and photos taken close together
/// don't when they look very different; until their looks are known, the
/// time alone joins them.
pub fn stacks(tiles: &[Tile], rules: StackRules) -> Vec<Range<usize>> {
    let joins = |before: &Tile, tile: &Tile| {
        if before.video || tile.video || before.taken_at <= 0 || tile.taken_at <= 0 {
            return false;
        }
        let different = matches!(
            (before.look, tile.look),
            (Some(a), Some(b)) if distance(a, b) > DIFFERENT_DISTANCE
        );
        let close = !different
            && rules
                .within
                .is_some_and(|within| (tile.taken_at - before.taken_at).abs() <= within);
        let alike = rules.alike
            && before.day.is_some()
            && before.day == tile.day
            && matches!((before.look, tile.look), (Some(a), Some(b)) if distance(a, b) <= DEFAULT_DISTANCE);
        close || alike
    };
    let mut found = Vec::new();
    let mut start = 0;
    for index in 1..=tiles.len() {
        if index < tiles.len() && joins(&tiles[index - 1], &tiles[index]) {
            continue;
        }
        if index - start > 1 {
            found.push(start..index);
        }
        start = index;
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn picture(shift: u32, flip: bool) -> RgbaImage {
        RgbaImage::from_fn(90, 80, |x, y| {
            let level = ((x + shift) * 255 / 100) as u8 ^ if y > 40 { 0x40 } else { 0 };
            let level = if flip { 255 - level } else { level };
            Rgba([level, level, level, 255])
        })
    }

    #[test]
    fn photos_alike_have_close_looks_and_different_ones_far() {
        let base = look(&picture(0, false));
        assert!(
            distance(base, look(&picture(2, false))) <= DEFAULT_DISTANCE,
            "a slightly moved frame"
        );
        assert!(
            distance(base, look(&picture(0, true))) > DEFAULT_DISTANCE,
            "a different picture"
        );
        assert_eq!(distance(5, 5), 0);
        assert_eq!(distance(0, u64::MAX), 64);
    }

    #[test]
    fn a_burst_is_close_in_time_and_alike() {
        let shot = |id, taken_at, look| Shot { id, taken_at, look };
        let shots = [
            shot(1, 100, 0b1111),
            shot(2, 101, 0b1110),
            shot(3, 103, 0b1100),
            // Alike but a minute later: a new start.
            shot(4, 170, 0b1100),
            // Right after, but nothing like it.
            shot(5, 171, u64::MAX),
            shot(6, 172, u64::MAX - 1),
            // No time: never grouped.
            shot(7, 0, 0b1111),
        ];
        assert_eq!(
            group(&shots, DEFAULT_WINDOW, DEFAULT_DISTANCE),
            vec![vec![1, 2, 3], vec![5, 6]]
        );
        assert_eq!(
            group(&shots, 120, DEFAULT_DISTANCE),
            vec![vec![1, 2, 3, 4], vec![5, 6]],
            "a wider window"
        );
        assert_eq!(
            group(&shots, DEFAULT_WINDOW, 0),
            Vec::<Vec<i64>>::new(),
            "only identical looks"
        );
    }

    #[test]
    fn stacks_join_photos_in_a_row_by_time_or_look() {
        let tile = |taken_at, look| Tile {
            taken_at,
            day: Some((taken_at / 86_400) as i32),
            look,
            video: false,
        };
        // Newest first, as the grid shows them by default. The first is a
        // different picture two seconds before the next.
        let tiles = [
            tile(1_000_010, Some(0)),
            tile(1_000_008, Some(u64::MAX)),
            tile(1_000_007, Some(u64::MAX)),
            // A minute later, but alike: only a look-alike stack takes it.
            tile(1_000_000 - 60, Some(u64::MAX - 1)),
            tile(900_000, Some(0)),
        ];
        let within = |seconds| StackRules {
            within: Some(seconds),
            alike: false,
        };
        assert_eq!(
            stacks(&tiles, within(3)),
            vec![1..3],
            "not the different one"
        );
        assert_eq!(stacks(&tiles, within(1)), vec![1..3]);
        assert_eq!(
            stacks(
                &tiles,
                StackRules {
                    within: None,
                    alike: true
                }
            ),
            vec![1..4],
            "alike, the same day"
        );
        assert_eq!(
            stacks(
                &tiles,
                StackRules {
                    within: Some(3),
                    alike: true
                }
            ),
            vec![1..4],
            "either rule joins"
        );
        assert!(stacks(&tiles, StackRules::default()).is_empty());
        // Until the looks are known, the time alone joins them.
        let unknown: Vec<Tile> = tiles.iter().map(|t| Tile { look: None, ..*t }).collect();
        assert_eq!(stacks(&unknown, within(3)), vec![0..3]);
    }

    #[test]
    fn photos_taken_together_stack_unless_they_look_very_different() {
        let tile = |taken_at, look| Tile {
            taken_at,
            day: Some(1),
            look: Some(look),
            video: false,
        };
        let within = StackRules {
            within: Some(3),
            alike: false,
        };
        // Some bits apart, as a moving subject makes them: still one stack.
        let some = (1u64 << DIFFERENT_DISTANCE) - 1;
        assert_eq!(stacks(&[tile(10, 0), tile(11, some)], within), vec![0..2]);
        // One bit more, and they are pictures of different things.
        let more = (1u64 << (DIFFERENT_DISTANCE + 1)) - 1;
        assert!(stacks(&[tile(10, 0), tile(11, more)], within).is_empty());
        // A different picture between two alike ones splits them.
        assert!(stacks(&[tile(10, 0), tile(11, u64::MAX), tile(12, 0)], within).is_empty());
    }

    #[test]
    fn videos_undated_photos_and_other_days_never_stack() {
        let tile = |taken_at, day, video| Tile {
            taken_at,
            day,
            look: Some(7),
            video,
        };
        let both = StackRules {
            within: Some(10),
            alike: true,
        };
        assert!(stacks(&[tile(100, Some(1), false), tile(101, Some(1), true)], both).is_empty());
        assert!(stacks(&[tile(0, None, false), tile(0, None, false)], both).is_empty());
        // Alike, a few seconds apart, but over midnight: the time joins them.
        assert_eq!(
            stacks(
                &[tile(100, Some(1), false), tile(104, Some(2), false)],
                both
            ),
            vec![0..2]
        );
        let alike = StackRules {
            within: None,
            alike: true,
        };
        assert!(stacks(
            &[tile(100, Some(1), false), tile(104, Some(2), false)],
            alike
        )
        .is_empty());
        // No look yet: nothing to compare.
        let unknown = Tile {
            look: None,
            ..tile(100, Some(1), false)
        };
        assert!(stacks(&[unknown, unknown], alike).is_empty());
        assert!(stacks(&[], both).is_empty());
    }
}
