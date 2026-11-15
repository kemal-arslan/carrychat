//! Sound (§3.2, §9.10): a few hundred bytes, a contact card or a link
//! frame, played through a speaker and heard through a microphone, so two
//! devices side by side exchange cards, or messages, with nothing else in
//! common.
//!
//! Tones 46.875 Hz apart, in channels of 16. Each symbol sounds one tone per
//! channel, a nibble each. A round plays in one of four bands: six channels
//! from 1875 Hz up to about 6.3 kHz, where phone speakers and microphones
//! work well, three bytes to a 100 ms symbol; three channels from 18 kHz up
//! to about 20.2 kHz, above what most adults hear, a byte and a half to a
//! 64 ms symbol; or, to reach furthest, one channel that moves between two
//! places, from 1500 Hz to about 2.95 kHz with half a byte to a 100 ms
//! symbol, or unheard, from 18 kHz to about 19.45 kHz with half a byte to a
//! 128 ms symbol. A speaker plays its loudest one tone at a time, so a far
//! band's tone is about 16 dB louder than each of the six of the first, and
//! 10 dB louder than each of the three of the second; and it does not
//! distort the way several tones do at full volume. Moving between two
//! places, a tone does not land where the room still echoes the last.
//! A start marker of four symbols comes first, then a header with the
//! length, then the bytes. The sender plays it over and over; the
//! listener hears both bands and needs one clean round. A card verifies its
//! own signature; a frame carries a CRC-32, since a corrupt one could
//! otherwise be carried on.

use std::collections::VecDeque;
use std::f64::consts::TAU;

/// The rate `samples` and `wav` play at. `Listener` takes any rate.
pub const SAMPLE_RATE: u32 = 48_000;
/// The most bytes one round carries.
pub const MAX_LEN: usize = 255;
/// The longest frame one round carries, with its CRC-32.
pub const MAX_FRAME_LEN: usize = MAX_LEN - 4;

/// What a round carries, as its header's first byte says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A contact card (§3.1), which its signature checks.
    Card,
    /// A link frame (§7), followed by the CRC-32 of the frame.
    Frame,
}

impl Kind {
    /// The byte that names it, over sound and over light (§9.11).
    pub(crate) fn magic(self) -> u8 {
        match self {
            Self::Card => 0xCC,
            Self::Frame => 0xCD,
        }
    }

    pub(crate) fn of(magic: u8) -> Option<Self> {
        [Self::Card, Self::Frame]
            .into_iter()
            .find(|kind| kind.magic() == magic)
    }
}

/// Where a round's tones lie.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Band {
    /// 1875 Hz to about 6.3 kHz, which every speaker and microphone carries.
    #[default]
    Audible,
    /// 18 kHz to about 20.2 kHz, above what most adults hear. Not every
    /// speaker plays it, and it carries less a second.
    Inaudible,
    /// 1500 Hz to about 2.95 kHz, one tone at a time: the furthest reach,
    /// at a sixth of the audible band's speed.
    Far,
    /// 18 kHz to about 19.45 kHz, one tone at a time, in longer symbols: as
    /// far as sound above hearing reaches, at a quarter of the inaudible
    /// band's speed.
    FarInaudible,
}

impl Band {
    const ALL: [Self; 4] = [
        Self::Audible,
        Self::Inaudible,
        Self::Far,
        Self::FarInaudible,
    ];

    /// The lowest tone.
    fn base_hz(self) -> f64 {
        match self {
            Self::Audible => 1875.0,
            Self::Inaudible | Self::FarInaudible => 18_000.0,
            Self::Far => 1500.0,
        }
    }

    /// Channels of `TONES` tones, each sounding a nibble a symbol.
    fn channels(self) -> usize {
        match self {
            Self::Audible => 6,
            Self::Inaudible => 3,
            Self::Far | Self::FarInaudible => 1,
        }
    }

    /// Places the channels move between, one symbol to the next.
    fn hops(self) -> usize {
        match self {
            Self::Audible | Self::Inaudible => 1,
            Self::Far | Self::FarInaudible => 2,
        }
    }

    fn tones(self) -> usize {
        self.hops() * self.channels() * TONES
    }

    /// Tone `tone` of channel `channel` in symbol `symbol`, among the band's
    /// tones.
    fn tone(self, symbol: usize, channel: usize, tone: usize) -> usize {
        ((symbol % self.hops()) * self.channels() + channel) * TONES + tone
    }

