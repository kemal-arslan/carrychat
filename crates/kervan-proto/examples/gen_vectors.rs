//! Prints the protocol test vectors.
//!
//! ```sh
//! cargo run -p kervan-proto --example gen_vectors > docs/test-vectors.json
//! ```
//!
//! `tests/vectors.rs` fails if the output no longer matches the committed file.

use data_encoding::HEXLOWER;
use kervan_proto::bundle::{seal_with_nonce, Message, Tombstone};
use kervan_proto::frame::{frames_for, Frame, MESHTASTIC_MTU};
use kervan_proto::identity::Identity;
use kervan_proto::intro::seal_introduction_with;
use kervan_proto::pair::PairKeys;
use kervan_proto::{day_of, DAY_SECS};
use serde_json::{json, Value};

const CREATED_AT: u32 = 1_790_000_000;

pub fn vectors() -> Value {
    let alice = Identity::from_secret_bytes(&[0x11; 32], &[0x12; 32], "alice").unwrap();
    let bob = Identity::from_secret_bytes(&[0x21; 32], &[0x22; 32], "bob").unwrap();
    let keys = PairKeys::derive(&alice, &bob.dh_public()).unwrap();
    let (k_enc, k_tag, k_ack) = keys.expose_secret_keys();

    let short = "İyiyim, Kadıköy'deyim. Su ve ilaç lazım.";
    let long = "Köprü yıkıldı, karşıya geçemiyoruz. ".repeat(16);
    let messages: Vec<Value> = [(short, [0x33; 24]), (long.as_str(), [0x44; 24])]
        .into_iter()
        .map(|(text, nonce)| {
            let message = Message::text(text, CREATED_AT);
            let expires_at = CREATED_AT + 3 * DAY_SECS;
            let bundle =
                seal_with_nonce(&keys, &bob.dh_public(), &message, expires_at, 8, nonce).unwrap();
            let tombstone = Tombstone::for_bundle(&keys, &bundle);
            let frames: Vec<String> = frames_for(&bundle, MESHTASTIC_MTU)
                .unwrap()
                .iter()
                .map(|frame| HEXLOWER.encode(&frame.to_bytes()))
                .collect();
            json!({
                "text": text,
                "created_at": CREATED_AT,
                "expires_at": expires_at,
                "hop_limit": 8,
                "nonce": HEXLOWER.encode(&nonce),
                "recipient_tag": HEXLOWER.encode(&bundle.tag),
                "ack_secret": HEXLOWER.encode(&tombstone.ack_secret),
                "bundle_id": HEXLOWER.encode(&bundle.id.0),
                "bundle": HEXLOWER.encode(&bundle.to_bytes()),
                "meshtastic_frames": frames,
                "tombstone_frame": HEXLOWER.encode(
                    &Frame::Tombstone { hop_limit: 8, tombstone }.to_bytes()
                ),
            })
        })
        .collect();

    json!({
        "description": "Kervan protocol v0 test vectors. Alice sends both messages to Bob. Byte strings are lowercase hex.",
        "alice": identity_json(&alice, [0x11; 32], [0x12; 32]),
        "bob": identity_json(&bob, [0x21; 32], [0x22; 32]),
        "pair": {
            "k_enc": HEXLOWER.encode(k_enc),
            "k_tag": HEXLOWER.encode(k_tag),
            "k_ack": HEXLOWER.encode(k_ack),
        },
        "day": day_of(CREATED_AT),
        "messages": messages,
        "introduction": introduction(&alice, &bob),
    })
}

fn identity_json(identity: &Identity, signing: [u8; 32], dh: [u8; 32]) -> Value {
    let card = identity.card();
    json!({
        "name": identity.name(),
        "ed25519_secret": HEXLOWER.encode(&signing),
        "x25519_secret": HEXLOWER.encode(&dh),
        "ed25519_public": HEXLOWER.encode(card.signing_key().as_bytes()),
        "x25519_public": HEXLOWER.encode(card.dh_key().as_bytes()),
        "fingerprint": identity.fingerprint().to_string(),
        "card": card.to_text(),
    })
}

fn main() {
    println!("{}", serde_json::to_string_pretty(&vectors()).unwrap());
}

/// Alice writes to Bob, who does not have her card yet (§5.5).
fn introduction(alice: &Identity, bob: &Identity) -> Value {
    let text = "Merhaba Bob, ben Alice. Kartım bu mesajın içinde.";
    let (ephemeral, nonce, tag) = ([0x55; 32], [0x66; 24], [0x77; 8]);
    let expires_at = CREATED_AT + 3 * DAY_SECS;
    let bundle = seal_introduction_with(
        alice,
        &bob.dh_public(),
        text,
        CREATED_AT,
        expires_at,
        8,
        ephemeral,
        nonce,
        tag,
    )
    .unwrap();
    let frames: Vec<String> = frames_for(&bundle, MESHTASTIC_MTU)
        .unwrap()
        .iter()
        .map(|frame| HEXLOWER.encode(&frame.to_bytes()))
        .collect();
    json!({
        "text": text,
        "created_at": CREATED_AT,
        "expires_at": expires_at,
        "hop_limit": 8,
        "ephemeral_secret": HEXLOWER.encode(&ephemeral),
        "nonce": HEXLOWER.encode(&nonce),
        "tag": HEXLOWER.encode(&tag),
        "bundle_id": HEXLOWER.encode(&bundle.id.0),
        "bundle": HEXLOWER.encode(&bundle.to_bytes()),
        "meshtastic_frames": frames,
    })
}
