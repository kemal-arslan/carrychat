//! Groups between engines (§5.7), with every frame flooded to every node.

use std::collections::VecDeque;

use carrychat_node::engine::Engine;
use carrychat_node::store::Store;
use carrychat_node::{DataKey, Event, GroupView};
use carrychat_proto::frame::Frame;
use tempfile::TempDir;

const NOW: u32 = 1_790_000_000;

/// Nodes that hear every frame any of them sends, like one local network.
struct Mesh {
    nodes: Vec<Engine>,
    /// What each node told its app.
    events: Vec<Vec<Event>>,
}

const ALICE: usize = 0;
const BOB: usize = 1;
const CAROL: usize = 2;
const DAVE: usize = 3;

impl Mesh {
    /// Alice knows everyone; nobody else knows anyone but Alice.
    fn new(names: &[&str]) -> Self {
        let nodes: Vec<Engine> = names
            .iter()
            .map(|name| Engine::create(Store::open_in_memory().unwrap(), name).unwrap())
            .collect();
        Self::with(nodes)
    }

    fn with(nodes: Vec<Engine>) -> Self {
        let mut mesh = Self {
            events: vec![Vec::new(); nodes.len()],
            nodes,
        };
        for other in 1..mesh.nodes.len() {
            mesh.befriend(ALICE, other);
        }
        mesh
    }

    /// Two nodes add each other and exchange a first message, so each has
    /// the other's card.
    fn befriend(&mut self, a: usize, b: usize) {
        let card_b = self.nodes[b].profile().card;
        let card_a = self.nodes[a].profile().card;
        let b_at_a = self.nodes[a].add_contact(&card_b, NOW).unwrap().0.id;
        self.nodes[b].add_contact(&card_a, NOW).unwrap();
        let (_, frame) = self.nodes[a].send_text(b_at_a, "selam", NOW).unwrap();
        self.flood(a, vec![frame]);
    }

    /// Passes frames from `from` to every other node until nothing new moves.
    fn flood(&mut self, from: usize, frames: Vec<Frame>) {
        let mut queue: VecDeque<(usize, Frame)> = frames.into_iter().map(|f| (from, f)).collect();
        self.collect_own(from, &mut queue);
        while let Some((sender, frame)) = queue.pop_front() {
            for node in 0..self.nodes.len() {
                if node != sender {
                    self.deliver(node, frame.clone(), &mut queue);
                }
            }
        }
    }

    /// Hands frames to one node only, and keeps what it sends back.
    fn send_only(&mut self, to: usize, frames: Vec<Frame>) -> Vec<(usize, Frame)> {
        let mut queue = VecDeque::new();
        for frame in frames {
            self.deliver(to, frame, &mut queue);
        }
        queue.into()
    }

    fn deliver(&mut self, node: usize, frame: Frame, queue: &mut VecDeque<(usize, Frame)>) {
        let handled = self.nodes[node].handle(frame, NOW + 60).unwrap();
        self.events[node].extend(handled.events);
        queue.extend(handled.relay.into_iter().map(|f| (node, f)));
        self.collect_own(node, queue);
    }

    fn collect_own(&mut self, node: usize, queue: &mut VecDeque<(usize, Frame)>) {
        let own = self.nodes[node].take_own();
        queue.extend(own.into_iter().map(|(f, _)| (node, f)));
    }

    fn group(&self, node: usize) -> GroupView {
        let groups = self.nodes[node].groups().unwrap();
        assert_eq!(groups.len(), 1, "node {node} has {} groups", groups.len());
        groups.into_iter().next().unwrap()
    }

    fn contact_id(&self, node: usize, name: &str) -> i64 {
        self.nodes[node]
            .contacts()
            .unwrap()
            .into_iter()
            .find(|contact| contact.name == name)
            .unwrap_or_else(|| panic!("node {node} has no contact {name}"))
            .id
    }

