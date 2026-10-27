//! MeshCore's companion radio protocol, as far as the LoRa link uses it:
//! the frames between an app and the device, what the device says about
//! itself, its channels and the devices it heard, and channel datagrams.
//! See `docs/companion_protocol.md` in MeshCore's repository.

use sha2::{Digest, Sha256};

/// A frame to the device over a serial port or TCP: `<`, the body's length
/// (little-endian `u16`), the body. Over Bluetooth LE each write is one body.
pub const TO_DEVICE: u8 = b'<';
/// A frame from the device: `>`, the length, the body. Over Bluetooth LE
/// each notification is one body.
pub const FROM_DEVICE: u8 = b'>';
/// The longest body either side sends.
pub const MAX_FRAME_LEN: usize = 300;
/// MeshCore devices on the network listen on this port.
pub const TCP_PORT: u16 = 5000;

pub const CMD_APP_START: u8 = 1;
pub const CMD_GET_CONTACTS: u8 = 4;
pub const CMD_SET_DEVICE_TIME: u8 = 6;
pub const CMD_SYNC_NEXT_MESSAGE: u8 = 10;
pub const CMD_DEVICE_QUERY: u8 = 22;
pub const CMD_GET_CHANNEL: u8 = 31;
pub const CMD_SET_CHANNEL: u8 = 32;
pub const CMD_SEND_CHANNEL_DATA: u8 = 62;

pub const RESP_OK: u8 = 0;
pub const RESP_ERR: u8 = 1;
pub const RESP_CONTACTS_START: u8 = 2;
pub const RESP_CONTACT: u8 = 3;
pub const RESP_END_OF_CONTACTS: u8 = 4;
pub const RESP_SELF_INFO: u8 = 5;
pub const RESP_NO_MORE_MESSAGES: u8 = 10;
pub const RESP_DEVICE_INFO: u8 = 13;
pub const RESP_CHANNEL_INFO: u8 = 18;
pub const RESP_CHANNEL_DATA: u8 = 27;

/// Codes from here up are pushed by the device at any time, not replies.
pub const PUSH_FIRST: u8 = 0x80;
/// A device's advert was heard; its public key follows.
pub const PUSH_ADVERT: u8 = 0x80;
/// Messages wait on the device, to fetch with `CMD_SYNC_NEXT_MESSAGE`.
pub const PUSH_MSG_WAITING: u8 = 0x83;
/// A new device's advert was heard; its contact follows.
pub const PUSH_NEW_ADVERT: u8 = 0x8A;

/// The send queue is full: try again later.
pub const ERR_TABLE_FULL: u8 = 3;

/// The protocol version CarryChat speaks, sent with `CMD_DEVICE_QUERY`.
pub const PROTOCOL_VERSION: u8 = 3;
/// Channel slots a device has when it does not say.
pub const DEFAULT_CHANNELS: u8 = 8;
/// The channel CarryChat's datagrams go on: a hashtag channel, whose secret
/// anyone can derive from its name. Frames are sealed end to end already.
pub const CHANNEL_NAME: &str = "#carrychat";
/// CarryChat's datagram type, from the range MeshCore leaves for testing
/// until one is registered.
pub const DATA_TYPE: u16 = 0xFFCC;
/// `path_len` that floods a datagram through every repeater.
pub const FLOOD: u8 = 0xFF;
/// The most a channel datagram carries.
pub const MTU: usize = 165;
/// What a channel datagram adds on the air: the header, the path length,
/// the channel hash, the MAC, the data type and length, and up to 15 bytes
/// of padding.
pub const OVERHEAD: usize = 24;

/// The secret of `CHANNEL_NAME`: the first 16 bytes of its SHA-256.
pub fn channel_secret() -> [u8; 16] {
    let hash = Sha256::digest(CHANNEL_NAME.as_bytes());
    let mut secret = [0; 16];
    secret.copy_from_slice(&hash[..16]);
    secret
}

/// `body` framed for the device.
pub fn frame(body: &[u8]) -> Vec<u8> {
    framed(TO_DEVICE, body)
}

/// `body` framed as the device frames it.
pub fn frame_from_device(body: &[u8]) -> Vec<u8> {
    framed(FROM_DEVICE, body)
}

fn framed(marker: u8, body: &[u8]) -> Vec<u8> {
    let len = body.len() as u16;
    let mut bytes = Vec::with_capacity(3 + body.len());
    bytes.push(marker);
    bytes.extend_from_slice(&len.to_le_bytes());
    bytes.extend_from_slice(body);
    bytes
}

