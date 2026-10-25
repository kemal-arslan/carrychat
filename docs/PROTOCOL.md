# Kervan protocol v0

**Status: draft.** Breaking changes are expected until v1. The reference implementation is [`crates/kervan-proto`](../crates/kervan-proto), and [`test-vectors.json`](test-vectors.json) pins its output so other implementations can check themselves against it.

## 1. Goals

- Deliver short private messages between two people as soon as one of them has the other's contact card, over whatever link exists: a local network, a LoRa radio, the internet, or a USB stick carried by hand.
- A normal text message fits in **one Meshtastic packet** (233 bytes).
- Relays, couriers and internet servers carry opaque bytes. They cannot read a message, and cannot tell who it is from or for unless they hold one of the two keys.
- Store and forward: a message may take days and several kinds of link to arrive.
- Delivery receipts that anyone carrying a copy can verify, so copies are dropped as soon as the message has arrived.

Not in v0: group chat, public channels, messages from people who do not have your card, files, voice, forward secrecy.

## 2. Notation

- `||` is concatenation. Integers are unsigned big-endian. `u32(x)` is a 4-byte integer.
- `H(x)` is SHA-256, `HMAC(k, x)` is HMAC-SHA256, `x[a..b]` is bytes `a` up to but not including `b`.
- Times are unix seconds as `u32`. `day(t) = floor(t / 86400)`.
- Labels such as `"kervan/v0/card"` are ASCII bytes without a terminator.

## 3. Identity

Each user holds two independent key pairs:

- an **Ed25519** signing key, which signs the contact card, and
- an **X25519** key, used for pairwise key agreement.

```
fingerprint = H("kervan/v0/fingerprint" || ed25519_pk || x25519_pk)[0..16]
```

The fingerprint is shown as lowercase RFC 4648 base32 without padding, in groups of four (`abcd efgh ijkl mnop qrst uvwx yz`). Two people compare fingerprints to verify each other.

### 3.1 Contact card

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 1 | version, `0x00` |
| 1 | 32 | Ed25519 public key |
| 33 | 32 | X25519 public key |
| 65 | 1 | name length `N`, 0 to 32 |
| 66 | N | name, UTF-8 |
| 66+N | 64 | Ed25519 signature over `"kervan/v0/card" \|\| bytes[0..66+N]` |

The text form is `kervan:` followed by the card in base64url without padding, short enough for a QR code. Receivers verify the signature (strict Ed25519 verification) and reject unknown versions.

v0 has no contact discovery over the network. One person gives their card to the other, in person or over a channel they already trust. The first message then carries the sender's card the other way (§5.5). Fingerprints are compared either way.

## 4. Pair keys

For a pair of contacts:

```
shared   = X25519(own_secret, peer_public)       abort if all-zero (non-contributory)
(P1, P2) = the two X25519 public keys, sorted bytewise ascending
prk      = HKDF-Extract(salt = "kervan/v0/pair", ikm = shared)
K_enc    = HKDF-Expand(prk, "enc" || P1 || P2, 32)
K_tag    = HKDF-Expand(prk, "tag" || P1 || P2, 32)
K_ack    = HKDF-Expand(prk, "ack" || P1 || P2, 32)
```

Both contacts derive the same three keys. Nobody else can.

### 4.1 Recipient tag

```
tag(recipient_pk, d) = HMAC(K_tag, recipient_pk || u32(d))[0..8]
```

`d` is the day the message was created. The tag tells the recipient which contact a bundle is from, so it knows which key to try, and tells nobody else anything. It changes every day and differs per direction, so an observer cannot link a pair's traffic across days, or the two directions of one conversation.

A receiver precomputes the tags of every contact for the days `today − 7` to `today + 1`: seven days is the longest a bundle lives, and one day absorbs clock drift.

## 5. Bundle

A bundle is one sealed message. It is the unit that is stored, carried and forwarded.

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 1 | version, `0x00` |
| 1 | 1 | hop limit |
| 2 | 4 | expires at, `u32` |
| 6 | 8 | bundle id |
| 14 | 8 | recipient tag |
| 22 | 24 | nonce |
| 46 | n + 16 | ciphertext, then the 16-byte Poly1305 tag |

A bundle is at most 4096 bytes.

### 5.1 Sealing

```
nonce      = 24 random bytes, never reused under the same K_enc
ack_secret = HMAC(K_ack, nonce)[0..16]
bundle_id  = H("kervan/v0/ack" || ack_secret)[0..8]
tag        = tag(recipient_pk, day(created_at))
aad        = the 46-byte header without the hop limit byte: bytes[0..1] || bytes[2..46]
ciphertext = XChaCha20-Poly1305-Encrypt(K_enc, nonce, plaintext, aad)
```

The hop limit is left out of the associated data so relays can decrement it. Every other header field is authenticated.