    /// Alice creates a group of everyone.
    fn create_group(&mut self) -> i64 {
        let ids: Vec<i64> = (1..self.nodes.len())
            .map(|node| self.contact_id(ALICE, &self.nodes[node].profile().name))
            .collect();
        let group = self.nodes[ALICE]
            .create_group("Mahalle", &ids, NOW + 10)
            .unwrap();
        self.flood(ALICE, Vec::new());
        group.id
    }

    /// One node writes to the group. Returns who read it.
    fn say(&mut self, node: usize, text: &str) -> Vec<usize> {
        let group = self.group(node).id;
        let before: Vec<usize> = self.events.iter().map(Vec::len).collect();
        let (_, frame) = self.nodes[node]
            .send_group_text(group, text, NOW + 30)
            .unwrap();
        self.flood(node, vec![frame]);
        self.readers_since(&before, text)
    }

    fn readers_since(&self, before: &[usize], text: &str) -> Vec<usize> {
        (0..self.nodes.len())
            .filter(|&node| {
                self.events[node][before[node]..].iter().any(|event| {
                    matches!(event, Event::GroupMessageReceived { message } if message.body == text)
                })
            })
            .collect()
    }
}

#[test]
fn every_member_reads_one_bundle_even_without_knowing_the_others() {
    let mut mesh = Mesh::new(&["alice", "bob", "carol"]);
    let tags_before = mesh.nodes[BOB].subscription(NOW + 60).unwrap().0.len();
    mesh.create_group();
    // Relays are asked for the tags of Alice's and Carol's chains too, 9 days of each,
    // and for Carol's pair tags now that she is a group peer.
    let tags_after = mesh.nodes[BOB].subscription(NOW + 60).unwrap().0.len();
    assert_eq!(tags_after, tags_before + 3 * 9);
    for node in [ALICE, BOB, CAROL] {
        let group = mesh.group(node);
        assert_eq!(group.name, "Mahalle");
        assert_eq!(group.members.len(), 3);
        assert!(group.member);
        assert_eq!(group.admin, node == ALICE);
    }
    // Bob and Carol are not contacts: each is only a group peer to the other.
    let at_carol = mesh.group(CAROL);
    let bob = at_carol.members.iter().find(|m| m.name == "bob").unwrap();
    assert_eq!(bob.contact_id, None);
    assert_eq!(mesh.nodes[CAROL].contacts().unwrap().len(), 1);

    assert_eq!(mesh.say(BOB, "Su geldi mi?"), vec![ALICE, CAROL]);
    assert_eq!(mesh.say(CAROL, "Bizde yok"), vec![ALICE, BOB]);
    assert_eq!(mesh.say(ALICE, "Okulda dağıtım var"), vec![BOB, CAROL]);

    let at_bob = mesh.nodes[BOB].group_messages(mesh.group(BOB).id).unwrap();
    let lines: Vec<(Option<&str>, &str)> = at_bob
        .iter()
        .map(|m| (m.sender.as_deref(), m.body.as_str()))
        .collect();
    assert_eq!(
        lines,
        vec![
            (None, "Su geldi mi?"),
            (Some("carol"), "Bizde yok"),
            (Some("alice"), "Okulda dağıtım var"),
        ]
    );
    assert_eq!(mesh.group(BOB).unread, 2);
    let group_id = mesh.group(BOB).id;
    mesh.nodes[BOB].mark_group_read(group_id, NOW + 40).unwrap();
    assert_eq!(mesh.group(BOB).unread, 0);
}

