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
use kervan_proto::group::{GroupId, GroupState, ReceiverChain, SenderChain};
use kervan_proto::identity::Identity;
use kervan_proto::intro::{mailbox_tag, seal_introduction_with};
use kervan_proto::pair::PairKeys;
use kervan_proto::ratchet::{Role, Session};
use kervan_proto::{day_of, DAY_SECS};
use rand_core::{CryptoRng, RngCore};
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
        "ratchet": ratchet(&alice, &bob),
        "group": group(&alice, &bob),
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
        "mailbox_tag": HEXLOWER.encode(&mailbox_tag(&bob.dh_public(), day_of(CREATED_AT))),
    })
}

/// Hands out the same 32 bytes every time: the ratchet keys of the vectors.
struct Fixed([u8; 32]);

impl RngCore for Fixed {
    fn next_u32(&mut self) -> u32 {
        u32::from_le_bytes(self.0[..4].try_into().unwrap())
    }
    fn next_u64(&mut self) -> u64 {
        u64::from_le_bytes(self.0[..8].try_into().unwrap())
    }
    fn fill_bytes(&mut self, dest: &mut [u8]) {
        dest.copy_from_slice(&self.0[..dest.len()]);
    }
    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
        self.fill_bytes(dest);
        Ok(())
    }
}

impl CryptoRng for Fixed {}

/// Alice, the initiator, starts a session; Bob answers on his first chain (§5.6).
fn ratchet(alice: &Identity, bob: &Identity) -> Value {
    let alice_keys = PairKeys::derive(alice, &bob.dh_public()).unwrap();
    let bob_keys = PairKeys::derive(bob, &alice.dh_public()).unwrap();
    let mut alice_session = Session::new(alice, &bob.dh_public(), &alice_keys);
    let mut bob_session = Session::new(bob, &alice.dh_public(), &bob_keys);
    assert_eq!(alice_session.role(), Role::Initiator);
    let expires_at = CREATED_AT + 3 * DAY_SECS;

    let (alice_ratchet, first_nonce) = ([0x88; 32], [0x89; 24]);
    let first_text = "Oturumu ben başlatıyorum.";
    let first = alice_session
        .seal_with_nonce(
            &alice_keys,
            &bob.dh_public(),
            &Message::text(first_text, CREATED_AT),
            expires_at,
            8,
            first_nonce,
            &mut Fixed(alice_ratchet),
        )
        .unwrap();

    let bob_ratchet = [0x99; 32];
    let opened = bob_session
        .open(
            &bob_keys,
            &bob.dh_public(),
            &first,
            |_, _| None,
            &mut Fixed(bob_ratchet),
        )
        .unwrap();
    assert_eq!(opened.message.body, first_text.as_bytes());
    bob_session = opened.session;

    let (second_nonce, second_text) = ([0x9a; 24], "Aldım, şimdi ben yazıyorum.");
    let second = bob_session
        .seal_with_nonce(
            &bob_keys,
            &alice.dh_public(),
            &Message::text(second_text, CREATED_AT + 60),
            expires_at,
            8,
            second_nonce,
            &mut Fixed([0; 32]),
        )
        .unwrap();
    let back = alice_session
        .open(
            &alice_keys,
            &alice.dh_public(),
            &second,
            |_, _| None,
            &mut Fixed([0x77; 32]),
        )
        .unwrap();
    assert_eq!(back.message.body, second_text.as_bytes());

    json!({
        "description": "Alice's first ratchet message starts the session; Bob's reply is the first on his own chain.",
        "first": {
            "text": first_text,
            "created_at": CREATED_AT,
            "expires_at": expires_at,
            "hop_limit": 8,
            "ratchet_secret": HEXLOWER.encode(&alice_ratchet),
            "nonce": HEXLOWER.encode(&first_nonce),
            "bundle": HEXLOWER.encode(&first.to_bytes()),
        },
        "reply": {
            "text": second_text,
            "created_at": CREATED_AT + 60,
            "expires_at": expires_at,
            "hop_limit": 8,
            "ratchet_secret": HEXLOWER.encode(&bob_ratchet),
            "nonce": HEXLOWER.encode(&second_nonce),
            "bundle": HEXLOWER.encode(&second.to_bytes()),
        },
    })
}

/// Alice, the admin, writes twice to a group of two on her sender chain (§5.7).
fn group(alice: &Identity, bob: &Identity) -> Value {
    let state = GroupState {
        id: GroupId([0xa1; 16]),
        version: 2,
        name: "Mahalle".into(),
        members: vec![alice.card(), bob.card()],
    };
    let (sender_key, chain, signing) = ([0xb1; 32], [0xb2; 32], [0xb3; 32]);
    let mut sender = SenderChain::from_bytes(
        &[sender_key.as_slice(), &chain, &0u32.to_be_bytes(), &signing].concat(),
    )
    .unwrap();
    let distribution = sender.distribution(state.id);
    let mut receiver = ReceiverChain::new(&distribution);
    let expires_at = CREATED_AT + 3 * DAY_SECS;

    let messages: Vec<Value> = [
        ("Su dağıtımı okulun önünde.", CREATED_AT, [0xc1; 24]),
        ("Saat ondan sonra.", CREATED_AT + 30, [0xc2; 24]),
    ]
    .into_iter()
    .map(|(text, created_at, nonce)| {
        let iteration = sender.iteration();
        let bundle = sender
            .seal_with_nonce(&Message::text(text, created_at), expires_at, 8, nonce)
            .unwrap();
        let opened = receiver.open(&bundle, |_| None).unwrap();
        assert_eq!(opened.message.body, text.as_bytes());
        receiver = opened.chain;
        json!({
            "text": text,
            "created_at": created_at,
            "expires_at": expires_at,
            "hop_limit": 8,
            "iteration": iteration,
            "nonce": HEXLOWER.encode(&nonce),
            "bundle": HEXLOWER.encode(&bundle.to_bytes()),
        })
    })
    .collect();

    json!({
        "description": "Alice's group state, her sender chain as she hands it to Bob, and two messages on it.",
        "state": HEXLOWER.encode(&state.to_bytes().unwrap()),
        "group_id": HEXLOWER.encode(&state.id.0),
        "version": state.version,
        "name": state.name,
        "sender_key": HEXLOWER.encode(&sender_key),
        "chain_key": HEXLOWER.encode(&chain),
        "ed25519_secret": HEXLOWER.encode(&signing),
        "distribution": HEXLOWER.encode(&distribution.to_bytes()),
        "messages": messages,
    })
}
