//! Public channels between engines (§5.8), with every frame flooded to every
//! node. None of the nodes are contacts.

use std::collections::VecDeque;

use carrychat_node::engine::Engine;
use carrychat_node::store::Store;
use carrychat_node::{Event, PostView};
use carrychat_proto::frame::Frame;

const NOW: u32 = 1_790_000_000;

struct Net {
    nodes: Vec<Engine>,
    events: Vec<Vec<Event>>,
}

impl Net {
    fn new(names: &[&str]) -> Self {
        let nodes: Vec<Engine> = names
            .iter()
            .map(|name| Engine::create(Store::open_in_memory().unwrap(), name).unwrap())
            .collect();
        Self {
            events: vec![Vec::new(); nodes.len()],
            nodes,
        }
    }

    fn join(&mut self, node: usize, name: &str, password: &str, nick: &str) -> i64 {
        let (channel, opened) = self.nodes[node]
            .join_channel(name, password, nick, NOW)
            .unwrap();
        for handled in opened {
            self.events[node].extend(handled.events);
        }
        channel.id
    }

    fn flood(&mut self, from: usize, frame: Frame) {
        let mut queue = VecDeque::from([(from, frame)]);
        while let Some((sender, frame)) = queue.pop_front() {
            for node in (0..self.nodes.len()).filter(|&node| node != sender) {
                let handled = self.nodes[node].handle(frame.clone(), NOW + 60).unwrap();
                self.events[node].extend(handled.events);
                queue.extend(handled.relay.into_iter().map(|f| (node, f)));
            }
        }
    }

    /// One node posts. Returns the posts the others got from it.
    fn post(&mut self, node: usize, channel: i64, text: &str) -> Vec<(usize, PostView)> {
        let before: Vec<usize> = self.events.iter().map(Vec::len).collect();
        let (_, frame) = self.nodes[node].send_post(channel, text, NOW + 30).unwrap();
        self.flood(node, frame);
        self.received_since(&before)
    }

    fn received_since(&self, before: &[usize]) -> Vec<(usize, PostView)> {
        (0..self.nodes.len())
            .flat_map(|node| {
                self.events[node][before[node]..]
                    .iter()
                    .filter_map(move |event| match event {
                        Event::PostReceived { post } => Some((node, post.clone())),
                        _ => None,
                    })
            })
            .collect()
    }
}

#[test]
fn strangers_who_know_the_name_read_each_other() {
    let mut net = Net::new(&["alice", "bob", "carol", "dave"]);
    let alice = net.join(0, "#Mahalle", "", "Ayşe");
    net.join(1, "mahalle", "", "Bora");
    net.join(2, "MAHALLE", "", "Cem");

    let got = net.post(0, alice, "Su dağıtımı okulda");
    let readers: Vec<usize> = got.iter().map(|(node, _)| *node).collect();
    assert_eq!(readers, vec![1, 2], "dave did not join");
    for (_, post) in &got {
        assert_eq!(post.body, "Su dağıtımı okulda");
        assert_eq!(post.nick, "Ayşe");
        assert_eq!(post.author, got[0].1.author);
        assert!(!post.outgoing);
    }
    // Dave carries the post without reading it.
    assert_eq!(net.nodes[3].counts().unwrap().1, 1);
    assert!(net.nodes[0].contacts().unwrap().is_empty());
}

#[test]
fn a_password_keeps_out_who_does_not_know_it() {
    let mut net = Net::new(&["alice", "bob", "carol"]);
    let alice = net.join(0, "aile", "kapı-23", "anne");
    net.join(1, "aile", "kapı-23", "oğul");
    net.join(2, "aile", "", "meraklı");
    let readers: Vec<usize> = net
        .post(0, alice, "akşam 7'de")
        .into_iter()
        .map(|(node, _)| node)
        .collect();
    assert_eq!(readers, vec![1]);
    assert!(net.nodes[0].channels().unwrap()[0].has_password);
}

#[test]
fn joining_later_opens_posts_still_carried() {
    let mut net = Net::new(&["alice", "bob"]);
    let alice = net.join(0, "mahalle", "", "Ayşe");
    net.post(0, alice, "erken");
    assert!(net.events[1].is_empty());
    let bob = net.join(1, "mahalle", "", "Bora");
    let posts = net.nodes[1].posts(bob).unwrap();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].body, "erken");
    assert_eq!(net.nodes[1].channels().unwrap()[0].unread, 1);
}