#[test]
fn a_removed_member_reads_nothing_after() {
    let mut mesh = Mesh::new(&["alice", "bob", "carol"]);
    let group = mesh.create_group();
    assert_eq!(mesh.say(BOB, "önce"), vec![ALICE, CAROL]);

    let carol = mesh.group(ALICE).members[2].fingerprint.clone();
    let view = mesh.nodes[ALICE]
        .remove_group_member(group, &carol, NOW + 20)
        .unwrap();
    assert_eq!(view.members.len(), 2);
    mesh.flood(ALICE, Vec::new());
    assert!(!mesh.group(CAROL).member);
    assert_eq!(mesh.group(BOB).members.len(), 2);

    assert_eq!(mesh.say(BOB, "sonra"), vec![ALICE]);
    assert_eq!(mesh.say(ALICE, "yine"), vec![BOB]);
    let group_id = mesh.group(CAROL).id;
    assert!(mesh.nodes[CAROL]
        .send_group_text(group_id, "hâlâ burada mıyım?", NOW + 50)
        .is_err());
}

#[test]
fn a_member_leaves_and_the_admin_takes_them_off() {
    let mut mesh = Mesh::new(&["alice", "bob", "carol"]);
    mesh.create_group();
    let group_id = mesh.group(BOB).id;
    let view = mesh.nodes[BOB].leave_group(group_id, NOW + 20).unwrap();
    assert!(!view.member);
    mesh.flood(BOB, Vec::new());
    assert_eq!(mesh.group(ALICE).members.len(), 2);
    assert_eq!(mesh.group(CAROL).members.len(), 2);
    assert_eq!(mesh.say(CAROL, "gitti"), vec![ALICE]);
    let group_id = mesh.group(ALICE).id;
    assert!(mesh.nodes[ALICE].leave_group(group_id, NOW + 60).is_err());
}

#[test]
fn a_new_member_reads_only_what_comes_after() {
    let mut mesh = Mesh::new(&["alice", "bob", "carol", "dave"]);
    let ids = [
        mesh.contact_id(ALICE, "bob"),
        mesh.contact_id(ALICE, "carol"),
    ];
    let group = mesh.nodes[ALICE]
        .create_group("Mahalle", &ids, NOW + 10)
        .unwrap()
        .id;
    mesh.flood(ALICE, Vec::new());
    assert!(mesh.nodes[DAVE].groups().unwrap().is_empty());
    assert_eq!(mesh.say(BOB, "dave yokken"), vec![ALICE, CAROL]);

    let dave = mesh.contact_id(ALICE, "dave");
    mesh.nodes[ALICE]
        .add_group_member(group, dave, NOW + 20)
        .unwrap();
    mesh.flood(ALICE, Vec::new());
    assert_eq!(mesh.group(DAVE).members.len(), 4);
    assert_eq!(mesh.say(BOB, "dave geldi"), vec![ALICE, CAROL, DAVE]);
    assert_eq!(mesh.say(DAVE, "merhaba"), vec![ALICE, BOB, CAROL]);
    let at_dave = mesh.nodes[DAVE]
        .group_messages(mesh.group(DAVE).id)
        .unwrap();
    assert!(at_dave.iter().all(|m| m.body != "dave yokken"));
}

/// The frame as its sender sent it, before the relay used a hop.
fn relayed(frame: &Frame) -> Frame {
    match frame {
        Frame::Bundle(bundle) => Frame::Bundle(carrychat_proto::bundle::Bundle {
            hop_limit: bundle.hop_limit + 1,
            ..bundle.clone()
        }),
        other => other.clone(),
    }
}

#[test]
fn keys_that_overtake_the_state_are_used_once_it_arrives() {
    let mut mesh = Mesh::new(&["alice", "bob", "carol"]);
    let ids = [
        mesh.contact_id(ALICE, "bob"),
        mesh.contact_id(ALICE, "carol"),
    ];
    mesh.nodes[ALICE]
        .create_group("Mahalle", &ids, NOW + 10)
        .unwrap();
    let from_alice: Vec<Frame> = mesh.nodes[ALICE]
        .take_own()
        .into_iter()
        .map(|(f, _)| f)
        .collect();

    // Carol hears Alice first and hands Bob her chain before Bob knows her.
    let from_carol = mesh.send_only(CAROL, from_alice.clone());
    let carol_to_bob: Vec<Frame> = from_carol
        .into_iter()
        .map(|(_, f)| f)
        .filter(|frame| !from_alice.contains(&relayed(frame)))
        .collect();
    mesh.send_only(BOB, carol_to_bob.clone());
    assert!(mesh.nodes[BOB].groups().unwrap().is_empty());

    // Then Alice's state reaches Bob: he meets Carol and opens her chain.
    mesh.flood(ALICE, from_alice);
    // Carol's chain for Alice comes along with the next sync.
    mesh.flood(CAROL, carol_to_bob);
    assert_eq!(mesh.say(CAROL, "duydun mu?"), vec![ALICE, BOB]);
}

