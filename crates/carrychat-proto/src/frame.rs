//! What transports carry: whole bundles, fragments, tombstones and file
//! chunks (§7).

use std::collections::HashMap;

use crate::bundle::{Bundle, BundleId, Tombstone};
use crate::{truncate, Error, Result};

/// Largest payload of one Meshtastic packet (`DATA_PAYLOAD_LEN`).
pub const MESHTASTIC_MTU: usize = 233;
pub const FRAGMENT_HEADER_LEN: usize = 11;
pub const MAX_FRAGMENTS: usize = 32;
pub const TOMBSTONE_FRAME_LEN: usize = 26;
/// How long an incomplete bundle waits for its other fragments: long enough
/// for the largest bundle over LoRa at Meshtastic's slowest preset, sent
/// within the airtime budget (§9.2), which takes over five hours.
pub const DEFAULT_REASSEMBLY_TIMEOUT_SECS: u32 = 6 * 60 * 60;

const TYPE_LEN: usize = 1;
const TYPE_BUNDLE: u8 = 0x01;
const TYPE_FRAGMENT: u8 = 0x02;
const TYPE_TOMBSTONE: u8 = 0x03;
const TYPE_CHUNK: u8 = 0x04;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Frame {
    Bundle(Bundle),
    Fragment(Fragment),
    Tombstone {
        hop_limit: u8,
        tombstone: Tombstone,
    },
    /// A file chunk (§5.9): a bundle that never goes over LoRa.
    Chunk(Bundle),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fragment {
    pub id: BundleId,
    pub index: u8,
    pub count: u8,
    pub chunk: Vec<u8>,
}

impl Frame {
    pub fn to_bytes(&self) -> Vec<u8> {
        match self {
            Frame::Bundle(bundle) => [&[TYPE_BUNDLE][..], &bundle.to_bytes()].concat(),
            Frame::Chunk(bundle) => [&[TYPE_CHUNK][..], &bundle.to_bytes()].concat(),
            Frame::Fragment(fragment) => [
                &[TYPE_FRAGMENT][..],
                &fragment.id.0,
                &[fragment.index, fragment.count],
                &fragment.chunk,
            ]
            .concat(),
            Frame::Tombstone {
                hop_limit,
                tombstone,
            } => [
                &[TYPE_TOMBSTONE, *hop_limit][..],
                &tombstone.id.0,
                &tombstone.ack_secret,
            ]
            .concat(),
        }
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let (&frame_type, rest) = bytes.split_first().ok_or(Error::Truncated)?;
        match frame_type {
            TYPE_BUNDLE => Ok(Frame::Bundle(Bundle::from_bytes(rest)?)),
            TYPE_CHUNK => Ok(Frame::Chunk(Bundle::from_bytes(rest)?)),
            TYPE_FRAGMENT => {
                if bytes.len() <= FRAGMENT_HEADER_LEN {
                    return Err(Error::Truncated);
                }
                let (index, count) = (bytes[9], bytes[10]);
                if !(2..=MAX_FRAGMENTS).contains(&(count as usize)) || index >= count {
                    return Err(Error::Invalid("fragment index"));
                }
                Ok(Frame::Fragment(Fragment {
                    id: BundleId(truncate(&bytes[1..9])),
                    index,
                    count,
                    chunk: bytes[FRAGMENT_HEADER_LEN..].to_vec(),
                }))
            }
            TYPE_TOMBSTONE => {
                match bytes.len().cmp(&TOMBSTONE_FRAME_LEN) {
                    core::cmp::Ordering::Less => return Err(Error::Truncated),
                    core::cmp::Ordering::Greater => return Err(Error::TooLong),
                    core::cmp::Ordering::Equal => {}
                }
                Ok(Frame::Tombstone {
                    hop_limit: bytes[1],
                    tombstone: Tombstone {
                        id: BundleId(truncate(&bytes[2..10])),
                        ack_secret: truncate(&bytes[10..26]),
                    },
                })
            }
            _ => Err(Error::Invalid("frame type")),
        }
    }
}

/// The frames that carry `bundle` over a link whose packets hold `mtu` bytes.
pub fn frames_for(bundle: &Bundle, mtu: usize) -> Result<Vec<Frame>> {
    let bytes = bundle.to_bytes();
    if TYPE_LEN + bytes.len() <= mtu {
        return Ok(vec![Frame::Bundle(bundle.clone())]);
    }
    let chunk_len = mtu
        .checked_sub(FRAGMENT_HEADER_LEN)
        .filter(|&len| len > 0)
        .ok_or(Error::Invalid("mtu"))?;
    let count = bytes.len().div_ceil(chunk_len);
    if count > MAX_FRAGMENTS {
        return Err(Error::TooLong);
    }
    Ok(bytes
        .chunks(chunk_len)
        .enumerate()
        .map(|(index, chunk)| {
            Frame::Fragment(Fragment {
                id: bundle.id,
                index: index as u8,
                count: count as u8,
                chunk: chunk.to_vec(),
            })
        })
        .collect())
}

/// Puts fragmented bundles back together, with a timeout and a cap on how
/// many incomplete bundles it holds.
pub struct Reassembler {
    pending: HashMap<BundleId, Pending>,
    timeout_secs: u32,
    max_pending: usize,
}

struct Pending {
    chunks: Vec<Option<Vec<u8>>>,
    first_seen: u32,
}

impl Reassembler {
    pub fn new(timeout_secs: u32, max_pending: usize) -> Self {
        Self {
            pending: HashMap::new(),
            timeout_secs,
            max_pending,
        }
    }

    /// Adds a fragment. Returns the bundle once its last fragment arrives.
    pub fn push(&mut self, fragment: Fragment, now: u32) -> Result<Option<Bundle>> {
        self.pending
            .retain(|_, pending| now.saturating_sub(pending.first_seen) < self.timeout_secs);
        if !self.pending.contains_key(&fragment.id) && self.pending.len() >= self.max_pending {
            let oldest = self
                .pending
                .iter()
                .min_by_key(|(_, pending)| pending.first_seen)
                .map(|(id, _)| *id);
            if let Some(oldest) = oldest {
                self.pending.remove(&oldest);
            }
        }

        let pending = self.pending.entry(fragment.id).or_insert_with(|| Pending {
            chunks: vec![None; fragment.count as usize],
            first_seen: now,
        });
        if pending.chunks.len() != fragment.count as usize {
            self.pending.remove(&fragment.id);
            return Err(Error::Invalid("fragment count"));
        }
        pending.chunks[fragment.index as usize] = Some(fragment.chunk);
        if pending.chunks.iter().any(Option::is_none) {
            return Ok(None);
        }

        let pending = self
            .pending
            .remove(&fragment.id)
            .expect("entry was just used");
        let bytes: Vec<u8> = pending.chunks.into_iter().flatten().flatten().collect();
        let bundle = Bundle::from_bytes(&bytes)?;
        if bundle.id != fragment.id {
            return Err(Error::Invalid("fragment id"));
        }
        Ok(Some(bundle))
    }
}

impl Default for Reassembler {
    fn default() -> Self {
        Self::new(DEFAULT_REASSEMBLY_TIMEOUT_SECS, 64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundle::{seal, Message, DEFAULT_TTL_SECS};
    use crate::identity::Identity;
    use crate::pair::PairKeys;

    const NOW: u32 = 1_790_000_000;

    fn bundle_with_body(len: usize) -> Bundle {
        let alice = Identity::generate("alice").unwrap();
        let bob = Identity::generate("bob").unwrap();
        let keys = PairKeys::derive(&alice, &bob.dh_public()).unwrap();
        seal(
            &keys,
            &bob.dh_public(),
            &Message::text(&"ş".repeat(len / 2), NOW),
            NOW + DEFAULT_TTL_SECS,
            8,
        )
        .unwrap()
    }

    #[test]
    fn every_frame_type_round_trips() {
        let bundle = bundle_with_body(40);
        let frames = [
            Frame::Bundle(bundle.clone()),
            Frame::Fragment(Fragment {
                id: bundle.id,
                index: 1,
                count: 3,
                chunk: vec![1, 2, 3],
            }),
            Frame::Tombstone {
                hop_limit: 4,
                tombstone: Tombstone {
                    id: bundle.id,
                    ack_secret: [7; 16],
                },
            },
        ];
        for frame in frames {
            assert_eq!(Frame::from_bytes(&frame.to_bytes()).unwrap(), frame);
        }
    }

    #[test]
    fn small_bundles_are_not_fragmented() {
        let bundle = bundle_with_body(100);
        assert_eq!(
            frames_for(&bundle, MESHTASTIC_MTU).unwrap(),
            vec![Frame::Bundle(bundle)]
        );
    }

    #[test]
    fn fragments_reassemble_in_any_order() {
        let bundle = bundle_with_body(1000);
        let mut frames = frames_for(&bundle, MESHTASTIC_MTU).unwrap();
        assert!(frames.len() > 1);
        assert!(frames
            .iter()
            .all(|frame| frame.to_bytes().len() <= MESHTASTIC_MTU));
        frames.reverse();
        frames.push(frames[0].clone());

        let mut reassembler = Reassembler::default();
        let mut done = None;
        for frame in frames {
            let Frame::Fragment(fragment) = frame else {
                panic!("expected fragments");
            };
            if let Some(bundle) = reassembler.push(fragment, NOW).unwrap() {
                done = Some(bundle);
            }
        }
        assert_eq!(done, Some(bundle));
    }

    #[test]
    fn fragments_paced_by_lora_airtime_still_come_together() {
        // The largest bundle at Meshtastic's slowest preset takes over five
        // hours to send within the airtime budget.
        let bundle = bundle_with_body(1000);
        let frames = frames_for(&bundle, MESHTASTIC_MTU).unwrap();
        let mut reassembler = Reassembler::default();
        let count = frames.len() as u32;
        let mut done = None;
        for (index, frame) in frames.into_iter().enumerate() {
            let Frame::Fragment(fragment) = frame else {
                panic!("expected fragments")
            };
            let at = NOW + index as u32 * (5 * 60 * 60 / count);
            if let Some(bundle) = reassembler.push(fragment, at).unwrap() {
                done = Some(bundle);
            }
        }
        assert_eq!(done, Some(bundle));
    }

    #[test]
    fn incomplete_bundles_time_out() {
        let bundle = bundle_with_body(1000);
        let frames = frames_for(&bundle, MESHTASTIC_MTU).unwrap();
        let mut reassembler = Reassembler::new(60, 8);
        let mut fragments = frames.into_iter().map(|frame| match frame {
            Frame::Fragment(fragment) => fragment,
            _ => panic!("expected fragments"),
        });
        let first = fragments.next().unwrap();
        assert_eq!(reassembler.push(first, NOW).unwrap(), None);
        for fragment in fragments {
            assert_eq!(reassembler.push(fragment, NOW + 61).unwrap(), None);
        }
    }

    #[test]
    fn invalid_fragment_headers_are_rejected() {
        let mut bytes = Frame::Fragment(Fragment {
            id: BundleId([0; 8]),
            index: 0,
            count: 2,
            chunk: vec![0],
        })
        .to_bytes();
        bytes[9] = 2;
        assert_eq!(
            Frame::from_bytes(&bytes),
            Err(Error::Invalid("fragment index"))
        );
    }
}