    /// The energies of the place symbol `symbol` sounds in, out of all the
    /// band's.
    fn place(self, energies: &[f32], symbol: usize) -> &[f32] {
        let len = self.channels() * TONES;
        let from = (symbol % self.hops()) * len;
        &energies[from..from + len]
    }

    fn symbol_secs(self) -> f64 {
        match self {
            Self::Audible | Self::Far => 0.1,
            // Three and six cycles of `STEP_HZ`, so every tone runs a whole
            // number of cycles and ends where the next symbol's begins.
            Self::Inaudible => 0.064,
            Self::FarInaudible => 0.128,
        }
    }

    /// What the listener skips at the start of each symbol, where the room
    /// still echoes the last one. High tones die away sooner; far away,
    /// the echo is as loud as the tone.
    fn guard_secs(self) -> f64 {
        match self {
            Self::Audible => 0.03,
            Self::Inaudible => 0.02,
            Self::Far => 0.035,
            Self::FarInaudible => 0.025,
        }
    }

    /// How long sound fades in and out over. The audible bands fade every
    /// symbol, so they do not click. The other runs on from one symbol into
    /// the next and fades only at the round's ends, more slowly: a click
    /// would be heard although the tones are not.
    fn ramp_secs(self) -> f64 {
        match self {
            Self::Audible | Self::Far => 0.005,
            Self::Inaudible | Self::FarInaudible => 0.01,
        }
    }

    fn fades_every_symbol(self) -> bool {
        matches!(self, Self::Audible | Self::Far)
    }

    fn frequency(self, tone: usize) -> f64 {
        self.base_hz() + STEP_HZ * tone as f64
    }
}

/// CRC-32 as Ethernet and zip compute it.
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// Tones this far apart are orthogonal over a window of 1/STEP_HZ seconds.
const STEP_HZ: f64 = 46.875;
const TONES: usize = 16;
/// Silence before and after each round.
const LEAD_SECS: f64 = 0.1;
const TAIL_SECS: f64 = 0.4;
/// The start marker alternates two symbols, each with two tones in every
/// channel, which no data symbol has.
const MARKER: [[usize; 2]; 2] = [[0, 8], [4, 12]];
const MARKER_SYMBOLS: usize = 4;
/// Share of the energy in the marker's tones: 1 for a clean marker, about
/// 1/8 for noise and well under this for most data. A far band shares its
/// marker's energy with the noise of twice as many tones, about 1/16 alone,
/// so less suffices there.
const MARKER_THRESHOLD: f32 = 0.5;
const FAR_MARKER_THRESHOLD: f32 = 0.3;
/// How much sound the listener keeps: longer than the longest round, which
/// takes 67 seconds in the far band above hearing.
const KEEP_SECS: f64 = 70.0;

/// One round of `payload` in `band` as samples at `SAMPLE_RATE`, between -1
/// and 1. A frame gets its CRC-32 added.
pub fn samples(band: Band, kind: Kind, payload: &[u8]) -> Vec<f32> {
    let mut bytes = payload.to_vec();
    if kind == Kind::Frame {
        bytes.extend_from_slice(&crc32(payload).to_le_bytes());
    }
    round(band, kind.magic(), &bytes)
}

fn round(band: Band, magic: u8, bytes: &[u8]) -> Vec<f32> {
    assert!(bytes.len() <= MAX_LEN, "too long for one round");
    let rate = f64::from(SAMPLE_RATE);
    let mut out = vec![0.0; (LEAD_SECS * rate) as usize];
    let symbols = symbols(band, magic, bytes);
    let last = symbols.len() - 1;
    for (index, tones) in symbols.iter().enumerate() {
        let fades = band.fades_every_symbol();
        sound(
            &mut out,
            band,
            tones,
            (fades || index == 0, fades || index == last),
        );
    }
    out.resize(out.len() + (TAIL_SECS * rate) as usize, 0.0);
    out
}

/// Rounds of `payloads` in `band`, one after another, as a 16-bit mono WAV
/// file.
pub fn wav(band: Band, kind: Kind, payloads: &[&[u8]]) -> Vec<u8> {
    let samples: Vec<f32> = payloads
        .iter()
        .flat_map(|payload| samples(band, kind, payload))
        .collect();
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + samples.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    out.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        out.extend_from_slice(&((sample * f32::from(i16::MAX)) as i16).to_le_bytes());
    }
    out
}

