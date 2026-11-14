//! Light (§9.11): a contact card or a link frame blinked by a flashlight
//! and watched by a camera, for two devices in sight of each other with no
//! radio between them at all. A radio jammer does not reach it, and a
//! flashlight is seen from far away at night, but it is slow: a few bits a
//! second, minutes for a message.
//!
//! On-off keying, Manchester coded: each bit is two halves of
//! `HALF_BIT_MS`, lit then dark for a 1, dark then lit for a 0. A watcher
//! compares each bit's two halves, so the light around, or a camera
//! changing its exposure, does not decide the bits. A round, its kind, its
//! length and its bytes, is cut into blocks of `BLOCK_LEN` bytes. Each block
//! goes after a sync mark, four halves lit and two dark, longer than any
//! light in the data, with the round's id, the block's place, the last
//! place and a CRC-16. The sender blinks the round over and over; the
//! watcher keeps every block it reads right, from any pass, until it has
//! them all, so a person walking past or a shaking hand costs only the
//! blocks it hid.

use std::collections::VecDeque;

use crate::sound::{crc32, Kind};

/// How long the light holds each half of a bit: three pictures of a camera
/// at 30 a second.
pub const HALF_BIT_MS: u32 = 100;
/// The bytes of a round in each block.
pub const BLOCK_LEN: usize = 16;
/// The most blocks of a round: places are four bits.
const MAX_BLOCKS: usize = 16;
/// The most bytes a round carries, after its kind and length.
pub const MAX_LEN: usize = MAX_BLOCKS * BLOCK_LEN - 2;
/// The longest frame a round carries, with its CRC-32.
pub const MAX_FRAME_LEN: usize = MAX_LEN - 4;

/// Before each block: four halves lit, then two dark. Data is never lit for
/// more than two halves in a row.
const SYNC: [bool; 6] = [true, true, true, true, false, false];
/// A block as sent: the round's id, the block's place and the last place in
/// one byte, the bytes, and a CRC-16 of all that.
const BLOCK_SENT: usize = 2 + BLOCK_LEN + 2;
const BLOCK_BITS: usize = BLOCK_SENT * 8;
/// A block with its sync mark, in halves of a bit.
pub const BLOCK_HALVES: usize = SYNC.len() + 2 * BLOCK_BITS;

/// The light of one round of `payload`, a half bit at a time, true for lit,
/// each half for `HALF_BIT_MS`: every block after its sync mark, to blink
/// over and over. A frame gets its CRC-32 added.
pub fn pattern(kind: Kind, payload: &[u8]) -> Vec<bool> {
    let mut bytes = payload.to_vec();
    if kind == Kind::Frame {
        bytes.extend_from_slice(&crc32(payload).to_le_bytes());
    }
    round_pattern(kind.magic(), &bytes)
}

/// Rounds of `payloads`, one after another, as `pattern` gives each.
pub fn patterns(kind: Kind, payloads: &[&[u8]]) -> Vec<bool> {
    payloads
        .iter()
        .flat_map(|payload| pattern(kind, payload))
        .collect()
}

fn round_pattern(magic: u8, bytes: &[u8]) -> Vec<bool> {
    assert!(bytes.len() <= MAX_LEN, "too long for one round");
    let mut round = vec![magic, bytes.len() as u8];
    round.extend_from_slice(bytes);
    let id = round_id(&round);
    let blocks = round.len().div_ceil(BLOCK_LEN);
    round.resize(blocks * BLOCK_LEN, 0);
    let last = (blocks - 1) as u8;
    let mut out = Vec::with_capacity(blocks * BLOCK_HALVES);
    for (place, chunk) in round.chunks(BLOCK_LEN).enumerate() {
        let mut block = vec![id, (place as u8) << 4 | last];
        block.extend_from_slice(chunk);
        block.extend_from_slice(&crc16(&block).to_be_bytes());
        out.extend_from_slice(&SYNC);
        for byte in block {
            for shift in (0..8).rev() {
                let one = byte >> shift & 1 == 1;
                out.extend_from_slice(&[one, !one]);
            }
        }
    }
    out
}

