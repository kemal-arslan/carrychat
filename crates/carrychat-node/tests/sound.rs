//! A card played as sound by one node is heard by another's listener.

use carrychat_node::{Event, HeardCard, Node, SoundBand, SoundListener, Via};
use tempfile::TempDir;

/// The samples of a WAV file `card_sound` or `message_sound` made.
fn samples_of(wav: &[u8]) -> Vec<f32> {
    wav[44..]
        .chunks_exact(2)
        .map(|pair| f32::from(i16::from_le_bytes([pair[0], pair[1]])) / 32768.0)
        .collect()
}

#[test]
fn a_card_played_as_sound_is_heard_and_verified() {
    let dir = TempDir::new().unwrap();
    let node = Node::create(dir.path(), "Ayşe Yılmaz").unwrap();
    let other = TempDir::new().unwrap();
    let listening = Node::create(other.path(), "Deniz").unwrap();
    for band in [SoundBand::Audible, SoundBand::Inaudible] {
        let samples = samples_of(&node.card_sound(band).unwrap());
        let mut listener = SoundListener::new(48_000);
        let heard = listener
            .push(&listening, &samples)
            .expect("the card was not heard");
        assert_eq!(heard.name, "Ayşe Yılmaz");
        assert_eq!(heard.card, node.profile().card);
    }
}

#[test]
fn a_card_read_over_nfc_is_verified() {
    let dir = TempDir::new().unwrap();
    let node = Node::create(dir.path(), "Deniz").unwrap();
    let card = node.profile().card;
    assert_eq!(HeardCard::from_text(&card).unwrap().name, "Deniz");
    // One character off spoils the signature.
    let mut spoiled = card.into_bytes();
    let last = spoiled.len() - 5;
    spoiled[last] = if spoiled[last] == b'A' { b'B' } else { b'A' };
    assert_eq!(
        HeardCard::from_text(&String::from_utf8(spoiled).unwrap()),
        None
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_played_as_sound_reaches_a_node_that_listens() {
    message_played_in(SoundBand::Audible).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_played_above_hearing_reaches_a_node_that_listens() {
    message_played_in(SoundBand::Inaudible).await;
}

async fn message_played_in(band: SoundBand) {
    let (alice_dir, bob_dir) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    let alice = Node::create(alice_dir.path(), "Ayşe").unwrap();
    let bob = Node::create(bob_dir.path(), "Bora").unwrap();
    let bob_id = alice.add_contact(&bob.profile().card).unwrap().id;
    bob.add_contact(&alice.profile().card).unwrap();
    let mut bob_events = bob.subscribe();

    // Long enough to need more than one round.
    let text = "Yangın merdiveni kapalı, arka kapıdan çıkın. ".repeat(6);
    let message = alice.send_text(bob_id, &text).unwrap();
    let samples = samples_of(&alice.message_sound(message.id, band).unwrap());
    let mut listener = SoundListener::new(48_000);
    for piece in samples.chunks(12_000) {
        assert_eq!(listener.push(&bob, piece), None);
    }

    let received = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Event::MessageReceived { message } = bob_events.recv().await.unwrap() {
                return message;
            }
        }
    })
    .await
    .expect("the message was not heard");
    assert_eq!(received.body, text.trim_end());
    assert!(alice
        .journey(message.id)
        .unwrap()
        .steps
        .iter()
        .any(|step| step.via == Via::Sound));
}
