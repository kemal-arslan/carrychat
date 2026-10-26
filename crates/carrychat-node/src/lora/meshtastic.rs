//! The part of the Meshtastic client API that the LoRa link uses (§9.2):
//! the stream framing shared by USB serial and TCP, and the few protobuf
//! messages it reads and writes, with the field numbers of Meshtastic's
//! `mesh.proto` and `config.proto`.

use prost::Message;

/// A frame on the stream is `0x94 0xC3 || u16 length || protobuf`.
pub const START1: u8 = 0x94;
pub const START2: u8 = 0xC3;
pub const MAX_PACKET_LEN: usize = 512;
/// `PortNum.PRIVATE_APP`, which CarryChat uses until it has a number of its own.
pub const PRIVATE_APP: i32 = 256;
pub const BROADCAST: u32 = 0xffff_ffff;
/// Payload bytes of one Meshtastic packet (`DATA_PAYLOAD_LEN`).
pub const MTU: usize = 233;

#[derive(Clone, PartialEq, Message)]
pub struct ToRadio {
    #[prost(oneof = "to_radio::Payload", tags = "1, 3, 7")]
    pub payload: Option<to_radio::Payload>,
}

pub mod to_radio {
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum Payload {
        #[prost(message, tag = "1")]
        Packet(super::MeshPacket),
        /// Asks the device for its configuration; it ends with
        /// `config_complete_id` carrying the same number.
        #[prost(uint32, tag = "3")]
        WantConfigId(u32),
        #[prost(message, tag = "7")]
        Heartbeat(super::Heartbeat),
    }
}

#[derive(Clone, PartialEq, Message)]
pub struct FromRadio {
    #[prost(uint32, tag = "1")]
    pub id: u32,
    #[prost(oneof = "from_radio::Payload", tags = "2, 3, 5, 7")]
    pub payload: Option<from_radio::Payload>,
}

pub mod from_radio {
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum Payload {
        #[prost(message, tag = "2")]
        Packet(super::MeshPacket),
        #[prost(message, tag = "3")]
        MyInfo(super::MyNodeInfo),
        #[prost(message, tag = "5")]
        Config(super::Config),
        #[prost(uint32, tag = "7")]
        ConfigCompleteId(u32),
    }
}

#[derive(Clone, PartialEq, Message)]
pub struct MeshPacket {
    #[prost(fixed32, tag = "1")]
    pub from: u32,
    #[prost(fixed32, tag = "2")]
    pub to: u32,
    #[prost(uint32, tag = "3")]
    pub channel: u32,
    #[prost(oneof = "mesh_packet::Payload", tags = "4, 5")]
    pub payload: Option<mesh_packet::Payload>,
    #[prost(fixed32, tag = "6")]
    pub id: u32,
    #[prost(uint32, tag = "9")]
    pub hop_limit: u32,
}

pub mod mesh_packet {
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum Payload {
        #[prost(message, tag = "4")]
        Decoded(super::Data),
        /// A packet on a channel the device has no key for.
        #[prost(bytes, tag = "5")]
        Encrypted(Vec<u8>),
    }
}

#[derive(Clone, PartialEq, Message)]
pub struct Data {
    #[prost(int32, tag = "1")]
    pub portnum: i32,
    #[prost(bytes = "vec", tag = "2")]
    pub payload: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Heartbeat {}

#[derive(Clone, PartialEq, Message)]
pub struct MyNodeInfo {
    #[prost(uint32, tag = "1")]
    pub my_node_num: u32,
}

#[derive(Clone, PartialEq, Message)]
pub struct Config {
    #[prost(oneof = "config::Payload", tags = "6")]
    pub payload: Option<config::Payload>,
}

pub mod config {
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum Payload {
        #[prost(message, tag = "6")]
        Lora(super::LoRaConfig),
    }
}

#[derive(Clone, PartialEq, Message)]
pub struct LoRaConfig {
    #[prost(bool, tag = "1")]
    pub use_preset: bool,
    #[prost(int32, tag = "2")]
    pub modem_preset: i32,
    /// kHz, when no preset is used.
    #[prost(uint32, tag = "3")]
    pub bandwidth: u32,
    #[prost(uint32, tag = "4")]
    pub spread_factor: u32,
    /// The denominator of the coding rate: 5 to 8 for 4/5 to 4/8.
    #[prost(uint32, tag = "5")]
    pub coding_rate: u32,
    #[prost(uint32, tag = "8")]
    pub hop_limit: u32,
}

/// Wraps a message in the stream framing.
pub fn frame<M: Message>(message: &M) -> Vec<u8> {
    let body = message.encode_to_vec();
    let mut bytes = Vec::with_capacity(4 + body.len());
    bytes.extend_from_slice(&[START1, START2]);
    bytes.extend_from_slice(&(body.len() as u16).to_be_bytes());
    bytes.extend_from_slice(&body);
    bytes
}

/// Cuts framed messages out of a byte stream. Bytes outside frames, such as
/// the debug log a device prints on its serial port, are skipped.
#[derive(Default)]
pub struct Deframer {
    buffer: Vec<u8>,
}

impl Deframer {
    pub fn push(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
    }

