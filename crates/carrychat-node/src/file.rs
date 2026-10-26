//! Moving bundles by hand: a file of frames for a USB stick (§9.4).

use carrychat_proto::frame::Frame;

use crate::{Error, Result};

pub const MAGIC: &[u8; 5] = b"CRCH\0";
/// Files larger than this are refused on import.
pub const MAX_FILE_LEN: u64 = 64 * 1024 * 1024;
pub const EXTENSION: &str = "carrychat";

pub fn encode(frames: &[Frame]) -> Vec<u8> {
    let mut bytes = MAGIC.to_vec();
    for frame in frames {
        let frame = frame.to_bytes();
        let len = u16::try_from(frame.len()).expect("frames are smaller than 64 KiB");
        bytes.extend_from_slice(&len.to_be_bytes());
        bytes.extend_from_slice(&frame);
    }
    bytes
}

/// Reads the frames in a file. Frames that do not parse are skipped and
/// counted, so one damaged frame does not lose the rest.
pub fn decode(bytes: &[u8]) -> Result<(Vec<Frame>, usize)> {
    let mut rest = bytes
        .strip_prefix(MAGIC.as_slice())
        .ok_or(Error::Invalid("not a CarryChat file"))?;
    let mut frames = Vec::new();
    let mut skipped = 0;
    while !rest.is_empty() {
        let (len, tail) = rest
            .split_at_checked(2)
            .ok_or(Error::Invalid("file is truncated"))?;
        let len = u16::from_be_bytes([len[0], len[1]]) as usize;
        let (frame, tail) = tail
            .split_at_checked(len)
            .ok_or(Error::Invalid("file is truncated"))?;
        match Frame::from_bytes(frame) {
            Ok(frame) => frames.push(frame),
            Err(_) => skipped += 1,
        }
        rest = tail;
    }
    Ok((frames, skipped))
}