/// A round's id: the low byte of the CRC-32 of its kind, length and bytes,
/// without the padding.
fn round_id(round: &[u8]) -> u8 {
    crc32(round) as u8
}

/// CRC-16/CCITT-FALSE.
pub fn crc16(bytes: &[u8]) -> u16 {
    let mut crc = 0xFFFFu16;
    for &byte in bytes {
        crc ^= u16::from(byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                crc << 1 ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

const HALF: f64 = HALF_BIT_MS as f64;
/// Pictures kept: a block with its sync mark, and a little more.
const KEEP_MS: f64 = (BLOCK_HALVES + 8) as f64 * HALF;
/// What tells lit from dark: the brightness of the last few seconds.
const LEVEL_MS: f64 = 3_000.0;
/// The least difference between lit and dark worth watching, in the 0 to
/// 255 of a picture's brightness.
pub const MIN_CONTRAST: f32 = 6.0;
/// The shortest and longest light that is a sync mark: four halves, give or
/// take a flashlight's and a camera's delays, or five after a 0.
const SYNC_MIN: f64 = 3.0 * HALF;
const SYNC_MAX: f64 = 6.5 * HALF;
/// How much of each half's ends a bit's reading leaves out, where the light
/// may still be changing.
const MARGIN: f64 = 0.2 * HALF;
/// A block's bits are read `SHIFTS` steps of `SHIFT_MS` either side of
/// where its sync mark puts them, and decoded at the `TRIED_SHIFTS` where
/// they stand out most.
const SHIFTS: i32 = 5;
const SHIFT_MS: f64 = 0.1 * HALF;
const TRIED_SHIFTS: usize = 3;
/// The least sure bits tried the other way, one or two at a time, when a
/// block's CRC-16 does not match.
const DOUBTFUL: usize = 4;
/// Rounds put together at once; the stalest goes.
const MAX_ROUNDS: usize = 8;
/// Rounds read whole, kept so their next passes are not read again.
const HEARD_KEPT: usize = 16;

/// Watches a camera's view of a flashlight for what a device in sight
/// blinks: a card, or the frames of a message. It takes how bright the
/// light looks in each picture, and when the picture was taken.
pub struct Listener {
    /// The pictures of the last `KEEP_MS`: when each was taken, in
    /// milliseconds, and how bright the light looked.
    seen: Vec<(f64, f32)>,
    /// Whether the light is lit, once there is enough to tell.
    lit: Option<bool>,
    /// When it last came on, while it is on.
    rose: Option<f64>,
    /// Where the bits start of each block whose sync mark was seen.
    blocks: VecDeque<f64>,
    rounds: Vec<Round>,
    /// Rounds read whole, by id and last place.
    heard: VecDeque<(u8, u8)>,
    progress: Progress,
}

struct Round {
    id: u8,
    last: u8,
    blocks: [Option<[u8; BLOCK_LEN]>; MAX_BLOCKS],
    /// When a block of it last came.
    touched: f64,
}

/// How a watch goes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Progress {
    /// How far apart lit and dark look lately, 0 to 255. Under
    /// `MIN_CONTRAST`, no light is seen blinking.
    pub contrast: f32,
    /// Blocks read of the round last added to, and how many it has.
    pub blocks: usize,
    pub of: usize,
}

impl Default for Listener {
    fn default() -> Self {
        Self::new()
    }
}

impl Listener {
    pub fn new() -> Self {
        Self {
            seen: Vec::new(),
            lit: None,
            rose: None,
            blocks: VecDeque::new(),
            rounds: Vec::new(),
            heard: VecDeque::new(),
            progress: Progress::default(),
        }
    }

    pub fn progress(&self) -> Progress {
        self.progress
    }

    /// Takes in pictures, in the order taken: when each was taken, in
    /// milliseconds from any start, and how bright the light looks in it, 0
    /// to 255. Returns the rounds read whole: cards, and frames whose CRC-32
    /// matched, each once.
    pub fn push(&mut self, pictures: &[(f64, f32)]) -> Vec<(Kind, Vec<u8>)> {
        let mut out = Vec::new();
        for &(time, level) in pictures {
            if !time.is_finite() || !level.is_finite() {
                continue;
            }
            if let Some(&(last, _)) = self.seen.last() {
                if time < last - 1_000.0 {
                    // The clock went back: what was seen no longer lines up.
                    self.seen.clear();
                    self.blocks.clear();
                    self.lit = None;
                    self.rose = None;
                } else if time <= last {
                    continue;
                }
            }
            self.see(time, level);
            out.extend(self.read(time));
        }
        out
    }

    fn see(&mut self, time: f64, level: f32) {
        let before = self.seen.last().copied();
        self.seen.push((time, level));
        let old = self.seen.partition_point(|&(at, _)| at < time - KEEP_MS);
        self.seen.drain(..old);

        let from = self.seen.partition_point(|&(at, _)| at < time - LEVEL_MS);
        let recent = &self.seen[from..];
        if recent.len() < 8 || time - recent[0].0 < LEVEL_MS / 2.0 {
            return;
        }
        let mut levels: Vec<f32> = recent.iter().map(|&(_, level)| level).collect();
        levels.sort_by(f32::total_cmp);
        let dark = levels[levels.len() / 10];
        let bright = levels[levels.len() * 9 / 10];
        self.progress.contrast = bright - dark;
        if bright - dark < MIN_CONTRAST {
            self.lit = None;
            self.rose = None;
            return;
        }
        let middle = (bright + dark) / 2.0;
        let band = (bright - dark) * 0.15;
        let lit = match self.lit {
            Some(true) => level > middle - band,
            Some(false) => level > middle + band,
            None => level > middle,
        };
        if self.lit == Some(lit) {
            return;
        }
        // Where the light crossed the middle, between this picture and the one before.
        let edge = match before {
            Some((at, was)) if (level - was).abs() > f32::EPSILON => {
                let share = ((middle - was) / (level - was)).clamp(0.0, 1.0);
                at + (time - at) * f64::from(share)
            }
            _ => time,
        };
        match (self.lit, lit) {
            (Some(false), true) => self.rose = Some(edge),
            (Some(true), false) => {
                if let Some(rose) = self.rose.take() {
                    if (SYNC_MIN..=SYNC_MAX).contains(&(edge - rose)) {
                        self.blocks.push_back(edge + 2.0 * HALF);
                    }
                }
            }
            _ => self.rose = None,
        }
        self.lit = Some(lit);
    }

    /// Reads the blocks whose pictures have all come by `now`.
    fn read(&mut self, now: f64) -> Vec<(Kind, Vec<u8>)> {
        let mut out = Vec::new();
        while let Some(&start) = self.blocks.front() {
            if now < start + (2 * BLOCK_BITS) as f64 * HALF {
                break;
            }
            self.blocks.pop_front();
            if let Some(block) = self.block(start) {
                out.extend(self.take(&block, now));
            }
        }
        out
    }

    /// The block whose bits start about `start`, if its CRC-16 matches, with
    /// up to two of its least sure bits turned. A flashlight that comes on
    /// later than it goes off moves the bits from where the sync mark puts
    /// them, so they are read a little earlier and later too, and tried
    /// where their halves differ most.
    fn block(&self, start: f64) -> Option<[u8; BLOCK_SENT]> {
        let mut readings: Vec<(f32, [f32; BLOCK_BITS])> = (-SHIFTS..=SHIFTS)
            .filter_map(|shift| {
                let start = start + f64::from(shift) * SHIFT_MS;
                let mut soft = [0f32; BLOCK_BITS];
                for (bit, value) in soft.iter_mut().enumerate() {
                    let at = start + (2 * bit) as f64 * HALF;
                    *value = self.brightness(at, at + HALF)?
                        - self.brightness(at + HALF, at + 2.0 * HALF)?;
                }
                Some((soft.iter().map(|value| value.abs()).sum(), soft))
            })
            .collect();
        readings.sort_by(|a, b| b.0.total_cmp(&a.0));
        readings
            .iter()
            .take(TRIED_SHIFTS)
            .find_map(|(_, soft)| decode(soft))
    }

    /// How bright the light looked from `from` to `to`: the pictures inside,
    /// leaving out the ends, or else all of it, or else between the
    /// pictures either side.
    fn brightness(&self, from: f64, to: f64) -> Option<f32> {
        let mean = |from: f64, to: f64| {
            let start = self.seen.partition_point(|&(at, _)| at < from);
            let inside: Vec<f32> = self.seen[start..]
                .iter()
                .take_while(|&&(at, _)| at <= to)
                .map(|&(_, level)| level)
                .collect();
            (!inside.is_empty()).then(|| inside.iter().sum::<f32>() / inside.len() as f32)
        };
        mean(from + MARGIN, to - MARGIN)
            .or_else(|| mean(from, to))
            .or_else(|| {
                let middle = (from + to) / 2.0;
                let after = self.seen.partition_point(|&(at, _)| at < middle);
                let (&(t0, l0), &(t1, l1)) =
                    (self.seen.get(after.checked_sub(1)?)?, self.seen.get(after)?);
                let share = ((middle - t0) / (t1 - t0)) as f32;
                Some(l0 + (l1 - l0) * share)
            })
    }

    /// Keeps a block read right; gives its round once it is whole.
    fn take(&mut self, block: &[u8; BLOCK_SENT], now: f64) -> Option<(Kind, Vec<u8>)> {
        let id = block[0];
        let place = usize::from(block[1] >> 4);
        let last = block[1] & 0x0F;
        if self.heard.contains(&(id, last)) {
            return None;
        }
        let index = match self
            .rounds
            .iter()
            .position(|round| round.id == id && round.last == last)
        {
            Some(index) => index,
            None => {
                if self.rounds.len() == MAX_ROUNDS {
                    let stalest = (0..self.rounds.len()).min_by(|&a, &b| {
                        self.rounds[a].touched.total_cmp(&self.rounds[b].touched)
                    })?;
                    self.rounds.remove(stalest);
                }
                self.rounds.push(Round {
                    id,
                    last,
                    blocks: [None; MAX_BLOCKS],
                    touched: now,
                });
                self.rounds.len() - 1
            }
        };
        let round = &mut self.rounds[index];
        round.blocks[place] = Some(block[2..2 + BLOCK_LEN].try_into().ok()?);
        round.touched = now;
        let count = usize::from(last) + 1;
        let have = round.blocks[..count]
            .iter()
            .filter(|block| block.is_some())
            .count();
        self.progress.blocks = have;
        self.progress.of = count;
        if have < count {
            return None;
        }
        let round = self.rounds.remove(index);
        let bytes: Vec<u8> = round.blocks[..count]
            .iter()
            .flatten()
            .flatten()
            .copied()
            .collect();
        let kind = Kind::of(bytes[0])?;
        let len = usize::from(bytes[1]);
        // The length needs just these blocks, and the id is the round's own:
        // blocks of two rounds that share an id do not make one.
        if (len + 2).div_ceil(BLOCK_LEN) != count || round_id(&bytes[..2 + len]) != id {
            return None;
        }
        let mut payload = bytes[2..2 + len].to_vec();
        if kind == Kind::Frame {
            let crc = payload.split_off(payload.len().checked_sub(4)?);
            if crc32(&payload).to_le_bytes()[..] != crc[..] {
                return None;
            }
        }
        self.heard.push_back((id, last));
        if self.heard.len() > HEARD_KEPT {
            self.heard.pop_front();
        }
        Some((kind, payload))
    }
}

/// A block from how much brighter each bit's first half looked than its
/// second, if its CRC-16 matches as read or with one or two of the
/// `DOUBTFUL` least sure bits turned.
fn decode(soft: &[f32; BLOCK_BITS]) -> Option<[u8; BLOCK_SENT]> {
    let bytes = |turned: &[usize]| {
        let mut out = [0u8; BLOCK_SENT];
        for (bit, &value) in soft.iter().enumerate() {
            if (value > 0.0) != turned.contains(&bit) {
                out[bit / 8] |= 0x80 >> (bit % 8);
            }
        }
        out
    };
    let right = |block: &[u8; BLOCK_SENT]| {
        let (body, crc) = block.split_at(BLOCK_SENT - 2);
        crc16(body).to_be_bytes()[..] == crc[..] && block[1] >> 4 <= block[1] & 0x0F
    };
    let mut doubtful: Vec<usize> = (0..BLOCK_BITS).collect();
    doubtful.sort_by(|&a, &b| soft[a].abs().total_cmp(&soft[b].abs()));
    doubtful.truncate(DOUBTFUL);
    let mut tries = vec![Vec::new()];
    tries.extend(doubtful.iter().map(|&bit| vec![bit]));
    for (index, &first) in doubtful.iter().enumerate() {
        tries.extend(
            doubtful[index + 1..]
                .iter()
                .map(|&second| vec![first, second]),
        );
    }
    tries.iter().map(|turned| bytes(turned)).find(right)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Noise(u64);

    impl Noise {
        /// Between -1 and 1.
        fn next(&mut self) -> f32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0
        }
    }

    /// A flashlight and the camera that films it.
    #[derive(Clone, Copy)]
    struct Scene {
        /// Milliseconds between pictures, and how far each may be off.
        frame_ms: f64,
        jitter: f64,
        /// How long each picture takes the light in, before it is stamped.
        exposure: f64,
        /// How late the flashlight comes on and goes off.
        rise: f64,
        fall: f64,
        /// Brightness lit and dark, and the noise on each picture.
        lit: f32,
        dark: f32,
        noise: f32,
    }

    const SHAKY: Scene = Scene {
        frame_ms: 1000.0 / 30.0,
        jitter: 6.0,
        exposure: 30.0,
        rise: 40.0,
        fall: 10.0,
        lit: 120.0,
        dark: 70.0,
        noise: 4.0,
    };

    /// When the flashlight is on, blinking `pattern` `passes` times from 0.
    fn lit_spans(scene: &Scene, pattern: &[bool], passes: usize) -> Vec<(f64, f64)> {
        let mut spans = Vec::new();
        let mut on: Option<f64> = None;
        let halves = pattern.iter().cycle().take(pattern.len() * passes);
        for (index, &lit) in halves.chain([false].iter()).enumerate() {
            let at = index as f64 * HALF;
            match (on, lit) {
                (None, true) => on = Some(at),
                (Some(start), false) => {
                    spans.push((start + scene.rise, at + scene.fall));
                    on = None;
                }
                _ => {}
            }
        }
        spans
    }

    /// How much of `from` to `to` the light was on.
    fn on_share(spans: &[(f64, f64)], from: f64, to: f64) -> f32 {
        let first = spans.partition_point(|&(_, end)| end <= from);
        let on: f64 = spans[first..]
            .iter()
            .take_while(|&&(start, _)| start < to)
            .map(|&(start, end)| end.min(to) - start.max(from))
            .filter(|&overlap| overlap > 0.0)
            .sum();
        (on / (to - from)) as f32
    }

    /// The pictures a camera takes from `from` to `to` of a flashlight
    /// blinking `pattern`, its clock 12 345 ms ahead of the flashlight's.
    fn film(
        scene: &Scene,
        pattern: &[bool],
        passes: usize,
        from: f64,
        to: f64,
        seed: u64,
    ) -> Vec<(f64, f32)> {
        let spans = lit_spans(scene, pattern, passes);
        let mut noise = Noise(seed);
        let mut pictures = Vec::new();
        let mut index = (from / scene.frame_ms) as usize;
        loop {
            let at = index as f64 * scene.frame_ms + f64::from(noise.next()) * scene.jitter;
            if at > to {
                return pictures;
            }
            let share = on_share(&spans, at - scene.exposure, at);
            let level = scene.dark + (scene.lit - scene.dark) * share + noise.next() * scene.noise;
            pictures.push((at + 12_345.0, level));
            index += 1;
        }
    }

    /// Gives `pictures` to `listener` in pieces of about a quarter second, as a page would.
    fn watch(listener: &mut Listener, pictures: &[(f64, f32)]) -> Vec<(Kind, Vec<u8>)> {
        pictures
            .chunks(8)
            .flat_map(|chunk| listener.push(chunk))
            .collect()
    }

    fn payload(len: u8) -> Vec<u8> {
        (0..len)
            .map(|byte| byte.wrapping_mul(37).wrapping_add(11))
            .collect()
    }

    fn duration(pattern: &[bool]) -> f64 {
        pattern.len() as f64 * HALF
    }

    #[test]
    fn a_round_is_blocks_after_sync_marks() {
        let frame = payload(143);
        let pattern = pattern(Kind::Frame, &frame);
        // 2 + 143 + 4 bytes: ten blocks.
        assert_eq!(pattern.len(), 10 * BLOCK_HALVES);
        for block in pattern.chunks(BLOCK_HALVES) {
            assert_eq!(block[..SYNC.len()], SYNC);
            // No light in the data as long as a sync mark's.
            let longest = block[SYNC.len()..]
                .split(|&lit| !lit)
                .map(<[bool]>::len)
                .max();
            assert_eq!(longest, Some(2));
        }
    }

    #[test]
    fn reads_a_frame_through_a_shaky_camera() {
        let frame = payload(143);
        let pattern = pattern(Kind::Frame, &frame);
        let length = duration(&pattern);
        // From a third of the way into the first pass to the end of the second.
        let pictures = film(&SHAKY, &pattern, 2, length / 3.0, 2.0 * length, 7);
        let mut listener = Listener::new();
        assert_eq!(watch(&mut listener, &pictures), vec![(Kind::Frame, frame)]);
        let progress = listener.progress();
        assert_eq!((progress.blocks, progress.of), (10, 10));
        assert!(progress.contrast > 40.0);
    }

    #[test]
    fn reads_a_card_at_sixty_pictures_a_second() {
        let card = payload(200);
        let pattern = pattern(Kind::Card, &card);
        let scene = Scene {
            frame_ms: 1000.0 / 60.0,
            jitter: 3.0,
            exposure: 8.0,
            ..SHAKY
        };
        let length = duration(&pattern);
        let pictures = film(&scene, &pattern, 2, length / 2.0, 2.0 * length, 11);
        assert_eq!(
            watch(&mut Listener::new(), &pictures),
            vec![(Kind::Card, card)]
        );
    }

    #[test]
    fn reads_a_slow_flashlight() {
        let frame = payload(90);
        let pattern = pattern(Kind::Frame, &frame);
        let scene = Scene {
            rise: 70.0,
            fall: 5.0,
            jitter: 10.0,
            ..SHAKY
        };
        let pictures = film(&scene, &pattern, 2, 0.0, 2.0 * duration(&pattern), 29);
        assert_eq!(
            watch(&mut Listener::new(), &pictures),
            vec![(Kind::Frame, frame)]
        );
    }

    #[test]
    fn reads_a_faint_light() {
        let frame = payload(40);
        let pattern = pattern(Kind::Frame, &frame);
        let scene = Scene {
            lit: 80.0,
            dark: 70.0,
            noise: 1.5,
            ..SHAKY
        };
        let pictures = film(&scene, &pattern, 2, 0.0, 2.0 * duration(&pattern), 3);
        assert_eq!(
            watch(&mut Listener::new(), &pictures),
            vec![(Kind::Frame, frame)]
        );
    }

    #[test]
    fn reads_while_the_camera_brightens_the_picture() {
        let frame = payload(60);
        let pattern = pattern(Kind::Frame, &frame);
        let length = duration(&pattern);
        // The whole picture brightens by half, steadily, over two passes.
        let pictures: Vec<(f64, f32)> = film(&SHAKY, &pattern, 2, 0.0, 2.0 * length, 5)
            .into_iter()
            .map(|(at, level)| (at, level * (1.0 + ((at - 12_345.0) / length / 4.0) as f32)))
            .collect();
        assert_eq!(
            watch(&mut Listener::new(), &pictures),
            vec![(Kind::Frame, frame)]
        );
    }

    #[test]
    fn keeps_the_blocks_it_read_until_the_hidden_ones_come_again() {
        let frame = payload(143);
        let pattern = pattern(Kind::Frame, &frame);
        let length = duration(&pattern);
        let mut pictures = film(&SHAKY, &pattern, 3, 0.0, 3.0 * length, 13);
        // Someone stands in the way for a third of the first pass.
        for picture in &mut pictures {
            let at = picture.0 - 12_345.0;
            if at > length * 0.3 && at < length * 0.63 {
                picture.1 = 65.0;
            }
        }
        let mut listener = Listener::new();
        let first = pictures.partition_point(|&(at, _)| at - 12_345.0 < length);
        assert!(watch(&mut listener, &pictures[..first]).is_empty());
        let progress = listener.progress();
        assert!(progress.blocks < progress.of);
        // The second pass fills the gap; the third is not read again.
        assert_eq!(
            watch(&mut listener, &pictures[first..]),
            vec![(Kind::Frame, frame)]
        );
    }

    #[test]
    fn a_frame_whose_crc_is_wrong_reads_as_nothing() {
        let frame = payload(30);
        let mut bytes = frame.clone();
        bytes.extend_from_slice(&crc32(&frame).wrapping_add(1).to_le_bytes());
        let pattern = round_pattern(Kind::Frame.magic(), &bytes);
        let pictures = film(&SHAKY, &pattern, 2, 0.0, 2.0 * duration(&pattern), 17);
        assert!(watch(&mut Listener::new(), &pictures).is_empty());
    }

    #[test]
    fn a_flickering_light_reads_as_nothing() {
        let mut noise = Noise(23);
        let mut lit = false;
        let pictures: Vec<(f64, f32)> = (0..20_000)
            .map(|index| {
                if noise.next() > 0.7 {
                    lit = !lit;
                }
                (
                    index as f64 * 33.3,
                    if lit { 150.0 } else { 40.0 } + noise.next() * 10.0,
                )
            })
            .collect();
        assert!(watch(&mut Listener::new(), &pictures).is_empty());
    }

    #[test]
    fn turning_a_doubtful_bit_mends_a_block() {
        let mut block = [0u8; BLOCK_SENT];
        block[0] = 0x5A;
        block[1] = 0x23;
        for (index, byte) in block[2..2 + BLOCK_LEN].iter_mut().enumerate() {
            *byte = index as u8 * 9;
        }
        let crc = crc16(&block[..BLOCK_SENT - 2]).to_be_bytes();
        block[BLOCK_SENT - 2..].copy_from_slice(&crc);
        let mut soft = [0f32; BLOCK_BITS];
        for (bit, value) in soft.iter_mut().enumerate() {
            *value = if block[bit / 8] & 0x80 >> (bit % 8) != 0 {
                30.0
            } else {
                -30.0
            };
        }
        soft[77] = -soft[77] / 20.0;
        soft[130] = -soft[130] / 15.0;
        assert_eq!(decode(&soft), Some(block));
        // A wrong bit read surely is not mended.
        soft[3] = -soft[3];
        assert_eq!(decode(&soft), None);
    }
}