/// The tones of each symbol: the marker, then the header and the bytes, a
/// nibble to each channel, the last symbol zero-padded. The three header
/// bytes fill whole symbols in every band.
fn symbols(band: Band, magic: u8, payload: &[u8]) -> Vec<Vec<usize>> {
    let channels = band.channels();
    let mut symbols: Vec<Vec<usize>> = (0..MARKER_SYMBOLS)
        .map(|index| {
            (0..channels)
                .flat_map(|channel| MARKER[index % 2].map(|tone| band.tone(index, channel, tone)))
                .collect()
        })
        .collect();
    let len = payload.len() as u8;
    let nibbles: Vec<u8> = [magic, len, !len]
        .iter()
        .chain(payload)
        .flat_map(|byte| [byte >> 4, byte & 0x0F])
        .collect();
    for nibbles in nibbles.chunks(channels) {
        let index = symbols.len();
        symbols.push(
            (0..channels)
                .map(|channel| {
                    let nibble = nibbles.get(channel).copied().unwrap_or(0);
                    band.tone(index, channel, usize::from(nibble))
                })
                .collect(),
        );
    }
    symbols
}

/// Appends one symbol: its tones at equal loudness, each channel's from its
/// own phase so the channels do not all peak together, faded in or out at
/// the ends `fades` says.
fn sound(out: &mut Vec<f32>, band: Band, tones: &[usize], fades: (bool, bool)) {
    let rate = f64::from(SAMPLE_RATE);
    let len = (band.symbol_secs() * rate).round() as usize;
    let ramp = band.ramp_secs() * rate;
    let loudness = 0.9 / tones.len() as f64;
    out.extend((0..len).map(|index| {
        let at = index as f64;
        let rise = if fades.0 { at / ramp } else { 1.0 };
        let fall = if fades.1 {
            (len as f64 - at) / ramp
        } else {
            1.0
        };
        let fade = 0.5 - 0.5 * (rise.min(fall).min(1.0) * std::f64::consts::PI).cos();
        let sum: f64 = tones
            .iter()
            .map(|&tone| {
                // Each channel from its own phase, the same in either place,
                // so a band that does not fade runs on without a step.
                let channel = (tone / TONES) % band.channels();
                let phase = TAU * channel as f64 / band.channels() as f64;
                (TAU * band.frequency(tone) * at / rate + phase).sin()
            })
            .sum();
        (sum * loudness * fade) as f32
    }));
}

/// Listens for rounds in a microphone's samples, in every band below half
/// its rate.
pub struct Listener {
    windows: Windows,
    searches: Vec<Search>,
}

/// The sound heard, as overlapping windows and each window's energy in
/// every tone of every band heard.
struct Windows {
    rate: f64,
    /// Samples per window, and between windows.
    len: usize,
    hop: usize,
    /// `2·cos(ω)` of each tone, band after band, for Goertzel's algorithm.
    coefficients: Vec<f64>,
    /// Samples not yet analysed.
    pending: VecDeque<f32>,
    /// Each window's energy in every tone. `energies[0]` is window `first`.
    energies: VecDeque<Vec<f32>>,
    first: usize,
}

/// Looking for rounds in one band.
struct Search {
    band: Band,
    /// Where the band's tones start among a window's energies.
    offset: usize,
    /// Where the next marker may start, as a window index.
    searched: usize,
    /// A run of windows that each could start a marker, still growing.
    run: Option<usize>,
    /// Markers found whose rounds are still being heard, by the sample
    /// they start at.
    rounds: Vec<f64>,
}

impl Listener {
    /// A listener for samples at `rate` per second.
    pub fn new(rate: u32) -> Self {
        let rate = f64::from(rate);
        let len = (rate / STEP_HZ).round() as usize;
        let mut coefficients = Vec::new();
        let mut searches = Vec::new();
        for band in Band::ALL {
            if band.frequency(band.tones() - 1) >= rate / 2.0 {
                continue;
            }
            searches.push(Search {
                band,
                offset: coefficients.len(),
                searched: 0,
                run: None,
                rounds: Vec::new(),
            });
            coefficients.extend(
                (0..band.tones()).map(|tone| 2.0 * (TAU * band.frequency(tone) / rate).cos()),
            );
        }
        Self {
            windows: Windows {
                rate,
                len,
                hop: len / 4,
                coefficients,
                pending: VecDeque::new(),
                energies: VecDeque::new(),
                first: 0,
            },
            searches,
        }
    }