`created_at < expires_at ≤ created_at + 604800` (7 days). The default lifetime is 3 days and the default hop limit is 8.

### 5.2 Plaintext

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 1 | kind |
| 1 | 4 | created at, `u32` |
| 5 | rest | body |

| Kind | Body |
| --- | --- |
| `0x01` text | UTF-8 text |
| `0x02` introduction | Only inside introductions (§5.5) |
| `0x03` read | Up to 64 bundle ids (8 bytes each) of the recipient's bundles that the sender has read. Optional: a user may turn read receipts off. |
| `0x04` card | The sender's contact card (§3.1), after a name change. The receiver accepts it only if both keys equal those of the card it has. |

All other values are reserved. A receiver that opens a bundle of an unknown kind ignores the body but still emits a tombstone, so the bundle stops circulating.

### 5.3 Opening

1. Look the tag up among the precomputed tags to find the contact and the day.
2. Decrypt with that contact's `K_enc`. If decryption fails, the bundle is not ours or is corrupt.
3. Check that `tag == tag(own_pk, day(created_at))`, that the lifetime rule in §5.1 holds, and that `bundle_id` is the one derived from the nonce. Reject the bundle if any check fails.
4. Emit a tombstone (§6).

### 5.4 Size budget

Overhead is 46 (header) + 5 (plaintext header) + 16 (Poly1305 tag) = **67 bytes**. In one 233-byte Meshtastic packet with its 1-byte frame type (§7), the body can be up to **165 bytes**: 165 ASCII characters, fewer for text with letters such as ç, ğ, ı, ö, ş and ü, which take two bytes each in UTF-8.

### 5.5 Introductions

An introduction is a first message to someone who does not have the sender's card yet. Only the sender needs the recipient's card. It is a bundle with the header of §5, except that its recipient tag is 8 random bytes, and its ciphertext field starts with a one-time public key:

```
e          = a fresh X25519 key pair, used once
shared     = X25519(e_secret, recipient_pk)                abort if all-zero
prk        = HKDF-Extract(salt = "kervan/v0/intro", ikm = shared)
K_enc      = HKDF-Expand(prk, "enc" || e_pk || recipient_pk, 32)
K_ack      = HKDF-Expand(prk, "ack" || e_pk || recipient_pk, 32)
ack_secret = HMAC(K_ack, nonce)[0..16]
bundle_id  = H("kervan/v0/ack" || ack_secret)[0..8]
signature  = Ed25519-Sign(sender's signing key, "kervan/v0/intro-signature"
               || e_pk || recipient_pk || nonce || u32(created_at) || text)
plaintext  = 0x02 || u32(created_at) || u8(len(card)) || card || signature || text
ciphertext = e_pk || XChaCha20-Poly1305-Encrypt(K_enc, nonce, plaintext, aad)
```

`card` is the sender's contact card (§3.1) and `aad` is the one of §5.1.

**Receiving.** A node tries every bundle whose tag matches none of its contacts as an introduction: it computes `X25519(own_secret, e_pk)`, derives the keys and decrypts. If that works, it verifies the card's own signature, then `signature` with the card's Ed25519 key, then the lifetime rule and the bundle id. The card proves whose it is; the signature proves the sender holds that card's key, and ties the message to this recipient and this bundle, so it cannot be replayed to someone else. A node that opens an introduction emits a tombstone as for any bundle (§6).

**Requests.** If the sender is not a contact yet, it becomes a *request*: the app shows the message with the sender's name and fingerprint, and the user accepts or declines. Accepting makes the sender a contact. Declining deletes the request and remembers the sender's key: later introductions from it are dropped, but still tombstoned so they stop circulating. Pair bundles (§5.1) that arrive from a requester before the request is answered are kept with it.

**Sending.** A node sends introductions to a contact until it knows the contact has its card: a tombstone for one of its bundles to that contact came back, or a bundle from that contact arrived. From then on it sends pair bundles.

A short introduction takes two Meshtastic packets: its overhead is 164 bytes plus the card, which is 130 bytes plus the sender's name.

## 6. Tombstone

A tombstone proves delivery. After opening a bundle, the recipient publishes `(bundle_id, ack_secret)`.

Anyone holding the bundle checks `H("kervan/v0/ack" || ack_secret)[0..8] == bundle_id`, then deletes the bundle and stops forwarding it. The sender runs the same check and marks the message delivered. Only the two contacts know `K_ack`, so only they can produce `ack_secret` for a real bundle. Forging one means finding a preimage of a 64-bit truncated hash.

- A node forwards a tombstone only for a bundle id it holds or has seen. Anyone can build a self-consistent tombstone for an id they made up, so tombstones for unknown ids are dropped.
- A node remembers tombstoned ids until the bundle would have expired, so a late copy is not accepted again.