    /// The next whole frame's protobuf bytes, if one is in.
    pub fn next_frame(&mut self) -> Option<Vec<u8>> {
        loop {
            let start = self
                .buffer
                .windows(2)
                .position(|pair| pair == [START1, START2]);
            let Some(start) = start else {
                // Keep a trailing START1, which may begin the next frame.
                let keep = usize::from(self.buffer.last() == Some(&START1));
                self.buffer.drain(..self.buffer.len() - keep);
                return None;
            };
            self.buffer.drain(..start);
            if self.buffer.len() < 4 {
                return None;
            }
            let len = u16::from_be_bytes([self.buffer[2], self.buffer[3]]) as usize;
            if len > MAX_PACKET_LEN {
                // Not a frame after all: look past this start.
                self.buffer.drain(..1);
                continue;
            }
            if self.buffer.len() < 4 + len {
                return None;
            }
            let body = self.buffer[4..4 + len].to_vec();
            self.buffer.drain(..4 + len);
            return Some(body);
        }
    }
}

/// How the radio modulates, which sets how long a packet is on the air.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Modulation {
    pub spread_factor: u32,
    pub bandwidth_hz: u32,
    /// 1 to 4 for 4/5 to 4/8.
    pub coding_rate: u32,
}

impl Modulation {
    /// Meshtastic's default preset, LongFast.
    pub const LONG_FAST: Self = Self {
        spread_factor: 11,
        bandwidth_hz: 250_000,
        coding_rate: 1,
    };

    pub fn of(config: &LoRaConfig) -> Self {
        if !config.use_preset && config.spread_factor > 0 && config.bandwidth > 0 {
            return Self {
                spread_factor: config.spread_factor,
                bandwidth_hz: config.bandwidth * 1000,
                coding_rate: config.coding_rate.clamp(5, 8) - 4,
            };
        }
        // `ModemPreset` values of config.proto.
        let (spread_factor, bandwidth_hz, coding_rate) = match config.modem_preset {
            1 => (12, 125_000, 4), // LONG_SLOW
            2 => (12, 62_500, 4),  // VERY_LONG_SLOW
            3 => (10, 250_000, 1), // MEDIUM_SLOW
            4 => (9, 250_000, 1),  // MEDIUM_FAST
            5 => (8, 250_000, 1),  // SHORT_SLOW
            6 => (7, 250_000, 1),  // SHORT_FAST
            7 => (11, 125_000, 4), // LONG_MODERATE
            8 => (7, 500_000, 1),  // SHORT_TURBO
            _ => (11, 250_000, 1), // LONG_FAST
        };
        Self {
            spread_factor,
            bandwidth_hz,
            coding_rate,
        }
    }

    /// Seconds a packet of `len` bytes is on the air, by Semtech's formula
    /// for the SX126x, with Meshtastic's 16-symbol preamble and a CRC.
    pub fn airtime(&self, len: usize) -> f64 {
        let sf = f64::from(self.spread_factor);
        let symbol = 2f64.powf(sf) / f64::from(self.bandwidth_hz);
        let low_rate = if symbol > 0.016 { 1.0 } else { 0.0 };
        let bits = 8.0 * len as f64 - 4.0 * sf + 28.0 + 16.0;
        let symbols = 8.0
            + ((bits / (4.0 * (sf - 2.0 * low_rate))).ceil() * f64::from(self.coding_rate + 4))
                .max(0.0);
        (16.0 + 4.25 + symbols) * symbol
    }
}