    /// Takes in samples. Returns what a round carries once one has been
    /// heard whole, in either band: a card as heard, or a frame whose
    /// CRC-32 matched.
    pub fn push(&mut self, samples: &[f32]) -> Option<(Kind, Vec<u8>)> {
        self.windows.push(samples);
        for search in &mut self.searches {
            search.find_markers(&self.windows);
        }
        self.searches
            .iter_mut()
            .find_map(|search| search.decode_rounds(&self.windows))
    }
}

impl Windows {
    fn push(&mut self, samples: &[f32]) {
        self.pending.extend(samples);
        while self.pending.len() >= self.len {
            let window = &self.pending.make_contiguous()[..self.len];
            let energies = energies(&self.coefficients, window);
            self.energies.push_back(energies);
            self.pending.drain(..self.hop);
        }
        let keep = (KEEP_SECS * self.rate) as usize / self.hop;
        while self.energies.len() > keep {
            self.energies.pop_front();
            self.first += 1;
        }
    }

    /// One past the last window heard.
    fn end(&self) -> usize {
        self.first + self.energies.len()
    }
}

/// The energy in each tone, by its coefficient, over `window`.
fn energies(coefficients: &[f64], window: &[f32]) -> Vec<f32> {
    coefficients
        .iter()
        .map(|coefficient| {
            let (mut previous, mut before) = (0.0, 0.0);
            for &sample in window {
                let next = f64::from(sample) + coefficient * previous - before;
                before = previous;
                previous = next;
            }
            (previous * previous + before * before - coefficient * previous * before) as f32
        })
        .collect()
}

impl Search {
    /// The mean energy of each of the band's tones over symbol `index` of a
    /// round starting at sample `start`, or None until it has been heard
    /// whole.
    fn symbol(&self, windows: &Windows, start: f64, index: usize) -> Option<Vec<f32>> {
        let len = self.band.symbol_secs() * windows.rate;
        let from = start + len * index as f64 + self.band.guard_secs() * windows.rate;
        let to = start + len * (index + 1) as f64 - windows.len as f64;
        let first = (from / windows.hop as f64).ceil().max(0.0) as usize;
        let last = (to / windows.hop as f64).floor() as usize;
        if last < first || last >= windows.end() || first < windows.first {
            return None;
        }
        let tones = self.offset..self.offset + self.band.tones();
        let mut sum = vec![0.0; self.band.tones()];
        for energies in windows
            .energies
            .range(first - windows.first..=last - windows.first)
        {
            for (total, energy) in sum.iter_mut().zip(&energies[tones.clone()]) {
                *total += energy;
            }
        }
        Some(sum)
    }

    /// How much like marker symbol `index` a symbol sounds, from 0 to 1: the
    /// share of each channel's energy in the marker's tones, about 1/8 for
    /// noise. In a far band the share is of the whole band's energy: the
    /// place's alone would stay as high with the symbol read tens of
    /// milliseconds late, when the next symbol sounds in the other place,
    /// and the round would be timed wrong. Its data can sound like the
    /// marker by chance, one tone a symbol; such a round's header does not
    /// read right, and it is dropped.
    fn marker_score(band: Band, energies: &[f32], index: usize) -> f32 {
        let tones = MARKER[index % 2];
        let place = band.place(energies, index);
        if band.hops() > 1 {
            let total: f32 = energies.iter().sum();
            return if total > 0.0 {
                (place[tones[0]] + place[tones[1]]) / total
            } else {
                0.0
            };
        }
        let mut score = 0.0;
        for channel in place.chunks(TONES) {
            let total: f32 = channel.iter().sum();
            if total > 0.0 {
                score += (channel[tones[0]] + channel[tones[1]]) / total;
            }
        }
        score / (place.len() / TONES) as f32
    }

