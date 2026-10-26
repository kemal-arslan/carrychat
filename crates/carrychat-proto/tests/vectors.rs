//! Checks the implementation against `docs/test-vectors.json`.

#[path = "../examples/gen_vectors.rs"]
#[allow(dead_code)]
mod gen_vectors;

use carrychat_proto::bundle::{open, Bundle, Kind, Tombstone};
use carrychat_proto::frame::{Frame, Reassembler};
use carrychat_proto::identity::{ContactCard, Identity};
use carrychat_proto::intro::open_introduction;
use carrychat_proto::pair::PairKeys;
use data_encoding::HEXLOWER;
use serde_json::Value;

const VECTORS: &str = include_str!("../../../docs/test-vectors.json");

fn stored() -> Value {
    serde_json::from_str(VECTORS).unwrap()
}

fn hex(value: &Value) -> Vec<u8> {
    HEXLOWER.decode(value.as_str().unwrap().as_bytes()).unwrap()
}

#[test]
fn output_matches_the_committed_vectors() {
    assert_eq!(
        gen_vectors::vectors(),
        stored(),
        "regenerate with: cargo run -p carrychat-proto --example gen_vectors > docs/test-vectors.json"
    );
}

#[test]
fn bob_opens_the_stored_bundles() {
    let vectors = stored();
    let alice = ContactCard::from_text(vectors["alice"]["card"].as_str().unwrap()).unwrap();
    let bob = Identity::from_secret_bytes(
        &hex(&vectors["bob"]["ed25519_secret"]).try_into().unwrap(),
        &hex(&vectors["bob"]["x25519_secret"]).try_into().unwrap(),
        "bob",
    )
    .unwrap();
    let keys = PairKeys::derive(&bob, alice.dh_key()).unwrap();

    for message in vectors["messages"].as_array().unwrap() {
        let bundle = Bundle::from_bytes(&hex(&message["bundle"])).unwrap();
        let opened = open(&keys, &bob.dh_public(), &bundle).unwrap();
        assert_eq!(opened.kind, Kind::Text);
        assert_eq!(opened.body, message["text"].as_str().unwrap().as_bytes());

        let tombstone = Tombstone::for_bundle(&keys, &bundle);
        assert!(tombstone.verify());
        assert_eq!(tombstone.ack_secret.to_vec(), hex(&message["ack_secret"]));

        let mut reassembler = Reassembler::default();
        let mut received = None;
        for frame in message["meshtastic_frames"].as_array().unwrap() {
            match Frame::from_bytes(&hex(frame)).unwrap() {
                Frame::Bundle(whole) => received = Some(whole),
                Frame::Fragment(fragment) => {
                    received = reassembler.push(fragment, 0).unwrap().or(received)
                }
                Frame::Tombstone { .. } | Frame::Chunk(_) => panic!("unexpected frame"),
            }
        }
        assert_eq!(received, Some(bundle));
    }
}

#[test]
fn bob_opens_the_stored_introduction() {
    let vectors = stored();
    let bob = Identity::from_secret_bytes(
        &hex(&vectors["bob"]["ed25519_secret"]).try_into().unwrap(),
        &hex(&vectors["bob"]["x25519_secret"]).try_into().unwrap(),
        "bob",
    )
    .unwrap();
    let stored = &vectors["introduction"];
    let bundle = Bundle::from_bytes(&hex(&stored["bundle"])).unwrap();
    let intro = open_introduction(&bob, &bundle).unwrap();
    assert_eq!(
        intro.card.to_text(),
        vectors["alice"]["card"].as_str().unwrap()
    );
    assert_eq!(intro.text, stored["text"].as_str().unwrap());
    assert!(intro.tombstone.verify());
}
