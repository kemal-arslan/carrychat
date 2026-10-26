//! A frame is handled whole or not at all: a write that fails half way must
//! not leave a receipt for a message that was not kept.

use carrychat_node::engine::{Engine, Outcome};
use carrychat_node::store::Store;
use carrychat_node::Event;
use carrychat_proto::frame::Frame;
use carrychat_proto::identity::ContactCard;
use rusqlite::Connection;
use tempfile::TempDir;

const NOW: u32 = 1_790_000_000;

fn dh_key(engine: &Engine) -> [u8; 32] {
    *ContactCard::from_text(&engine.profile().card)
        .unwrap()
        .dh_key()
        .as_bytes()
}

#[test]
fn a_failed_write_keeps_no_receipt_and_the_message_still_opens() {
    let dir = TempDir::new().unwrap();
    let paths = [dir.path().join("a.db"), dir.path().join("b.db")];
    let mut a = Engine::create(Store::open(&paths[0]).unwrap(), "a").unwrap();
    let mut b = Engine::create(Store::open(&paths[1]).unwrap(), "b").unwrap();
    let b_at_a = a.add_contact(&b.profile().card, NOW).unwrap().0.id;
    let a_at_b = b.add_contact(&a.profile().card, NOW).unwrap().0.id;
    let (_, intro) = a.send_text(b_at_a, "selam", NOW).unwrap();
    let received = b.handle(intro, NOW).unwrap();
    a.handle(received.relay[0].clone(), NOW).unwrap();

    // The initiator writes, so the message is a ratchet message (§5.6): opening
    // it moves the receiver's session on before the message is stored.
    let ((mut sender, to), (mut receiver, from, path)) = if dh_key(&a) < dh_key(&b) {
        ((a, b_at_a), (b, a_at_b, &paths[1]))
    } else {
        ((b, a_at_b), (a, b_at_a, &paths[0]))
    };
    let (sent, frame) = sender.send_text(to, "kaybolmamalı", NOW + 10).unwrap();
    let Frame::Bundle(bundle) = &frame else {
        panic!("expected a bundle");
    };
    let id = bundle.id;

    // The receiver's disk refuses to store messages.
    let sql = Connection::open(path).unwrap();
    sql.execute_batch(
        "CREATE TRIGGER fail BEFORE INSERT ON messages BEGIN SELECT RAISE(ABORT, 'disk full'); END;",
    )
    .unwrap();
    let failed = receiver.atomically(|engine| engine.handle(frame.clone(), NOW + 20));
    assert!(failed.is_err());
    let receipts = receiver.tombstone_frames().unwrap();
    assert!(!receipts.iter().any(
        |receipt| matches!(receipt, Frame::Tombstone { tombstone, .. } if tombstone.id == id)
    ));
    assert_eq!(receiver.messages(from).unwrap().len(), 1);

    // Once it can write again, the same bundle opens: nothing was used up.
    sql.execute_batch("DROP TRIGGER fail;").unwrap();
    let handled = receiver
        .atomically(|engine| engine.handle(frame, NOW + 30))
        .unwrap();
    assert_eq!(handled.outcome, Outcome::Received);
    assert!(
        matches!(&handled.events[..1], [Event::MessageReceived { message }] if message.body == "kaybolmamalı")
    );
    let receipt = handled.relay[0].clone();
    assert_eq!(
        sender.handle(receipt, NOW + 40).unwrap().events,
        vec![Event::MessageDelivered {
            contact_id: to,
            message_id: sent.id
        }]
    );
}