A tombstone reveals that some bundle was delivered at about this time and place. A recipient that does not want that can delay the tombstone or skip it, and the bundle then circulates until it expires.

## 7. Frames

Transports move frames. A transport whose MTU fits a whole bundle (local network, internet, file) never fragments.

| Type | Layout |
| --- | --- |
| `0x01` bundle | `type \|\| bundle` |
| `0x02` fragment | `type \|\| bundle_id (8) \|\| index (1) \|\| count (1) \|\| chunk` |
| `0x03` tombstone | `type \|\| hop_limit (1) \|\| bundle_id (8) \|\| ack_secret (16)`, 26 bytes |

If `1 + len(bundle) > mtu`, the bundle bytes are split into chunks of `mtu − 11` bytes, from 2 to 32 of them. Receivers reassemble by bundle id with a timeout (10 minutes by default) and a cap on the number of incomplete bundles, then check that the reassembled bundle carries the same id as its fragments. Relays reassemble before forwarding, because they need the hop limit and the expiry.

Frame types `0x10` to `0x1f` are reserved for transport-local control traffic (§9.1).

## 8. Forwarding

Each node keeps one store: its own outbox, plus the bundles it carries for others.

- Duplicates are dropped by bundle id.
- Expired bundles are deleted. Bundles that claim to expire more than 8 days from now are refused.
- Forwarding decrements the hop limit. A bundle whose hop limit is 0 is kept but not forwarded.
- Bundles carried for others have a quota. When it is full, the bundle closest to expiry is evicted first.
- A bundle is not sent back over the connection it arrived on. A tombstone is, because the node at the other end may be the sender waiting for it.
- A node that adds a contact opens the bundles it already carries that turn out to be from that contact.
- **Split horizon on LoRa.** A bundle that arrived over LoRa is not sent over LoRa again by the same node, because the Meshtastic mesh already floods it. It may be bridged to other links. Bundles that arrived over any other link may be sent over LoRa within the airtime budget, with the node's own bundles and tombstones first.

## 9. Transports (draft)

These mappings are fixed when each transport is built (see the roadmap in the README).

### 9.1 Local network

Implemented in `crates/kervan-node/src/lan.rs`.

**Discovery.** Every 3 seconds a node sends a 16-byte beacon to the multicast group `239.255.74.74`, port `47474`:

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 6 | `"KRVN" \|\| 0x00 \|\| 0x01` (magic, version, beacon) |
| 6 | 8 | session id, random per run |
| 14 | 2 | TCP port, `u16` |

The beacon carries no identity. On hearing a beacon from a node it is not connected to, the node with the smaller session id dials. The other node dials too after three beacons without a connection, in case its own beacons do not get through.

**Sync.** Over TCP, every frame is prefixed with its `u16` length. Besides the frames of §7, three control frames exist:

| Type | Layout |
| --- | --- |
| `0x10` HAVE | `type \|\| count (u16) \|\| bundle_id × count` |
| `0x11` WANT | `type \|\| count (u16) \|\| bundle_id × count` |
| `0x12` HELLO | `type \|\| session id (8)` |

Both sides send HELLO first. A second connection between the same two nodes is closed. Then each side sends HAVE with the ids of the bundles it can pass on, and every tombstone it knows. The other side answers with WANT for the ids it has never seen, and receives those bundles. After that, new bundles and tombstones are pushed as they appear. An empty HAVE every 15 seconds keeps the connection alive, and a connection silent for 45 seconds is closed.

### 9.2 LoRa via Meshtastic

- The app talks to a Meshtastic node over USB serial (Bluetooth and TCP also work) with the Meshtastic client API.
- Each frame is the payload of one Meshtastic `Data` packet sent to broadcast, MTU 233 bytes (`DATA_PAYLOAD_LEN`). During development it uses portnum `PRIVATE_APP` (256). A dedicated portnum will be requested once the format settles.
- The Meshtastic mesh rebroadcasts packets itself, with a hop limit of up to 7, so Kervan nodes do not re-flood LoRa traffic (§8).
- In the `EU_868` region Meshtastic uses 869.4–869.65 MHz, where the legal duty cycle limit is 10%, and every node in range shares that airtime. Kervan keeps its own airtime budget well below the limit and sends only bundles and tombstones over LoRa, never bulk data.

### 9.3 Internet via Nostr

Implemented in `crates/kervan-node/src/internet.rs`.

- **Events.** Every bundle and tombstone travels as one Nostr event (NIP-01) of kind `9378`, a provisional number not registered with Nostr. The content is the frame (§7) in base64url without padding. A bundle event has the tag `["x", hex(tag)]`, a tombstone event `["b", hex(bundle_id)]`.
- **Tags.** A pair bundle is posted under its recipient tag. An introduction, whose own tag is random, is posted under the recipient's mailbox tag, which anyone holding the recipient's card can compute:

  ```
  mailbox_tag(pk, d) = HMAC(H("kervan/v0/mailbox" || pk), u32(d))[0..8]
  ```

  `d` is the day the introduction was created.