    /// Looks for the start marker at every window not yet searched. Each run
    /// of windows where it could start ends in one round, timed from the
    /// middle of the run.
    fn find_markers(&mut self, windows: &Windows) {
        self.searched = self.searched.max(windows.first);
        while self.searched < windows.end() {
            let start = (self.searched * windows.hop) as f64;
            // Every symbol must sound like the marker, or in a far band, the
            // four on average: one symbol in a notch of the speaker's sound
            // does not lose the round.
            let mut scores = [0f32; MARKER_SYMBOLS];
            for (index, score) in scores.iter_mut().enumerate() {
                match self.symbol(windows, start, index) {
                    Some(energies) => *score = Self::marker_score(self.band, &energies, index),
                    None => return,
                }
            }
            let score = if self.band.hops() > 1 {
                scores.iter().sum::<f32>() / MARKER_SYMBOLS as f32
            } else {
                scores.iter().copied().fold(f32::MAX, f32::min)
            };
            let threshold = if self.band.hops() > 1 {
                FAR_MARKER_THRESHOLD
            } else {
                MARKER_THRESHOLD
            };
            if score >= threshold {
                self.run.get_or_insert(self.searched);
            } else if let Some(run) = self.run.take() {
                let middle = (run + self.searched - 1) as f64 / 2.0;
                self.rounds.push(middle * windows.hop as f64);
            }
            self.searched += 1;
        }
    }

    /// Reads each round found once it has been heard whole. A round whose
    /// header does not read right is dropped, and so is one whose start the
    /// listener no longer keeps.
    fn decode_rounds(&mut self, windows: &Windows) -> Option<(Kind, Vec<u8>)> {
        let mut index = 0;
        while index < self.rounds.len() {
            let start = self.rounds[index];
            let decoded = if start < (windows.first * windows.hop) as f64 {
                Round::Bad
            } else {
                self.decode(windows, start)
            };
            match decoded {
                Round::Waiting => index += 1,
                Round::Bad => {
                    self.rounds.remove(index);
                }
                Round::Heard(kind, bytes) => {
                    self.rounds.remove(index);
                    return Some((kind, bytes));
                }
            }
        }
        None
    }

    /// The first `count` bytes after the marker of a round starting at
    /// `start`, each channel's loudest tone a nibble, or None until they
    /// have been heard.
    fn bytes(&self, windows: &Windows, start: f64, count: usize) -> Option<Vec<u8>> {
        let mut nibbles = Vec::with_capacity(count * 2 + self.band.channels());
        let mut index = MARKER_SYMBOLS;
        while nibbles.len() < count * 2 {
            let energies = self.symbol(windows, start, index)?;
            nibbles.extend(
                self.band
                    .place(&energies, index)
                    .chunks(TONES)
                    .map(|channel| {
                        let (loudest, _) = channel.iter().enumerate().fold(
                            (0, f32::MIN),
                            |best, (tone, &energy)| {
                                if energy > best.1 {
                                    (tone, energy)
                                } else {
                                    best
                                }
                            },
                        );
                        loudest as u8
                    }),
            );
            index += 1;
        }
        Some(
            nibbles[..count * 2]
                .chunks(2)
                .map(|pair| (pair[0] << 4) | pair[1])
                .collect(),
        )
    }

    fn decode(&self, windows: &Windows, start: f64) -> Round {
        let Some(header) = self.bytes(windows, start, 3) else {
            return Round::Waiting;
        };
        let (magic, len, check) = (header[0], header[1], header[2]);
        let Some(kind) = Kind::of(magic).filter(|_| check == !len) else {
            return Round::Bad;
        };
        let len = usize::from(len);
        let Some(mut bytes) = self.bytes(windows, start, 3 + len) else {
            return Round::Waiting;
        };
        bytes.drain(..3);
        if kind == Kind::Frame {
            let Some(at) = len.checked_sub(4) else {
                return Round::Bad;
            };
            let crc = u32::from_le_bytes(bytes[at..].try_into().expect("4 bytes"));
            bytes.truncate(at);
            if crc32(&bytes) != crc {
                return Round::Bad;
            }
        }
        Round::Heard(kind, bytes)
    }
}

enum Round {
    Waiting,
    Bad,
    Heard(Kind, Vec<u8>),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic noise, without a dependency.
    struct Noise(u64);

