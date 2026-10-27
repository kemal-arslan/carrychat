//! A card played as sound by one node is heard by another's listener.

use carrychat_node::{CardListener, Node};
use tempfile::TempDir;

#[test]
fn a_card_played_as_sound_is_heard_and_verified() {
    let dir = TempDir::new().unwrap();
    let node = Node::create(dir.path(), "Ayşe Yılmaz").unwrap();
    let wav = node.card_sound().unwrap();
    let samples: Vec<f32> = wav[44..]
        .chunks_exact(2)
        .map(|pair| f32::from(i16::from_le_bytes([pair[0], pair[1]])) / 32768.0)
        .collect();

    let mut listener = CardListener::new(48_000);
    let heard = listener.push(&samples).expect("the card was not heard");
    assert_eq!(heard.name, "Ayşe Yılmaz");
    assert_eq!(heard.card, node.profile().card);
}