- **Keys.** Each event is signed (BIP-340) with a one-time key `HMAC(node_secret, content)`, where `node_secret` is a random secret the node keeps. Keys cannot be linked to the user or to each other, and the same node posting the same frame again produces the same event, which relays drop as a duplicate.
- **Time.** A bundle event's `created_at` is `expires_at − 604800`, never later than the bundle's creation. A tombstone event's is the time it is posted.
- **Subscription.** A node asks every relay for `{"kinds": [9378], "#x": [...], "since": now − 691200}` with the pair tags of all its contacts and its own mailbox tags over the tag window (§4.1), and for `{"kinds": [9378], "#b": [...]}` with the ids of its own bundles still waiting for a receipt. It renews the subscription when its contacts change, when it sends, and every 10 minutes.
- **Connections.** A node pings every relay every 20 seconds and drops a connection that has been silent for 40, then connects again. Right after sending, it also pings every relay and drops a connection that does not answer within 5 seconds. On connecting again it posts its waiting bundles once more. A computer that moves to another network can otherwise keep a dead connection for minutes without noticing, and lose what it sends into it.
- **What a node posts.** Its own bundles when it sends them and again on every connection, the tombstones of bundles it receives, and, as a gateway, every bundle it starts carrying that came over another link.
- **What a node takes.** Only bundles for itself and tombstones. A bundle from a relay that does not open is dropped, not carried, so a relay cannot fill a node's store.
- **Relays** see tags, sizes, timing and the node's IP address. They also see which tags one connection asks for together, which links the tags of one node's contacts within a day. Reaching relays over Tor is future work.

The default relays are `wss://relay.damus.io`, `wss://nos.lol` and `wss://relay.primal.net`. Fetching for nodes without internet access (downlink through a gateway) is still open (§11).

### 9.4 File

Implemented in `crates/kervan-node/src/file.rs`, with the extension `.kervan`. A file is `"KRVN" || 0x00 || frames`, each frame prefixed with its `u16` length. Exporting writes the store's bundles and tombstones. Importing feeds every frame into the store, exactly as if it had arrived over any other link.

## 10. Security

**v0 provides:**

- confidentiality and integrity of message content end to end, whatever carried it,
- unlinkability for relays: without a pair key they cannot tell sender or recipient, or link a pair's traffic across days or directions,
- delivery receipts that cannot be forged,
- a closed inbox: only someone holding your card can write to you, and their first message is a request you accept or decline.

**v0 does not provide:**

- **Hidden introductions.** An introduction cannot be linked to its sender or recipient, but its size sets it apart from most pair bundles.
- **Forward secrecy.** Pair keys come from long-term keys, and introductions are sealed to the recipient's long-term key. Whoever obtains a device's X25519 secret can decrypt all past and future traffic with every contact of that device that they have recorded, and can impersonate any of those contacts to it. A ratchet over live links is planned.
- **Radio metadata privacy.** A LoRa transmission reveals the transmitting Meshtastic node and, to direction-finding equipment, its location. Bundle sizes and timing are visible, because v0 does not pad.
- **Replay protection on the network.** A recorded bundle can be injected again until it expires. Receivers drop duplicates, but replays still spend airtime.
- **Fragment integrity at relays.** Relays cannot authenticate fragments, so a forged fragment spoils reassembly of that bundle until the timeout.
- **Reliable time.** Devices without internet access may have wrong clocks. The tag window absorbs a day of drift, and expiry checks use local time.

## 11. Open questions

1. Meshtastic channel: the default public channel (more relays, more contention for airtime) or a dedicated one.
2. Downlink for nodes without internet access through a gateway, without telling the gateway who they are.
3. Padding for LoRa frames: a smaller size leak against more airtime.
4. Exchanging contacts at a distance.
5. Group messages.
6. A Bluetooth link between nearby devices.
7. HF radio. Amateur bands forbid encryption that hides a message's meaning, so HF would need a separate signed but unencrypted message kind, limited to messages the user explicitly marks as public.

## 12. Constants

| Name | Value |
| --- | --- |
| Protocol version | `0` |
| Longest name | 32 bytes |
| Longest bundle | 4096 bytes |
| Bundle overhead | 67 bytes |
| Introduction overhead | 164 bytes plus the card |
| Default lifetime | 259200 s (3 days) |
| Longest lifetime | 604800 s (7 days) |
| Default hop limit | 8 |
| Tag window | `today − 7` to `today + 1` |
| Meshtastic MTU | 233 bytes |
| Most fragments per bundle | 32 |
| Reassembly timeout | 600 s |
