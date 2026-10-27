//! Cards over sound (§3.2): a few hundred bytes, such as a contact card,
//! played through a speaker and heard through a microphone, so two devices
//! that are side by side exchange cards with nothing else in common.
//!
//! Six channels of 16 tones each, 46.875 Hz apart from 1875 Hz up to about
//! 6.3 kHz, where phone speakers and microphones work well. Each 100 ms
//! symbol sounds one tone per channel, a nibble each, so three bytes. A start
//! marker of four symbols comes first, then a header with the length, then
//! the bytes. The sender plays it over and over; the listener needs one clean
//! round. Nothing checks the bytes here: a card verifies its own signature.

use std::collections::VecDeque;
use std::f64::consts::TAU;

/// The rate `samples` and `wav` play at. `Listener` takes any rate.
pub const SAMPLE_RATE: u32 = 48_000;
/// The most bytes one round carries.
pub const MAX_LEN: usize = 255;

const BASE_HZ: f64 = 1875.0;
/// Tones this far apart are orthogonal over a window of 1/STEP_HZ seconds.
const STEP_HZ: f64 = 46.875;
const CHANNELS: usize = 6;
const TONES: usize = 16;
const SYMBOL_SECS: f64 = 0.1;
/// Each symbol fades in and out over this long, so it does not click.
const RAMP_SECS: f64 = 0.005;
/// What the listener skips at the start of each symbol, where the room
/// still echoes the last one.
const GUARD_SECS: f64 = 0.03;
/// Silence before and after each round.
const LEAD_SECS: f64 = 0.1;
const TAIL_SECS: f64 = 0.4;
/// The start marker alternates two symbols, each with two tones in every
/// channel, which no data symbol has.
const MARKER: [[usize; 2]; 2] = [[0, 8], [4, 12]];
const MARKER_SYMBOLS: usize = 4;
/// Share of a channel's energy in the marker's tones: 1 for a clean marker,
/// about 1/8 for noise and well under this for data.
const MARKER_THRESHOLD: f32 = 0.5;
/// The header's first byte; the other two are the length and its complement.
const MAGIC: u8 = 0xCC;
/// How much sound the listener keeps: longer than the longest round.
const KEEP_SECS: f64 = 12.0;

/// One round of `payload` as samples at `SAMPLE_RATE`, between -1 and 1.
pub fn samples(payload: &[u8]) -> Vec<f32> {
    assert!(payload.len() <= MAX_LEN, "too long for one round");
    let rate = f64::from(SAMPLE_RATE);
    let mut out = vec![0.0; (LEAD_SECS * rate) as usize];
    for symbol in symbols(payload) {
        sound(&mut out, &symbol, rate);
    }
    out.resize(out.len() + (TAIL_SECS * rate) as usize, 0.0);
    out
}

