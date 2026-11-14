//! A message blinked as light by one node is read by another's watcher.

use carrychat_node::{Event, LightListener, Node, Via};
use carrychat_proto::light::HALF_BIT_MS;
use tempfile::TempDir;

/// What a camera at 30 pictures a second sees of a flashlight blinking
/// `pattern` twice: each picture the light over the 30 ms before it, lit
/// at 140 and dark at 60.
fn film(pattern: &[bool]) -> Vec<(f64, f32)> {
    let half = f64::from(HALF_BIT_MS);
    let lit_at = |at: f64| at >= 0.0 && pattern[(at / half) as usize % pattern.len()];
    let length = pattern.len() as f64 * half * 2.0;
    (0..)
        .map(|index| f64::from(index) * 1000.0 / 30.0)
        .take_while(|&at| at < length)
        .map(|at| {
            let lit = (0..30).filter(|ms| lit_at(at - f64::from(*ms))).count();
            (at, 60.0 + 80.0 * lit as f32 / 30.0)
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_blinked_as_light_reaches_a_node_that_watches() {
    let (alice_dir, bob_dir) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    let alice = Node::create(alice_dir.path(), "Ayşe").unwrap();
    let bob = Node::create(bob_dir.path(), "Bora").unwrap();
    let bob_id = alice.add_contact(&bob.profile().card).unwrap().id;
    bob.add_contact(&alice.profile().card).unwrap();
    let mut bob_events = bob.subscribe();

    let text = "Köprü yıkıldı, kuzeyden dolaşın.";
    let message = alice.send_text(bob_id, text).unwrap();
    let pattern = alice.message_light(message.id).unwrap();
    let mut listener = LightListener::new();
    let mut last = None;
    for pictures in film(&pattern).chunks(8) {
        last = Some(listener.push(&bob, pictures));
    }
    // A first message carries Ayşe's card (§5.5): two frames, two rounds.
    let watch = last.unwrap();
    assert_eq!(watch.frames, 2);
    assert_eq!(watch.blocks, watch.of);

    let received = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Event::MessageReceived { message } = bob_events.recv().await.unwrap() {
                return message;
            }
        }
    })
    .await
    .expect("the message was not read");
    assert_eq!(received.body, text);
    assert!(alice
        .journey(message.id)
        .unwrap()
        .steps
        .iter()
        .any(|step| step.via == Via::Light));
}