#[test]
fn a_muted_author_is_neither_shown_nor_passed_on() {
    let mut net = Net::new(&["alice", "bob", "carol"]);
    let alice = net.join(0, "mahalle", "", "spam");
    let bob = net.join(1, "mahalle", "", "Bora");
    let got = net.post(0, alice, "reklam");
    let author = got[0].1.author.clone();
    net.nodes[1].mute_author(bob, &author).unwrap();
    assert!(net.nodes[1].posts(bob).unwrap().is_empty());

    // Carol joins only now, so whatever reaches her came through the network.
    net.join(2, "mahalle", "", "Cem");
    let before: Vec<usize> = net.events.iter().map(Vec::len).collect();
    let (_, frame) = net.nodes[0]
        .send_post(alice, "yine reklam", NOW + 31)
        .unwrap();
    let frame_again = frame.clone();
    // Only Bob hears Alice directly: he must not pass the post on.
    let handled = net.nodes[1].handle(frame, NOW + 60).unwrap();
    assert!(handled.relay.is_empty());
    assert!(handled.events.is_empty());
    assert!(net.received_since(&before).is_empty());
    assert!(net.nodes[1].mute_author(bob, "nobody").is_err());

    // Unmuted, the posts Bob kept show again, and a dropped one is taken
    // when it is offered again.
    let muted = net.nodes[1].muted_authors(bob).unwrap();
    assert_eq!(muted.len(), 1);
    assert_eq!(muted[0].author, author);
    assert_eq!(muted[0].nick, "spam");
    net.nodes[1].unmute_author(bob, &author).unwrap();
    assert!(net.nodes[1].muted_authors(bob).unwrap().is_empty());
    assert_eq!(net.nodes[1].posts(bob).unwrap().len(), 1);
    let again = net.nodes[1].handle(frame_again, NOW + 61).unwrap();
    assert_eq!(again.events.len(), 1);
    assert_eq!(net.nodes[1].posts(bob).unwrap().len(), 2);
    assert!(net.nodes[1].unmute_author(bob, &author).is_err());
}

#[test]
fn a_shared_card_can_be_added() {
    let mut net = Net::new(&["alice", "bob"]);
    let alice = net.join(0, "mahalle", "", "Ayşe");
    net.join(1, "mahalle", "", "Bora");
    let before: Vec<usize> = net.events.iter().map(Vec::len).collect();
    let (own, frame) = net.nodes[0].share_card(alice, NOW + 30).unwrap();
    assert!(own.shared_card.is_some());
    net.flood(0, frame);
    let got = net.received_since(&before);
    let card = got[0].1.shared_card.clone().unwrap();
    assert_eq!(card.name, "alice");
    assert_eq!(card.fingerprint, net.nodes[0].profile().fingerprint);
    let contact = net.nodes[1].add_contact(&card.card, NOW + 40).unwrap().0;
    assert_eq!(contact.name, "alice");
}

#[test]
fn posting_is_limited_to_ten_a_minute() {
    let mut net = Net::new(&["alice"]);
    let alice = net.join(0, "mahalle", "", "Ayşe");
    for n in 0..10 {
        net.nodes[0]
            .send_post(alice, &format!("{n}"), NOW + n)
            .unwrap();
    }
    assert!(net.nodes[0].send_post(alice, "fazla", NOW + 30).is_err());
    assert!(net.nodes[0].send_post(alice, "sonra", NOW + 70).is_ok());
}

#[test]
fn names_nicks_and_leaving() {
    let mut net = Net::new(&["alice"]);
    assert!(net.nodes[0]
        .join_channel("iki kelime", "", "a", NOW)
        .is_err());
    assert!(net.nodes[0]
        .join_channel("mahalle", "", "   ", NOW)
        .is_err());
    let first = net.join(0, "#Mahalle", "", "Ayşe");
    assert_eq!(net.join(0, "mahalle", "", "Ayşe"), first);
    assert_eq!(net.nodes[0].channels().unwrap()[0].name, "mahalle");
    let renamed = net.nodes[0].set_channel_nick(first, "Ayşe Y.").unwrap();
    assert_eq!(renamed.nick, "Ayşe Y.");
    let (_, frame) = net.nodes[0].send_post(first, "hi", NOW).unwrap();
    assert!(matches!(frame, Frame::Bundle(ref bundle) if bundle.hop_limit == 3));
    net.nodes[0].leave_channel(first).unwrap();
    assert!(net.nodes[0].channels().unwrap().is_empty());
    assert!(net.nodes[0].posts(first).is_err());
}