/// Takes the frames out of a byte stream, skipping whatever comes between
/// them, such as a device's log.
pub struct Deframer {
    marker: u8,
    buffer: Vec<u8>,
}

impl Deframer {
    /// Reads frames that start with `marker`: `FROM_DEVICE` for what the
    /// device sends, `TO_DEVICE` for what is sent to it.
    pub fn new(marker: u8) -> Self {
        Self {
            marker,
            buffer: Vec::new(),
        }
    }

    pub fn push(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
    }

    pub fn next_frame(&mut self) -> Option<Vec<u8>> {
        loop {
            let start = self.buffer.iter().position(|&byte| byte == self.marker);
            let Some(start) = start else {
                self.buffer.clear();
                return None;
            };
            self.buffer.drain(..start);
            if self.buffer.len() < 3 {
                return None;
            }
            let len = usize::from(u16::from_le_bytes([self.buffer[1], self.buffer[2]]));
            if len == 0 || len > MAX_FRAME_LEN {
                self.buffer.remove(0);
                continue;
            }
            if self.buffer.len() < 3 + len {
                return None;
            }
            let body = self.buffer[3..3 + len].to_vec();
            self.buffer.drain(..3 + len);
            return Some(body);
        }
    }
}

/// `CMD_APP_START`: seven reserved bytes, then the app's name.
pub fn app_start() -> Vec<u8> {
    let mut body = vec![CMD_APP_START, 0, 0, 0, 0, 0, 0, 0];
    body.extend_from_slice(b"CarryChat");
    body
}

/// `CMD_SET_CHANNEL`: the slot, the name padded to 32 bytes, the secret.
pub fn set_channel(index: u8, name: &str, secret: &[u8; 16]) -> Vec<u8> {
    let mut body = vec![CMD_SET_CHANNEL, index];
    let mut padded = [0; 32];
    let name = name.as_bytes();
    padded[..name.len().min(31)].copy_from_slice(&name[..name.len().min(31)]);
    body.extend_from_slice(&padded);
    body.extend_from_slice(secret);
    body
}

/// `CMD_SEND_CHANNEL_DATA`, flooded.
pub fn send_channel_data(channel: u8, data: &[u8]) -> Vec<u8> {
    let mut body = vec![CMD_SEND_CHANNEL_DATA, channel, FLOOD];
    body.extend_from_slice(&DATA_TYPE.to_le_bytes());
    body.extend_from_slice(data);
    body
}

/// What `RESP_SELF_INFO` says about the device.
#[derive(Clone, Debug, PartialEq)]
pub struct SelfInfo {
    pub public_key: [u8; 32],
    pub name: String,
    pub frequency_khz: u32,
    pub bandwidth_hz: u32,
    pub spread_factor: u8,
    /// 5 to 8 for 4/5 to 4/8.
    pub coding_rate: u8,
}

impl SelfInfo {
    pub fn parse(body: &[u8]) -> Option<Self> {
        if body.len() < 58 || body[0] != RESP_SELF_INFO {
            return None;
        }
        Some(Self {
            public_key: body[4..36].try_into().ok()?,
            frequency_khz: u32_at(body, 48)?,
            bandwidth_hz: u32_at(body, 52)?,
            spread_factor: body[56],
            coding_rate: body[57],
            name: text(&body[58..]),
        })
    }

    /// As MeshCore's apps write it, such as `869.525 MHz, SF11, 250 kHz`.
    pub fn radio(&self) -> String {
        format!(
            "{} MHz, SF{}, {} kHz",
            f64::from(self.frequency_khz) / 1000.0,
            self.spread_factor,
            f64::from(self.bandwidth_hz) / 1000.0
        )
    }
}

/// The channel slots a device has, from `RESP_DEVICE_INFO`.
pub fn channel_count(body: &[u8]) -> u8 {
    match body {
        [RESP_DEVICE_INFO, _, _, count, ..] if *count > 0 => *count,
        _ => DEFAULT_CHANNELS,
    }
}

/// A channel slot, from `RESP_CHANNEL_INFO`.
#[derive(Clone, Debug, PartialEq)]
pub struct ChannelInfo {
    pub index: u8,
    pub name: String,
    pub secret: [u8; 16],
}

impl ChannelInfo {
    pub fn parse(body: &[u8]) -> Option<Self> {
        if body.len() < 50 || body[0] != RESP_CHANNEL_INFO {
            return None;
        }
        Some(Self {
            index: body[1],
            name: text(&body[2..34]),
            secret: body[34..50].try_into().ok()?,
        })
    }