#[test]
fn only_the_admin_changes_the_members() {
    let mut mesh = Mesh::new(&["alice", "bob", "carol"]);
    let ids = [mesh.contact_id(ALICE, "bob")];
    mesh.nodes[ALICE]
        .create_group("İkimiz", &ids, NOW + 10)
        .unwrap();
    mesh.flood(ALICE, Vec::new());
    let group = mesh.group(BOB).id;
    let alice = mesh.contact_id(BOB, "alice");
    assert!(mesh.nodes[BOB]
        .add_group_member(group, alice, NOW + 20)
        .is_err());
    assert!(mesh.nodes[CAROL].groups().unwrap().is_empty());
}

#[test]
fn a_group_peer_who_writes_directly_becomes_a_request() {
    let mut mesh = Mesh::new(&["alice", "bob", "carol"]);
    mesh.create_group();
    // Carol writes to Bob with the card the group gave her.
    let member = mesh
        .group(CAROL)
        .members
        .into_iter()
        .find(|member| member.name == "bob")
        .unwrap();
    assert_eq!(member.card, mesh.nodes[BOB].profile().card);
    let bob = mesh.nodes[CAROL]
        .add_contact(&member.card, NOW + 20)
        .unwrap()
        .0;
    assert!(!bob.request);
    let listed = mesh.group(CAROL).members;
    let listed = listed.iter().find(|m| m.name == "bob").unwrap();
    assert_eq!(listed.contact_id, Some(bob.id));
    let (_, frame) = mesh.nodes[CAROL]
        .send_text(bob.id, "Ben Carol, gruptan", NOW + 20)
        .unwrap();
    mesh.flood(CAROL, vec![frame]);
    let requests: Vec<_> = mesh.nodes[BOB]
        .contacts()
        .unwrap()
        .into_iter()
        .filter(|contact| contact.request)
        .collect();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].name, "carol");
    assert_eq!(
        requests[0].last_message.as_deref(),
        Some("Ben Carol, gruptan")
    );
}

#[test]
fn groups_survive_a_restart() {
    let dir = TempDir::new().unwrap();
    let paths: Vec<_> = ["alice", "bob", "carol"]
        .iter()
        .map(|name| dir.path().join(format!("{name}.db")))
        .collect();
    let nodes = ["alice", "bob", "carol"]
        .iter()
        .zip(&paths)
        .map(|(name, path)| Engine::create(Store::open(path, &key()).unwrap(), name).unwrap())
        .collect();
    let mut mesh = Mesh::with(nodes);
    mesh.create_group();
    assert_eq!(mesh.say(BOB, "bir"), vec![ALICE, CAROL]);
    drop(mesh);

    let nodes = paths
        .iter()
        .map(|path| {
            Engine::load(Store::open(path, &key()).unwrap())
                .unwrap()
                .unwrap()
        })
        .collect();
    let mut mesh = Mesh {
        events: vec![Vec::new(); 3],
        nodes,
    };
    assert_eq!(mesh.say(CAROL, "iki"), vec![ALICE, BOB]);
    assert_eq!(mesh.say(BOB, "üç"), vec![ALICE, CAROL]);
}

/// The key these tests' databases are encrypted with, the same on every
/// opening.
fn key() -> DataKey {
    DataKey::from_bytes(&[7; 32]).unwrap()
}