/// One round of `payload` as a 16-bit mono WAV file.
pub fn wav(payload: &[u8]) -> Vec<u8> {
    let samples = samples(payload);
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

/// The tones of each symbol: the marker, the header, then the bytes, three
/// to a symbol and zero-padded.
fn symbols(payload: &[u8]) -> Vec<Vec<usize>> {
    let len = payload.len() as u8;
    let mut symbols: Vec<Vec<usize>> = (0..MARKER_SYMBOLS)
        .map(|index| {
            (0..CHANNELS)
                .flat_map(|channel| MARKER[index % 2].map(|tone| channel * TONES + tone))
                .collect()
        })
        .collect();
    let header = [MAGIC, len, !len];
    for bytes in std::iter::once(&header[..]).chain(payload.chunks(3)) {
        let mut padded = [0; 3];
        padded[..bytes.len()].copy_from_slice(bytes);
        let nibbles = padded.iter().flat_map(|byte| [byte >> 4, byte & 0x0F]);
        symbols.push(
            nibbles
                .enumerate()
                .map(|(channel, nibble)| channel * TONES + usize::from(nibble))
                .collect(),
        );
    }
    symbols
}

fn frequency(tone: usize) -> f64 {
    BASE_HZ + STEP_HZ * tone as f64
}

/// Appends one symbol: its tones at equal loudness, faded in and out.
fn sound(out: &mut Vec<f32>, tones: &[usize], rate: f64) {
    let len = (SYMBOL_SECS * rate) as usize;
    let ramp = RAMP_SECS * rate;
    let loudness = 0.9 / tones.len() as f64;
    out.extend((0..len).map(|index| {
        let at = index as f64;
        let fade = (at / ramp).min((len as f64 - at) / ramp).min(1.0);
        let fade = 0.5 - 0.5 * (fade * std::f64::consts::PI).cos();
        let sum: f64 = tones
            .iter()
            .map(|&tone| (TAU * frequency(tone) * at / rate).sin())
            .sum();
        (sum * loudness * fade) as f32
    }));
}

/// Listens for rounds in a microphone's samples.
pub struct Listener {
    rate: f64,
    /// Samples per analysis window, and between windows.
    window: usize,
    hop: usize,
    /// `2·cos(ω)` of each tone at this rate, for Goertzel's algorithm.
    coefficients: Vec<f64>,
    /// Samples not yet analysed; the first is at `pending_at`.
    pending: VecDeque<f32>,
    /// Each window's energy in every tone. `frames[0]` is window `first`.
    frames: VecDeque<[f32; CHANNELS * TONES]>,
    first: usize,
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
        let window = (rate / STEP_HZ).round() as usize;
        let coefficients = (0..CHANNELS * TONES)
            .map(|tone| 2.0 * (TAU * frequency(tone) / rate).cos())
            .collect();
        Self {
            rate,
            window,
            hop: window / 4,
            coefficients,
            pending: VecDeque::new(),
            frames: VecDeque::new(),
            first: 0,
            searched: 0,
            run: None,
            rounds: Vec::new(),
        }
    }

    /// Takes in samples. Returns the bytes of a round once one has been
    /// heard whole.
    pub fn push(&mut self, samples: &[f32]) -> Option<Vec<u8>> {
        self.pending.extend(samples);
        while self.pending.len() >= self.window {
            let frame = self.analyse();
            self.frames.push_back(frame);
            self.pending.drain(..self.hop);
        }
        let keep = (KEEP_SECS * self.rate) as usize / self.hop;
        while self.frames.len() > keep {
            self.frames.pop_front();
            self.first += 1;
        }
        self.find_markers();
        self.decode_rounds()
    }

    /// The energy in every tone over the next window.
    fn analyse(&self) -> [f32; CHANNELS * TONES] {
        let mut energies = [0.0; CHANNELS * TONES];
        for (energy, coefficient) in energies.iter_mut().zip(&self.coefficients) {
            let (mut previous, mut before) = (0.0, 0.0);
            for &sample in self.pending.iter().take(self.window) {
                let next = f64::from(sample) + coefficient * previous - before;
                before = previous;
                previous = next;
            }
            *energy =
                (previous * previous + before * before - coefficient * previous * before) as f32;
        }
        energies
    }

    /// The samples in a symbol, and in its guard.
    fn symbol_len(&self) -> (f64, f64) {
        (SYMBOL_SECS * self.rate, GUARD_SECS * self.rate)
    }

    /// The mean energy of each tone over symbol `index` of a round starting
    /// at sample `start`, or None until it has been heard whole.
    fn symbol(&self, start: f64, index: usize) -> Option<[f32; CHANNELS * TONES]> {
        let (len, guard) = self.symbol_len();
        let from = start + len * index as f64 + guard;
        let to = start + len * (index + 1) as f64 - self.window as f64;
        let first = (from / self.hop as f64).ceil().max(0.0) as usize;
        let last = (to / self.hop as f64).floor() as usize;
        if last < first || last >= self.first + self.frames.len() || first < self.first {
            return None;
        }
        let mut sum = [0.0; CHANNELS * TONES];
        for frame in &self
            .frames
            .range(first - self.first..=last - self.first)
            .collect::<Vec<_>>()
        {
            for (total, energy) in sum.iter_mut().zip(frame.iter()) {
                *total += energy;
            }
        }
        Some(sum)
    }

    /// How much like marker symbol `index` a symbol sounds, from 0 to 1.
    fn marker_score(energies: &[f32; CHANNELS * TONES], index: usize) -> f32 {
        let tones = MARKER[index % 2];
        let mut score = 0.0;
        for channel in energies.chunks(TONES) {
            let total: f32 = channel.iter().sum();
            if total > 0.0 {
                score += (channel[tones[0]] + channel[tones[1]]) / total;
            }
        }
        score / CHANNELS as f32
    }

    /// Looks for the start marker at every window not yet searched. Each run
    /// of windows where it could start ends in one round, timed from the
    /// middle of the run.
    fn find_markers(&mut self) {
        let end = self.first + self.frames.len();
        self.searched = self.searched.max(self.first);
        while self.searched < end {
            let start = (self.searched * self.hop) as f64;
            let mut score = f32::MAX;
            for index in 0..MARKER_SYMBOLS {
                match self.symbol(start, index) {
                    Some(energies) => score = score.min(Self::marker_score(&energies, index)),
                    None => return,
                }
            }
            if score >= MARKER_THRESHOLD {
                self.run.get_or_insert(self.searched);
            } else if let Some(run) = self.run.take() {
                let middle = (run + self.searched - 1) as f64 / 2.0;
                self.rounds.push(middle * self.hop as f64);
            }
            self.searched += 1;
        }
    }

    /// Reads each round found once it has been heard whole. A round whose
    /// header does not read right is dropped.
    fn decode_rounds(&mut self) -> Option<Vec<u8>> {
        let mut index = 0;
        while index < self.rounds.len() {
            match self.decode(self.rounds[index]) {
                Round::Waiting => index += 1,
                Round::Bad => {
                    self.rounds.remove(index);
                }
                Round::Heard(bytes) => {
                    self.rounds.remove(index);
                    return Some(bytes);
                }
            }
        }
        None
    }

    fn decode(&self, start: f64) -> Round {
        let read = |index: usize| -> Option<[u8; 3]> {
            let energies = self.symbol(start, MARKER_SYMBOLS + index)?;
            let mut nibbles = energies.chunks(TONES).map(|channel| {
                let (loudest, _) =
                    channel
                        .iter()
                        .enumerate()
                        .fold((0, f32::MIN), |best, (tone, &energy)| {
                            if energy > best.1 {
                                (tone, energy)
                            } else {
                                best
                            }
                        });
                loudest as u8
            });
            let mut bytes = [0; 3];
            for byte in &mut bytes {
                *byte = (nibbles.next().unwrap_or(0) << 4) | nibbles.next().unwrap_or(0);
            }
            Some(bytes)
        };
        let Some([magic, len, check]) = read(0) else {
            return Round::Waiting;
        };
        if magic != MAGIC || check != !len {
            return Round::Bad;
        }
        let len = usize::from(len);
        let mut bytes = Vec::with_capacity(len + 2);
        for index in 0..len.div_ceil(3) {
            match read(1 + index) {
                Some(symbol) => bytes.extend_from_slice(&symbol),
                None => return Round::Waiting,
            }
        }
        bytes.truncate(len);
        Round::Heard(bytes)
    }
}