    /// An unused slot has no name and an all-zero secret.
    pub fn is_free(&self) -> bool {
        self.name.is_empty() && self.secret == [0; 16]
    }
}

/// A device the device knows, from `RESP_CONTACT` or `PUSH_NEW_ADVERT`.
#[derive(Clone, Debug, PartialEq)]
pub struct Contact {
    pub public_key: [u8; 32],
    pub name: String,
    /// The path to it, encoded: the low six bits count its hops, and
    /// `FLOOD` means the path is not known.
    pub path_len: u8,
    /// When the device last heard from it, by the device's clock.
    pub heard_at: u32,
}

impl Contact {
    pub fn parse(body: &[u8]) -> Option<Self> {
        if body.len() < 148 || !matches!(body[0], RESP_CONTACT | PUSH_NEW_ADVERT) {
            return None;
        }
        Some(Self {
            public_key: body[1..33].try_into().ok()?,
            path_len: body[35],
            name: text(&body[100..132]),
            heard_at: u32_at(body, 144)?,
        })
    }

    /// Hops between it and the device, if the path is known.
    pub fn hops(&self) -> Option<u32> {
        (self.path_len != FLOOD).then_some(u32::from(self.path_len & 0x3F))
    }
}

/// A channel datagram, from `RESP_CHANNEL_DATA`.
#[derive(Clone, Debug, PartialEq)]
pub struct ChannelData {
    /// The signal-to-noise ratio it was heard with, in dB.
    pub snr: f32,
    pub channel: u8,
    pub data_type: u16,
    pub data: Vec<u8>,
}

impl ChannelData {
    pub fn parse(body: &[u8]) -> Option<Self> {
        if body.len() < 9 || body[0] != RESP_CHANNEL_DATA {
            return None;
        }
        let len = usize::from(body[8]);
        Some(Self {
            snr: f32::from(body[1] as i8) / 4.0,
            channel: body[4],
            data_type: u16::from_le_bytes([body[6], body[7]]),
            data: body.get(9..9 + len)?.to_vec(),
        })
    }
}

/// The key that names a device: the start of its public key.
pub fn key_number(public_key: &[u8]) -> u32 {
    u32::from_le_bytes([public_key[0], public_key[1], public_key[2], public_key[3]])
}

fn u32_at(body: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(body.get(at..at + 4)?.try_into().ok()?))
}

/// A NUL-padded UTF-8 string.
fn text(bytes: &[u8]) -> String {
    let end = bytes
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_carrychat_channel_is_a_hashtag_channel() {
        // MeshCore's documented example: #test.
        let hash = Sha256::digest(b"#test");
        assert_eq!(
            hash[..16],
            [
                0x9c, 0xd8, 0xfc, 0xf2, 0x2a, 0x47, 0x33, 0x3b, 0x59, 0x1d, 0x96, 0xa2, 0xb8, 0x48,
                0xb7, 0x3f
            ]
        );
        assert_eq!(channel_secret()[..], Sha256::digest(b"#carrychat")[..16]);
    }

    #[test]
    fn frames_are_found_between_other_bytes() {
        let mut deframer = Deframer::new(FROM_DEVICE);
        let mut bytes = b"log line\n".to_vec();
        bytes.extend(frame_from_device(&[RESP_OK]));
        bytes.extend(b">\xff\xff");
        bytes.extend(frame_from_device(&[RESP_NO_MORE_MESSAGES]));
        deframer.push(&bytes[..12]);
        assert_eq!(deframer.next_frame(), None);
        deframer.push(&bytes[12..]);
        assert_eq!(deframer.next_frame(), Some(vec![RESP_OK]));
        assert_eq!(deframer.next_frame(), Some(vec![RESP_NO_MORE_MESSAGES]));
        assert_eq!(deframer.next_frame(), None);
    }

    #[test]
    fn a_channel_datagram_reads_back() {
        let body = [
            RESP_CHANNEL_DATA,
            (-22i8) as u8,
            0,
            0,
            2,
            1,
            0xCC,
            0xFF,
            3,
            7,
            8,
            9,
        ];
        assert_eq!(
            ChannelData::parse(&body),
            Some(ChannelData {
                snr: -5.5,
                channel: 2,
                data_type: DATA_TYPE,
                data: vec![7, 8, 9],
            })
        );
        assert_eq!(
            send_channel_data(2, &[7, 8, 9]),
            [CMD_SEND_CHANNEL_DATA, 2, FLOOD, 0xCC, 0xFF, 7, 8, 9]
        );
    }
}