/// Bytes Meshtastic adds around a CarryChat frame on the air: its 16-byte
/// header and the protobuf fields of `Data`.
pub const OVERHEAD: usize = 22;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_survive_noise_and_splits() {
        let packet = ToRadio {
            payload: Some(to_radio::Payload::WantConfigId(77)),
        };
        let mut stream = b"INFO | booting\r\n".to_vec();
        stream.extend(frame(&packet));
        stream.push(START1);
        stream.extend(b"x\x94");
        stream.extend(frame(&packet));

        let mut deframer = Deframer::default();
        let mut found = Vec::new();
        for byte in stream {
            deframer.push(&[byte]);
            while let Some(body) = deframer.next_frame() {
                found.push(ToRadio::decode(body.as_slice()).unwrap());
            }
        }
        assert_eq!(found, vec![packet.clone(), packet]);
    }

    fn hex(text: &str) -> Vec<u8> {
        data_encoding::HEXLOWER.decode(text.as_bytes()).unwrap()
    }

    /// Bytes from Meshtastic's own protobufs (the Python package, 2.7.11),
    /// so the field numbers here cannot drift from theirs unnoticed. The
    /// payload text, and the lengths before it, were changed by hand.
    #[test]
    fn messages_match_meshtastic_s_own_encoding() {
        let packet = ToRadio {
            payload: Some(to_radio::Payload::Packet(MeshPacket {
                to: BROADCAST,
                id: 0x0102_0304,
                hop_limit: 3,
                payload: Some(mesh_packet::Payload::Decoded(Data {
                    portnum: PRIVATE_APP,
                    payload: b"\x01carrychat".to_vec(),
                })),
                ..MeshPacket::default()
            })),
        };
        assert_eq!(
            packet.encode_to_vec(),
            hex("0a1d15ffffffff220f088002120a0163617272796368617435040302014803")
        );
        let want_config = ToRadio {
            payload: Some(to_radio::Payload::WantConfigId(77)),
        };
        assert_eq!(want_config.encode_to_vec(), hex("184d"));
        let heartbeat = ToRadio {
            payload: Some(to_radio::Payload::Heartbeat(Heartbeat {})),
        };
        assert_eq!(heartbeat.encode_to_vec(), hex("3a00"));

        let decode = |text| FromRadio::decode(hex(text).as_slice()).unwrap().payload;
        // MEDIUM_FAST, hop limit 5, with the region and tx_enabled skipped.
        let Some(from_radio::Payload::Config(Config {
            payload: Some(config::Payload::Lora(lora)),
        })) = decode("08092a0c320a08011004380340054801")
        else {
            panic!("expected a LoRa config");
        };
        assert_eq!(
            (lora.use_preset, lora.modem_preset, lora.hop_limit),
            (true, 4, 5)
        );
        assert_eq!(
            decode("080a384d"),
            Some(from_radio::Payload::ConfigCompleteId(77))
        );
        assert_eq!(
            decode("080b1a0308d209"),
            Some(from_radio::Payload::MyInfo(MyNodeInfo {
                my_node_num: 1234
            }))
        );
        // A packet heard, with signal fields this module does not read.
        let Some(from_radio::Payload::Packet(heard)) = decode(
            "080c122e0d5500000015ffffffff22090880021204036162633507000000450000d040480260a6ffffffffffffffff017803",
        ) else {
            panic!("expected a packet");
        };
        assert_eq!(
            (heard.from, heard.to, heard.id, heard.hop_limit),
            (0x55, BROADCAST, 7, 2)
        );
        assert_eq!(
            heard.payload,
            Some(mesh_packet::Payload::Decoded(Data {
                portnum: PRIVATE_APP,
                payload: b"\x03abc".to_vec(),
            }))
        );
    }

    #[test]
    fn a_full_packet_takes_about_two_seconds_on_long_fast() {
        let airtime = Modulation::LONG_FAST.airtime(MTU + OVERHEAD);
        assert!((2.0..2.4).contains(&airtime), "{airtime}");
        // A tombstone: a quarter of a full packet, most of it preamble and header.
        let short = Modulation::LONG_FAST.airtime(26 + OVERHEAD);
        assert!((0.5..0.7).contains(&short), "{short}");
        let fast = Modulation::of(&LoRaConfig {
            use_preset: true,
            modem_preset: 6,
            ..LoRaConfig::default()
        });
        assert!(fast.airtime(MTU + OVERHEAD) < 0.5);
    }
}