enum Round {
    Waiting,
    Bad,
    Heard(Vec<u8>),
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
    fn listen(listener: &mut Listener, samples: &[f32]) -> Option<Vec<u8>> {
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

    #[test]
    fn a_round_is_heard_back() {
        let payload = payload();
        let mut listener = Listener::new(SAMPLE_RATE);
        assert_eq!(listen(&mut listener, &samples(&payload)), Some(payload));
    }

    #[test]
    fn a_round_is_heard_through_a_room_at_another_rate() {
        let payload = payload();
        let round = samples(&payload);
        // The listener comes in halfway through a round, and hears the next.
        let mut played: Vec<f32> = round[round.len() / 2..].to_vec();
        played.extend_from_slice(&round);
        // Echoes, then noise at about a third of the signal's loudness.
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
        // Resampled to 44.1 kHz.
        let ratio = 48_000.0 / 44_100.0;
        let resampled: Vec<f32> = (0..(room.len() as f64 / ratio) as usize - 1)
            .map(|index| {
                let at = index as f64 * ratio;
                let (whole, part) = (at as usize, (at.fract()) as f32);
                room[whole] * (1.0 - part) + room[whole + 1] * part
            })
            .collect();
        let mut listener = Listener::new(44_100);
        assert_eq!(listen(&mut listener, &resampled), Some(payload));
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
        let wav = wav(&[1, 2, 3]);
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        assert_eq!(wav.len(), 44 + samples(&[1, 2, 3]).len() * 2);
    }
}