    impl Noise {
        fn next(&mut self) -> f32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0
        }
    }

    fn payload() -> Vec<u8> {
        (0..143u8)
            .map(|byte| byte.wrapping_mul(37).wrapping_add(11))
            .collect()
    }

    /// Feeds `samples` in uneven pieces, as a microphone would.
    fn listen(listener: &mut Listener, samples: &[f32]) -> Option<(Kind, Vec<u8>)> {
        let mut heard = None;
        let mut at = 0;
        let mut size = 777;
        while at < samples.len() {
            let end = (at + size).min(samples.len());
            heard = heard.or(listener.push(&samples[at..end]));
            at = end;
            size = size * 7 % 4093 + 100;
        }
        heard
    }

    /// Played halfway through a round and then the next, through a room:
    /// echoes, then noise at about a third of the signal's loudness, heard
    /// at 44.1 kHz.
    fn through_a_room(round: &[f32]) -> Vec<f32> {
        let mut played: Vec<f32> = round[round.len() / 2..].to_vec();
        played.extend_from_slice(round);
        let mut room = played.clone();
        for (delay, gain) in [(0.02, 0.5), (0.06, 0.25)] {
            let delay = (delay * f64::from(SAMPLE_RATE)) as usize;
            for index in delay..played.len() {
                room[index] += played[index - delay] * gain;
            }
        }
        let mut noise = Noise(0x9E37_79B9_7F4A_7C15);
        for sample in &mut room {
            *sample = *sample * 0.5 + noise.next() * 0.1;
        }
        let ratio = 48_000.0 / 44_100.0;
        (0..(room.len() as f64 / ratio) as usize - 1)
            .map(|index| {
                let at = index as f64 * ratio;
                let (whole, part) = (at as usize, (at.fract()) as f32);
                room[whole] * (1.0 - part) + room[whole + 1] * part
            })
            .collect()
    }

    #[test]
    fn a_round_is_heard_back_in_either_band() {
        let payload = payload();
        for band in Band::ALL {
            let mut listener = Listener::new(SAMPLE_RATE);
            assert_eq!(
                listen(&mut listener, &samples(band, Kind::Card, &payload)),
                Some((Kind::Card, payload.clone())),
                "{band:?}"
            );
        }
    }

    #[test]
    fn a_round_is_heard_through_a_room_at_another_rate() {
        let payload = payload();
        for band in Band::ALL {
            let room = through_a_room(&samples(band, Kind::Card, &payload));
            let mut listener = Listener::new(44_100);
            assert_eq!(
                listen(&mut listener, &room),
                Some((Kind::Card, payload.clone())),
                "{band:?}"
            );
        }
    }

    #[test]
    fn the_audible_band_keeps_its_format() {
        // A marker, the header, and three bytes a symbol: 90 symbols for the
        // longest round, as before there were two bands.
        let symbols = symbols(Band::Audible, 0xCC, &[0x5A; MAX_LEN]);
        assert_eq!(symbols.len(), MARKER_SYMBOLS + 1 + MAX_LEN / 3);
        assert_eq!(
            symbols[MARKER_SYMBOLS],
            vec![12, 16 + 12, 32 + 15, 48 + 15, 64, 80]
        );
    }

    #[test]
    fn the_longest_round_fits_in_what_the_listener_keeps() {
        for band in Band::ALL {
            let round = samples(band, Kind::Card, &[0; MAX_LEN]);
            assert!(
                (round.len() as f64) < KEEP_SECS * f64::from(SAMPLE_RATE),
                "{band:?}"
            );
        }
    }

    /// The power of `samples` at `hz`, Hann-windowed.
    fn power(samples: &[f32], hz: f64) -> f64 {
        let coefficient = 2.0 * (TAU * hz / f64::from(SAMPLE_RATE)).cos();
        let (mut previous, mut before) = (0.0, 0.0);
        for (index, &sample) in samples.iter().enumerate() {
            let hann = 0.5 - 0.5 * (TAU * index as f64 / samples.len() as f64).cos();
            let next = f64::from(sample) * hann + coefficient * previous - before;
            before = previous;
            previous = next;
        }
        previous * previous + before * before - coefficient * previous * before
    }

    #[test]
    fn the_bands_above_hearing_stay_out_of_hearing() {
        // A step from one symbol to the next would click, spreading sound
        // down into what people hear. Every tone ends a symbol where the
        // next begins, so the round stays near its tones.
        for band in [Band::Inaudible, Band::FarInaudible] {
            let round = samples(band, Kind::Card, &payload());
            let tones = (0..band.tones())
                .map(|tone| power(&round, band.frequency(tone)))
                .fold(0.0, f64::max);
            let heard = (1..=30)
                .map(|step| power(&round, f64::from(step) * 500.0))
                .fold(0.0, f64::max);
            assert!(heard < tones * 1e-6, "{band:?}: {heard} against {tones}");
        }
    }

    #[test]
    fn a_slow_listener_hears_only_the_audible_bands() {
        let payload = payload();
        let round = samples(Band::Audible, Kind::Card, &payload);
        let slow: Vec<f32> = round.iter().step_by(3).copied().collect();
        let mut listener = Listener::new(16_000);
        assert!(listener
            .searches
            .iter()
            .all(|search| search.band != Band::Inaudible));
        assert_eq!(listen(&mut listener, &slow), Some((Kind::Card, payload)));
    }

    #[test]
    fn the_far_band_moves_between_two_places_one_tone_at_a_time() {
        let symbols = symbols(Band::Far, 0xCC, &[0x5A]);
        // The marker, two tones each, then 0xCC, 1, !1 and 0x5A a nibble at a time.
        assert_eq!(
            symbols[..MARKER_SYMBOLS],
            [vec![0, 8], vec![20, 28], vec![0, 8], vec![20, 28]]
        );
        assert_eq!(
            symbols[MARKER_SYMBOLS..],
            [12, 16 + 12, 0, 16 + 1, 15, 16 + 14, 5, 16 + 10].map(|tone| vec![tone])
        );
        assert!((Band::Far.frequency(Band::Far.tones() - 1) - 2953.125).abs() < 1e-9);
    }

    /// `round` played `gain` as loud, from halfway through it and then once
    /// more, over noise at a tenth of full scale, as a microphone far away
    /// would hear it.
    fn faint(round: &[f32], gain: f32, seed: u64) -> Vec<f32> {
        let mut noise = Noise(seed);
        round[round.len() / 2..]
            .iter()
            .chain(round)
            .map(|sample| sample * gain + noise.next() * 0.1)
            .collect()
    }

    #[test]
    fn the_far_band_is_heard_where_the_others_are_not() {
        let payload = payload();
        // About 18 dB down from where every band is heard.
        let gain = 0.025;
        let heard = |band: Band| {
            let mut listener = Listener::new(SAMPLE_RATE);
            listen(
                &mut listener,
                &faint(&samples(band, Kind::Card, &payload), gain, 7),
            )
        };
        assert_eq!(heard(Band::Far), Some((Kind::Card, payload.clone())));
        assert_eq!(
            heard(Band::FarInaudible),
            Some((Kind::Card, payload.clone()))
        );
        assert_eq!(heard(Band::Audible), None);
        assert_eq!(heard(Band::Inaudible), None);
    }

    #[test]
    fn noise_is_not_heard_as_a_round() {
        let mut noise = Noise(42);
        let samples: Vec<f32> = (0..SAMPLE_RATE * 5).map(|_| noise.next() * 0.5).collect();
        let mut listener = Listener::new(SAMPLE_RATE);
        assert_eq!(listen(&mut listener, &samples), None);
    }

    #[test]
    fn a_wav_holds_the_round() {
        let wav = wav(Band::Audible, Kind::Card, &[&[1, 2, 3], &[4]]);
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        let rounds = samples(Band::Audible, Kind::Card, &[1, 2, 3]).len()
            + samples(Band::Audible, Kind::Card, &[4]).len();
        assert_eq!(wav.len(), 44 + rounds * 2);
    }

    #[test]
    fn a_frame_is_heard_back_after_its_check() {
        let frame = payload();
        for band in Band::ALL {
            let mut listener = Listener::new(SAMPLE_RATE);
            assert_eq!(
                listen(&mut listener, &samples(band, Kind::Frame, &frame)),
                Some((Kind::Frame, frame.clone()))
            );
            // A frame whose check does not match was heard wrong, and is
            // dropped.
            let mut wrong = frame.clone();
            wrong.extend_from_slice(&crc32(&frame).wrapping_add(1).to_le_bytes());
            let mut listener = Listener::new(SAMPLE_RATE);
            assert_eq!(
                listen(&mut listener, &round(band, Kind::Frame.magic(), &wrong)),
                None
            );
        }
    }

    #[test]
    fn the_check_is_crc_32() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }
}
