# CarryChat protocol v0

**Status: draft.** Breaking changes are expected until v1. The reference implementation is [`crates/carrychat-proto`](../crates/carrychat-proto), and [`test-vectors.json`](test-vectors.json) pins its output so other implementations can check themselves against it.

## 1. Goals

- Deliver short private messages between two people as soon as one of them has the other's contact card, over whatever link exists: a local network, a LoRa radio, the internet, or a USB stick carried by hand.
- A normal text message fits in **one Meshtastic packet** (233 bytes).
- Relays, couriers and internet servers carry opaque bytes. They cannot read a message, and cannot tell who it is from or for unless they hold one of the two keys.
- Store and forward: a message may take days and several kinds of link to arrive.
- Delivery receipts that anyone carrying a copy can verify, so copies are dropped as soon as the message has arrived.

Not yet: video, and files over LoRa.

## 2. Notation

- `||` is concatenation. Integers are unsigned big-endian. `u32(x)` is a 4-byte integer.
- `H(x)` is SHA-256, `HMAC(k, x)` is HMAC-SHA256, `x[a..b]` is bytes `a` up to but not including `b`.
- Times are unix seconds as `u32`. `day(t) = floor(t / 86400)`.
- Labels such as `"carrychat/v0/card"` are ASCII bytes without a terminator.

## 3. Identity

Each user holds two independent key pairs:

- an **Ed25519** signing key, which signs the contact card, and
- an **X25519** key, used for pairwise key agreement.

```
fingerprint = H("carrychat/v0/fingerprint" || ed25519_pk || x25519_pk)[0..16]
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
| 66+N | 64 | Ed25519 signature over `"carrychat/v0/card" \|\| bytes[0..66+N]` |

The text form is `carrychat:` followed by the card in base64url without padding, short enough for a QR code. Receivers verify the signature (strict Ed25519 verification) and reject unknown versions.

v0 has no contact discovery over the network. One person gives their card to the other, in person or over a channel they already trust. The first message then carries the sender's card the other way (§5.5). Fingerprints are compared either way.

### 3.2 Cards over sound

Two devices side by side can exchange a card as sound, with nothing else in common: one plays its card through the speaker, the other listens through the microphone. Implemented in `crates/carrychat-proto/src/sound.rs`.

- **Tones.** 96 tones, `1875 + 46.875·k` Hz for `k` from 0 to 95, in six channels of 16: channel `c` uses tones `16c` to `16c + 15`. Tones 46.875 Hz apart are orthogonal over a window of 1/46.875 s, and the range suits phone speakers and microphones.
- **Symbols.** A symbol lasts 100 ms and fades in and out over 5 ms. A data symbol sounds one tone per channel, a nibble each: channel 0 carries the high nibble of the first byte, channel 1 its low nibble, and so on, three bytes a symbol. Every tone in a symbol is equally loud.
- **Round.** 100 ms of silence; a start marker of four symbols that alternate tones `{0, 8}` and `{4, 12}` in every channel, two at once, which no data symbol has; a header symbol `0xCC || len || ~len`; the `len` bytes, zero-padded to whole symbols; 400 ms of silence. One round carries up to 255 bytes. A card of 143 bytes takes about 5.5 seconds.
- **Sending.** The sender plays rounds over and over until it is stopped. Nothing is added to check the bytes: a card verifies its own signature (§3.1), and the listener keeps listening when one does not.
- **Listening.** The listener measures every tone over windows of 1/46.875 s every quarter window, with Goertzel's algorithm at whatever rate the microphone gives. A marker starts wherever, for four symbols in a row, the marker's tones hold at least half of each channel's energy on average; a run of such starts is timed from its middle. Each symbol is read from the windows that lie wholly within it after its first 30 ms, where the room still echoes the symbol before, and each channel's loudest tone gives its nibble. A header that does not read right drops the round.
- **Apps.** Under **My card**, *Play by sound* plays the card; under **Add contact**, *Listen* fills in the card once it is heard, and adding it is as for a pasted card. The page turns off the browser's echo cancellation, noise suppression and gain control, which would take the tones for noise.

### 3.3 Cards over NFC

Two Android phones held back to back pass a card over NFC: the one giving its card answers as a contactless smart card would, and the other reads it. Implemented in the Android app (`CardService.kt`, `MainActivity.kt`), with the card verified by the node.

- **Giving.** While the card sheet is open, the phone offers its card through host card emulation, for the AID `F0 43 41 52 52 59 43 48` (`F0`, which marks a proprietary AID, then "CARRYCH"), in the category "other". Android routes it to CarryChat only while the screen is unlocked; with the sheet closed, the phone answers `6A 82`.
- **Reading.** While the add-contact sheet is open, the phone reads NFC-A and NFC-B tags in reader mode and sends `SELECT` by name for the AID: `00 A4 04 00 08 || AID || 00`. The reply is the card's bytes (§3.1) followed by `90 00`; a card is at most 162 bytes, so it fits one reply. The node verifies its signature before it is shown, and adding it is as for a pasted card.

## 4. Pair keys

For a pair of contacts:

```
shared   = X25519(own_secret, peer_public)       abort if all-zero (non-contributory)
(P1, P2) = the two X25519 public keys, sorted bytewise ascending
prk      = HKDF-Extract(salt = "carrychat/v0/pair", ikm = shared)
K_enc    = HKDF-Expand(prk, "enc" || P1 || P2, 32)
K_tag    = HKDF-Expand(prk, "tag" || P1 || P2, 32)
K_ack    = HKDF-Expand(prk, "ack" || P1 || P2, 32)
K_root   = HKDF-Expand(prk, "root" || P1 || P2, 32)
K_hdr    = HKDF-Expand(prk, "hdr" || P1 || P2, 32)
```

Both contacts derive the same keys. Nobody else can. `K_root` and `K_hdr` are used by ratchet sessions (§5.6).

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
bundle_id  = H("carrychat/v0/ack" || ack_secret)[0..8]
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
| `0x05` session | Empty. Starts a ratchet session (§5.6) and is not shown. |
| `0x06` group | A group state (§5.7), from the group's admin. |
| `0x07` sender key | A member's sender chain for a group (§5.7). |
| `0x08` leave | A group id (16 bytes): the sender leaves that group (§5.7). |
| `0x09` file | A file offer (§5.9): what a file is, and the key to its chunks. Also in groups and channels. |

All other values are reserved. A receiver that opens a bundle of an unknown kind ignores the body but still emits a tombstone, so the bundle stops circulating.

### 5.3 Opening

1. Look the tag up among the precomputed tags to find the contact and the day.
2. Decrypt with that contact's `K_enc`. If decryption fails, the bundle is not ours or is corrupt.
3. Check that `tag == tag(own_pk, day(created_at))`, that the lifetime rule in §5.1 holds, and that `bundle_id` is the one derived from the nonce. Reject the bundle if any check fails.
4. Emit a tombstone (§6).

### 5.4 Size budget

Overhead is 46 (header) + 5 (plaintext header) + 16 (Poly1305 tag) = **67 bytes**. In one 233-byte Meshtastic packet with its 1-byte frame type (§7), the body can be up to **165 bytes**: 165 ASCII characters, fewer for text with letters such as ç, ğ, ı, ö, ş and ü, which take two bytes each in UTF-8. A ratchet message (§5.6) adds its 37-byte header: 104 bytes of overhead and a body of up to 128 bytes.

### 5.5 Introductions

An introduction is a first message to someone who does not have the sender's card yet. Only the sender needs the recipient's card. It is a bundle with the header of §5, except that its recipient tag is 8 random bytes, and its ciphertext field starts with a one-time public key:

```
e          = a fresh X25519 key pair, used once
shared     = X25519(e_secret, recipient_pk)                abort if all-zero
prk        = HKDF-Extract(salt = "carrychat/v0/intro", ikm = shared)
K_enc      = HKDF-Expand(prk, "enc" || e_pk || recipient_pk, 32)
K_ack      = HKDF-Expand(prk, "ack" || e_pk || recipient_pk, 32)
ack_secret = HMAC(K_ack, nonce)[0..16]
bundle_id  = H("carrychat/v0/ack" || ack_secret)[0..8]
signature  = Ed25519-Sign(sender's signing key, "carrychat/v0/intro-signature"
               || e_pk || recipient_pk || nonce || u32(created_at) || text)
plaintext  = 0x02 || u32(created_at) || u8(len(card)) || card || signature || text
ciphertext = e_pk || XChaCha20-Poly1305-Encrypt(K_enc, nonce, plaintext, aad)
```

`card` is the sender's contact card (§3.1) and `aad` is the one of §5.1.

**Receiving.** A node tries every bundle whose tag matches none of its contacts as an introduction: it computes `X25519(own_secret, e_pk)`, derives the keys and decrypts. If that works, it verifies the card's own signature, then `signature` with the card's Ed25519 key, then the lifetime rule and the bundle id. The card proves whose it is; the signature proves the sender holds that card's key, and ties the message to this recipient and this bundle, so it cannot be replayed to someone else. A node that opens an introduction emits a tombstone as for any bundle (§6).

**Requests.** If the sender is not a contact yet, it becomes a *request*: the app shows the message with the sender's name and fingerprint, and the user accepts or declines. Accepting makes the sender a contact. Declining deletes the request and remembers the sender's key: later introductions from it are dropped, but still tombstoned so they stop circulating. Pair bundles (§5.1) that arrive from a requester before the request is answered are kept with it.

**Sending.** A node sends introductions to a contact until it knows the contact has its card: a tombstone for one of its bundles to that contact came back, or a bundle from that contact arrived. From then on it sends pair bundles.

A short introduction takes two Meshtastic packets: its overhead is 164 bytes plus the card, which is 130 bytes plus the sender's name.

### 5.6 Ratchet sessions

Pair bundles sealed with `K_enc` (§5.1) have no forward secrecy: whoever later obtains a device's X25519 secret can open every recorded one. A ratchet session fixes that with the Double Ratchet (Signal), adapted to a network where bundles arrive days late, out of order, twice, or over several paths.

**Roles.** Of two contacts, the one whose X25519 public key sorts first is the *initiator*, the other the *responder*. Only the initiator starts a session, so the two can never start two sessions at once. Both begin from the same state: root key `K_root`, own ratchet key pair = own X25519 key pair, peer ratchet key = the peer's X25519 key for the initiator and none for the responder, no chains.

**Functions.**

```
KDF_RK(rk, dh) = HKDF-SHA256(salt = rk, ikm = dh, info = "carrychat/v1/ratchet", 64)  → (rk', ck)
KDF_CK(ck)     = (HMAC(ck, 0x02), HMAC(ck, 0x01))                                  → (ck', mk)
```

**Sending.** If the sender has no sending chain and is the initiator, it starts the session: a fresh ratchet key pair `DHs`, then `(RK, CKs) = KDF_RK(RK, X25519(DHs, DHr))`. Then `(CKs, mk) = KDF_CK(CKs)`, and the bundle is sealed as follows; `Ns` counts messages on the current sending chain and `PN` is the length of the previous one.

```
header     = 0x01 || DHs_public (32) || u16(PN) || u16(Ns)               37 bytes
header_ct  = header XOR XChaCha20(K_hdr, nonce)[0..37]                    keystream from block 0
ciphertext = header_ct || XChaCha20-Poly1305-Encrypt(mk, nonce, plaintext, aad || header_ct)
```

`aad` is the one of §5.1; the outer bundle, its tag, bundle id and tombstone are exactly as in §5. The header has no tag of its own: changing it changes the message key, and the AEAD fails. Encrypting the header keeps ratchet keys, which would otherwise link a conversation's bundles, from anyone without the pair keys. A responder with no sending chain yet seals with `K_enc` as in §5.1.

**Receiving.** A receiver decrypts the first 37 bytes with the keystream; if the first byte is `0x01`, it opens the bundle as a ratchet message, otherwise as in §5.1. For a ratchet message it uses a stored skipped key for `(DHs_public, Ns)` if it has one. Otherwise, if `DHs_public` differs from its peer ratchet key, it derives the rest of the current receiving chain up to `PN`, then takes a DH ratchet step: `PN = Ns`, `Ns = Nr = 0`, `DHr = DHs_public`, `(RK, CKr) = KDF_RK(RK, X25519(own, DHr))`, a fresh own ratchet key pair, `(RK, CKs) = KDF_RK(RK, X25519(own, DHr))`. It then derives the chain up to `Ns` and the message key. Whenever it derives a chain ahead, it does so however far, and keeps as skipped keys those of the last 1000 messages it passed: after a long silence, older messages expired on the way, and refusing to go further would lock the session for good. Every change is kept only if the AEAD succeeds.

**Starting.** When the initiator opens a `K_enc` bundle from its contact and has no sending chain, it replies with a session message (kind `0x05`), so the responder can switch to ratchet messages.

**Hygiene.** Message keys are deleted once used. Skipped keys are deleted once used and after 8 days. The store overwrites deleted data (SQLite `secure_delete`). A chain allows 65535 messages; a sender that reaches that without hearing back falls back to `K_enc`.

### 5.7 Groups

A group message is one bundle that every member can open, however many members there are. Each member has its own *sender chain*, which it hands to the other members in pair bundles (§5.1, §5.6), and seals and signs its group messages with it.

**State.** The member who creates a group is its admin. The group state is

```
group_id (16) || u32(version) || u8(len(name)) || name || u8(n) || (u8(len(card)) || card) × n
```

`group_id` is 16 random bytes, `name` is UTF-8 of at most 32 bytes, and the `n` cards (§3.1) are the members, the admin included, at most 16. The admin raises `version` with every change and sends the new state in a pair bundle of kind `0x06` to every member, including members it has just removed. A node takes the state of a group it does not know only from a contact it has accepted, who becomes that group's admin; after that, only from that admin and only with a higher version. A node that is not in the member list has left the group.

**Peers.** The state gives every member the cards of the others. A member that is not a contact becomes a *group peer*: the node derives its pair keys and tags (§4), exchanges pair bundles with it, and treats it as having its card, but shows no direct messages from it.

**Sender chains.** A member starts a sender chain when it joins a group, and again whenever it sees a member leave or be removed:

```
sender_key = 32 random bytes
CK         = 32 random bytes, at iteration 0
(sk, pk)   = a fresh Ed25519 key pair
```

It sends every other member a pair bundle of kind `0x07` with the chain as it stands:

```
group_id (16) || sender_key (32) || u32(iteration) || CK (32) || pk (32)
```

When a member joins, every other member sends it its current chain the same way, so the new member reads nothing sent before. A node keeps a chain it receives until it leaves the group, or until 8 days after the same member sent a newer one.

**Sealing.** Message `i` of a chain is sealed with the bundle header of §5:

```
prk        = HKDF-Extract(salt = "carrychat/v1/group", ikm = sender_key)
K_gtag     = HKDF-Expand(prk, "tag", 32)
K_ghdr     = HKDF-Expand(prk, "hdr", 32)
(CK', mk)  = KDF_CK(CK)                                 as in §5.6; CK' is at iteration i + 1
tag        = HMAC(K_gtag, u32(day(created_at)))[0..8]
bundle_id  = H("carrychat/v1/group" || nonce)[0..8]
header_ct  = u32(i) XOR XChaCha20(K_ghdr, nonce)[0..4]    keystream from block 0
sealed     = XChaCha20-Poly1305-Encrypt(mk, nonce, plaintext, aad || header_ct)
signature  = Ed25519-Sign(sk, "carrychat/v1/group-signature" || aad || header_ct || sealed)
ciphertext = header_ct || sealed || signature
```

`aad` is the one of §5.1 and the plaintext that of §5.2. Version 1 sends text (kind `0x01`) and files (kind `0x09`, §5.9) in groups. The tag changes daily and differs per chain, so carriers can tell neither which group a bundle belongs to nor which member sent it.

**Receiving.** A member precomputes the tags of every chain it holds over the tag window (§4.1). For a bundle whose tag matches, it verifies the signature (strict Ed25519) with the chain's `pk`, decrypts the header, and takes the message key for iteration `i`: a stored skipped key, or the chain derived forward, at most 100 000 iterations, keeping the keys of the last 1000 it passes. It then decrypts and checks the tag against `day(created_at)`, the lifetime rule of §5.1 and the bundle id. Changes are kept only if every step succeeds, and each key opens one message. The bundle is accepted only if the chain belongs to someone the node currently counts as a member. A node opens the bundles it carries again when it learns a new chain or member, as it does for a new contact (§8).

**Delivery.** A group bundle has no tombstone: one member's receipt must not stop it from reaching the others. It circulates until it expires, and members carry it on for one another.

**Leaving.** A member leaves by sending the admin a pair bundle of kind `0x08` with the group id, and the admin removes it. The admin cannot leave, but can remove every other member. A removed member receives none of the chains started after its removal. Until a member has seen the new state, though, it keeps writing on its old chain, which the removed member can still read.

**Size.** A group bundle adds 46 (header) + 4 (iteration) + 5 (plaintext header) + 16 (Poly1305 tag) + 64 (signature) = **135 bytes**, so up to **97 bytes** of text fit in one Meshtastic packet.

### 5.8 Public channels

A public channel is a conversation that anyone who knows its name can read and write, without cards or contacts. A channel may also have a password: then only those who know both can.

**Names.** A channel name is trimmed, loses a leading `#`, and is lowercased with the Unicode default case mapping (no locale rules), after which a combining dot above (U+0307) that follows `i` is dropped, so `İ` becomes `i`. It must then be 1 to 32 bytes of UTF-8 made of letters, digits (Unicode Alphabetic and Numeric), `-` and `_`. `#Mahalle` and `mahalle` are the same channel.

**Keys.**

```
prk     = HKDF-Extract(salt = "carrychat/v1/channel", ikm = name || 0x00 || password)
K_ch    = HKDF-Expand(prk, "enc", 32)
K_chtag = HKDF-Expand(prk, "tag", 32)
K_chid  = HKDF-Expand(prk, "id", 32)
tag(d)  = HMAC(K_chtag, u32(d))[0..8]
```

`password` is UTF-8 and empty for a channel without one. Anyone who knows the name and password derives the same keys, so the tag tells members which bundles are the channel's and tells nobody else anything; it changes every day.

**Authors.** Each author signs with an Ed25519 key of its own for that channel, so its posts in different channels cannot be linked to each other or to its card. The reference implementation derives it as `seed = HMAC(own Ed25519 secret, "carrychat/v1/channel-key" || K_chid)`. Every post also carries a nickname of at most 20 bytes, chosen freely, so readers tell authors apart by the key, not the name.

**Sealing.**

```
plaintext  = kind || u32(created_at) || pk (32) || u8(len(nick)) || nick || body
bundle_id  = H("carrychat/v1/channel" || nonce)[0..8]
sealed     = XChaCha20-Poly1305-Encrypt(K_ch, nonce, plaintext, aad)
signature  = Ed25519-Sign(sk, "carrychat/v1/channel-signature" || aad || sealed)
ciphertext = sealed || signature
```

`aad` is the one of §5.1 and the tag is `tag(day(created_at))`. The kind is text (`0x01`, the body is UTF-8), card (`0x04`, the body is the author's contact card, shared so readers can add them) or file (`0x09`, an offer, §5.9). A channel bundle lives at most a day, `expires_at ≤ created_at + 86400`, and starts with a hop limit of 4, so a busy channel does not fill the network.

**Receiving.** A member precomputes the tags of its channels over the tag window (§4.1). For a bundle whose tag matches, it decrypts, verifies the signature (strict Ed25519) with the `pk` inside, and checks the tag against `day(created_at)`, the channel lifetime and the bundle id. Like a group bundle it has no tombstone, and members carry it on for one another. A member may mute an author's key: it then neither shows nor passes on that author's posts. The reference implementation also refuses to send more than 10 posts a minute to one channel.

**Size.** A channel bundle adds 46 (header) + 5 (plaintext header) + 33 (key and nickname length) + 16 (Poly1305 tag) + 64 (signature) = **164 bytes** plus the nickname, so with an 8-byte nickname up to **60 bytes** of text fit in one Meshtastic packet.

### 5.9 Files

A photo or a voice message is too large for a bundle. It travels as chunks, sealed once with a key of its own, and the key travels in an ordinary message of kind `0x09` to a contact, a group or a channel. A group or channel gets one set of chunks for all its members.

**Offer.** The body of a file message is

```
u8(len(mime)) || mime || u32(size) || K_f (32) || H(file) (32)
```

`mime` is the media type, such as `image/jpeg` or `audio/mp4`, `size` the file's length, at most 1 MiB, and `K_f` 32 random bytes chosen for this file.

**Chunks.** The file is cut into chunks of 4000 bytes, the last one shorter, so it has `n = ceil(size / 4000)` chunks. Chunk `i` is a bundle with the header of §5:

```
prk        = HKDF-Extract(salt = "carrychat/v1/file", ikm = K_f)
K_fenc     = HKDF-Expand(prk, "enc", 32)
K_ftag     = HKDF-Expand(prk, "tag", 32)
K_fack     = HKDF-Expand(prk, "ack", 32)
tag        = HMAC(K_ftag, u32(i))[0..8]
ack_secret = HMAC(K_fack, nonce)[0..16]
bundle_id  = H("carrychat/v0/ack" || ack_secret)[0..8]
ciphertext = XChaCha20-Poly1305-Encrypt(K_fenc, nonce, file[4000·i .. 4000·(i+1)], aad)
```

`aad` is the one of §5.1, so the tag binds the chunk to its place. The chunks expire with the message that offers them and start with its hop limit. They travel in chunk frames (§7), which never go over LoRa: a 1 MiB file would take hours of airtime.

**Receiving.** A node that opens an offer computes the tags of its chunks and takes the chunk bundles that match them, from any link and in any order, including chunks it was already carrying. Once it has all `n`, it joins them and checks the length and `H(file)`; a file that fails is dropped. In a conversation between two contacts, the recipient publishes the tombstone of every chunk it takes (§6), so carriers drop it. In a group or channel it does not, and members carry the chunks on for one another, as they do group bundles.

**Carrying.** Nodes keep the chunks they carry for others under a quota of their own, separate from that of messages, so files never push out messages. Over the internet (§9.3) a node posts its chunks to each relay at a measured pace, so a photo stays within the relays' rate limits.

## 6. Tombstone

A tombstone proves delivery. After opening a bundle, the recipient publishes `(bundle_id, ack_secret)`.

Anyone holding the bundle checks `H("carrychat/v0/ack" || ack_secret)[0..8] == bundle_id`, then deletes the bundle and stops forwarding it. The sender runs the same check and marks the message delivered. Only the two contacts know `K_ack`, so only they can produce `ack_secret` for a real bundle. Forging one means finding a preimage of a 64-bit truncated hash.

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
| `0x04` chunk | `type \|\| bundle`, a file chunk (§5.9). Never sent over LoRa. |

If `1 + len(bundle) > mtu`, the bundle bytes are split into chunks of `mtu − 11` bytes, from 2 to 32 of them. Receivers reassemble by bundle id with a timeout (10 minutes by default) and a cap on the number of incomplete bundles, then check that the reassembled bundle carries the same id as its fragments. Relays reassemble before forwarding, because they need the hop limit and the expiry.

Frame types `0x10` to `0x1f` are reserved for transport-local control traffic (§9.1).

## 8. Forwarding

Each node keeps one store: its own outbox, plus the bundles it carries for others. Handling a frame is atomic: if any part of it fails, for example a write to the store, nothing of it is kept, so a tombstone never leaves for a message that was not kept.

- Duplicates are dropped by bundle id.
- Expired bundles are deleted. Bundles that claim to expire more than 8 days from now are refused.
- Forwarding decrements the hop limit. A bundle whose hop limit is 0 is kept but not forwarded.
- Bundles carried for others have a quota. When it is full, the bundle closest to expiry is evicted first.
- A bundle is not sent back over the connection it arrived on. A tombstone is, because the node at the other end may be the sender waiting for it.
- A node that adds a contact opens the bundles it already carries that turn out to be from that contact.
- **Split horizon on LoRa.** A bundle that arrived over LoRa is not sent over LoRa again by the same node, because the Meshtastic or MeshCore mesh already floods it. It may be bridged to other links. Bundles that arrived over any other link may be sent over LoRa within the airtime budget, with the node's own bundles and tombstones first.

## 9. Transports (draft)

These mappings are fixed when each transport is built (see the roadmap in the README).

### 9.1 Local network

Implemented in `crates/carrychat-node/src/lan.rs`.

**Discovery.** Every 3 seconds a node sends a 16-byte beacon to the multicast group `239.255.74.74`, port `47474`:

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 6 | `"CRCH" \|\| 0x00 \|\| 0x01` (magic, version, beacon) |
| 6 | 8 | session id, random per run |
| 14 | 2 | TCP port, `u16` |

The beacon carries no identity. On hearing a beacon from a node it is not connected to, the node with the smaller session id dials. The other node dials too after three beacons without a connection, in case its own beacons do not get through.

**Sync.** Implemented in `crates/carrychat-node/src/sync.rs`, and the same over Bluetooth LE (§9.5) and Wi-Fi Aware (§9.6). Over TCP, every frame is prefixed with its `u16` length. Besides the frames of §7, four control frames exist:

| Type | Layout |
| --- | --- |
| `0x10` HAVE | `type \|\| count (u16) \|\| bundle_id × count` |
| `0x11` WANT | `type \|\| count (u16) \|\| bundle_id × count` |
| `0x12` HELLO | `type \|\| session id (8)` |
| `0x13` NEARBY | `type \|\| kind (1) \|\| card` (§9.7) |

Both sides send HELLO first. A second connection between the same two nodes, over any link, is closed. Then each side sends HAVE with the ids of the bundles it can pass on, and every tombstone it knows. The other side answers with WANT for the ids it has never seen, and receives those bundles. After that, new bundles and tombstones are pushed as they appear. An empty HAVE every 15 seconds keeps the connection alive, and a connection silent for 45 seconds is closed.

### 9.2 LoRa via Meshtastic or MeshCore

Implemented in `crates/carrychat-node/src/lora.rs`, with the devices' protocols in `lora/meshtastic.rs` and `lora/meshcore.rs`. Desktops and the command line reach the device over USB serial or TCP; phones reach it over Bluetooth LE.

- **Which device.** Over USB serial or TCP, the node first asks as MeshCore's apps do (`CMD_APP_START`), again after 2 seconds, and takes a device that has not answered within 4 seconds for a Meshtastic one. MeshCore goes first: a MeshCore device on TCP reads any three bytes as a frame header and loses track of its frames when Meshtastic's come in, while a Meshtastic device skips bytes that are not its own. Over Bluetooth LE the service the device offers tells which it is.
- **Meshtastic device.** The node talks to a Meshtastic device with the Meshtastic client API, over USB serial at 115200 baud, over TCP (a device on the network, or meshtasticd, on port 4403) or over Bluetooth LE. Serial and TCP carry the API's stream framing, `0x94 0xC3 || u16 length || protobuf`. Over Bluetooth LE, as Meshtastic's own apps do on phones, each `ToRadio` message is written to the ToRadio characteristic and `FromRadio` messages are read one by one from FromRadio until it reads empty, whenever FromNum notifies; the platform code that does this frames them as on a serial port, so the node runs the same session over it. The device must be paired first. Before the first message on a serial port, the node sends 32 bytes of `0xC3`, which switch a device that is printing its log over to the API. It then asks for the configuration (`want_config_id`), reads the radio's modem settings and hop limit from it, and starts once `config_complete_id` arrives. A heartbeat every 5 minutes keeps the connection open. When the connection ends, the node connects again after 10 seconds.
- **What a Meshtastic device hears.** The configuration also lists the devices the device knows (`NodeInfo`) and its region. The node keeps their names and when, how strongly and over how many hops each was last heard, updated by every packet the device hears, and when CarryChat last came in over LoRa. Apps show the devices heard in the last three hours, which tells whether a mesh is in range. A device whose region is `UNSET` does not transmit, and apps say so. Nothing extra is sent to find devices: listening is free, and airtime is not.
- **Meshtastic packets.** Each frame is the payload of one Meshtastic `Data` packet, broadcast on the primary channel with the device's hop limit, MTU 233 bytes (`DATA_PAYLOAD_LEN`). Bundles longer than that travel as fragments (§7). During development it uses portnum `PRIVATE_APP` (256). A dedicated portnum will be requested once the format settles. Only bundles and tombstones go over LoRa, never file chunks.
- **Meshtastic channel.** The primary channel is the public one by default, so community devices with the same region and modem preset relay CarryChat packets like any others. A primary channel of another name moves to another frequency, where they do not hear it. CarryChat bundles are end-to-end encrypted already, so the public channel's shared key costs nothing.
- **MeshCore device.** The node speaks MeshCore's companion protocol, over USB serial at 115200 baud, over TCP (port 5000) or over Bluetooth LE. Serial and TCP frame each message as `<` (to the device) or `>` (from it) `|| u16 length (little-endian) || body`. Over Bluetooth LE each body is one write to the Nordic UART service's RX characteristic, or one notification on its TX characteristic; the platform code frames them as on a serial port. The device must be paired first. The node sends one command at a time and waits up to 5 seconds for its reply, taking in what the device pushes meanwhile. It asks for the number of channel slots (`CMD_DEVICE_QUERY`), the device's name and radio settings (`CMD_APP_START`), sets its clock (`CMD_SET_DEVICE_TIME`), and finds CarryChat's channel.
- **MeshCore channel.** Frames travel on the hashtag channel `#carrychat`, whose secret is the first 16 bytes of the SHA-256 of its name, as for any MeshCore hashtag channel. A device that does not have it gets it in its first free slot (no name and an all-zero secret). A device with every slot taken keeps its channels and sends nothing, and apps say so. Anyone can derive the channel's key; CarryChat bundles are end-to-end encrypted already.
- **MeshCore packets.** Each frame is one channel datagram (`CMD_SEND_CHANNEL_DATA`), flooded through every repeater, with data type `0xFFCC` from MeshCore's testing range until one is registered. A datagram carries at most 165 bytes (`MAX_GROUP_DATA_LENGTH`), so bundles longer than that travel as fragments (§7); until a device has answered, the node splits bundles to that size, which fits either kind. Datagrams heard wait on the device: the node fetches them (`CMD_SYNC_NEXT_MESSAGE`) when the device says some are waiting and every 60 seconds, and takes in those on CarryChat's channel with CarryChat's data type. Other messages are meant for MeshCore's own apps, and are dropped while CarryChat holds the connection. A frame the device's full send queue refuses goes first again after 2 seconds.
- **What a MeshCore device hears.** The node reads the devices the device knows (`CMD_GET_CONTACTS`) when it connects and every 10 minutes, and the adverts it pushes meanwhile: their names, when the device last heard them, and how many hops away they are when the path is known. Repeaters advertise themselves every 47 hours by default, so apps show the devices heard in the last two days.
- **Split horizon.** Both meshes rebroadcast packets themselves, Meshtastic with a hop limit of up to 7, so a node does not send over LoRa again a bundle or tombstone it heard there, whole or in fragments (§8). It passes them on to its other links. What a frame makes the node send, such as the receipt for a message to it, does go over LoRa.
- **Airtime.** In the `EU_868` region Meshtastic uses 869.4–869.65 MHz, where the legal duty cycle limit is 10%, and every node in range shares that airtime. A node keeps its own airtime under 2.5% of the time, with bursts of at most 20 seconds, computing each packet's time on air from the device's modem settings and an allowance for the mesh's header: 22 bytes for Meshtastic, 24 for MeshCore (header, path length, channel hash, MAC, data type and length, padding). The same budget applies whichever device it is. On LongFast a full packet takes about 2.2 seconds, so a burst holds about nine. Its own bundles and tombstones go first. Up to 32 frames carried for others wait for airtime; past that, the oldest is dropped.

### 9.3 Internet via Nostr

Implemented in `crates/carrychat-node/src/internet.rs`.

- **Events.** Every bundle and tombstone travels as one Nostr event (NIP-01) of kind `9378`, a provisional number not registered with Nostr. The content is the frame (§7) in base64url without padding. A bundle event has the tag `["x", hex(tag)]`, a tombstone event `["b", hex(bundle_id)]`.
- **Tags.** A pair bundle is posted under its recipient tag. An introduction, whose own tag is random, is posted under the recipient's mailbox tag, which anyone holding the recipient's card can compute:

  ```
  mailbox_tag(pk, d) = HMAC(H("carrychat/v0/mailbox" || pk), u32(d))[0..8]
  ```

  `d` is the day the introduction was created.
- **Keys.** Each event is signed (BIP-340) with a one-time key `HMAC(node_secret, content)`, where `node_secret` is a random secret the node keeps. Keys cannot be linked to the user or to each other, and the same node posting the same frame again produces the same event, which relays drop as a duplicate.
- **Time.** A bundle event's `created_at` is `expires_at − 604800`, never later than the bundle's creation. A tombstone event's is the time it is posted.
- **Subscription.** A node asks every relay for `{"kinds": [9378], "#x": [...], "since": now − 691200}` with the pair tags of all its contacts and its own mailbox tags over the tag window (§4.1), and for `{"kinds": [9378], "#b": [...]}` with the ids of its own bundles still waiting for a receipt. It renews the subscription when its contacts change, when it sends, and every 10 minutes.
- **Connections.** A node pings every relay every 20 seconds and drops a connection that has been silent for 40, then connects again. Right after sending, it also pings every relay and drops a connection that does not answer within 5 seconds. On connecting again it posts its waiting bundles once more. A computer that moves to another network can otherwise keep a dead connection for minutes without noticing, and lose what it sends into it.
- **What a node posts.** Its own bundles when it sends them and again on every connection, the tombstones of bundles it receives, and, as a gateway, every bundle it carries that came over another link: as it arrives if a relay is connected, otherwise on the next connection.
- **What a node takes.** Only bundles for itself and tombstones. A bundle from a relay that does not open is dropped, not carried, so a relay cannot fill a node's store.
- **Tor.** A node may reach the relays over Tor, each relay on a circuit of its own, with the relay's host name looked up at the exit. Such a node never connects directly: while Tor does not work, it does not use the internet at all.
- **Relays** see tags, sizes and timing, and, unless the node comes over Tor, its IP address. They also see which tags one connection asks for together, which links the tags of one node's contacts within a day.

The default relays are `wss://relay.damus.io`, `wss://nos.lol` and `wss://relay.primal.net`. Fetching for nodes without internet access (downlink through a gateway) is still open (§11).

### 9.4 File

Implemented in `crates/carrychat-node/src/file.rs`, with the extension `.carrychat`. A file is `"CRCH" || 0x00 || frames`, each frame prefixed with its `u16` length. Exporting writes the store's bundles and tombstones. Importing feeds every frame into the store, exactly as if it had arrived over any other link.

### 9.5 Bluetooth LE

Implemented in `crates/carrychat-node/src/bluetooth.rs` for desktops, through btleplug, and in the Android app (`BleLink.kt`), which hands the bytes of each connection to the node.

- **Advertisement.** A node advertises the service `43524348-a0c5-4a3e-9b31-6f2d8e7c1a01`. Its scan response carries, as service data for that uuid, the node's 8-byte session id, the one of its local network beacons (§9.1). It advertises no name.
- **Stream.** The service has one characteristic, `43524348-a0c5-4a3e-9b31-6f2d8e7c1a02`, with write, write without response and indicate. The node that connects (the GATT client) writes to it with response, and the node it connected to (the GATT server) sends indications on it, which makes a byte stream in each direction. Each write or indication carries up to MTU − 3 bytes of the stream, at most 512, and the next waits until the other side confirms the last. With more in flight, notifications filled a phone's Bluetooth queue until it could not answer the other side's writes, and the link dropped when an answer was 30 seconds late (the ATT timeout). A write longer than the MTU arrives as prepared writes and joins the stream when executed. Subscribing to the indications opens the stream; disconnecting ends it.
- **Sync.** Over the stream runs the sync of §9.1: frames prefixed with their `u16` length, HELLO, HAVE, WANT, then pushes as they appear. A node already connected over the local network is not connected again over Bluetooth, and a connection to it that is made anyway closes after HELLO.
- **Who connects.** As on the local network, the node with the smaller session id connects, and the other after three sightings without a connection. A node that cannot advertise connects regardless. Desktops cannot advertise through btleplug, so they connect to phones, and reach other desktops over the local network.
- **Desktops** write with response, 512 bytes at a time: macOS drops writes without response when its queue is full, which would break the stream. Since a scan reports each device once, they look at the devices seen so far every 5 seconds and connect to the CarryChat nodes among them that are not connected.
- **Android** runs the link in the foreground service that keeps the node alive, so it works while the app is closed. It asks for the Bluetooth permissions with `neverForLocation`.

### 9.6 Wi-Fi Aware

Implemented in the Android app (`WifiAwareLink.kt`), which hands the bytes of each connection to the node like Bluetooth's. Phones that support Wi-Fi Aware (Neighbor Awareness Networking) connect over Wi-Fi directly, with no access point and no internet, farther and much faster than over Bluetooth. Desktops do not take part.

- **Discovery.** Each phone publishes and subscribes to the service named `carrychat`. The publication's service-specific info is the node's 8-byte session id.
- **Who connects.** As on the other links, the node chooses by session id (§9.1). The subscriber that connects first sends the publisher a discovery message holding its own session id. The publisher then listens on a TCP port and asks for a data path to that subscriber, giving the port; the subscriber asks for a data path to the publisher, learns the port and the publisher's IPv6 address from it, and connects. Both data paths use the passphrase `carrychat-wifi-aware`: the link is encrypted, though everything on it is sealed end to end already.
- **Sync.** The TCP connection carries the sync of §9.1.
- **Android** runs the link in the foreground service, from Android 10. It asks for `NEARBY_WIFI_DEVICES` with `neverForLocation` from Android 13, and for the location permission before that, which Wi-Fi Aware discovery needs there.

### 9.7 People nearby

A node can be visible to people nearby. A visible node tells the nodes it is directly connected to, over the local network, Bluetooth, Wi-Fi Aware or Wi-Fi Direct, who it is, but only those that said they are visible too: two people see each other only when both turned it on, and a node that did not never sees anyone's card. Nothing about it is relayed.

| Kind | Meaning |
| --- | --- |
| `0` ASK | "I am visible: who are you?" |
| `1` ME | "This is me", followed by the sender's contact card (§3) |
| `2` GONE | "I am no longer visible." |

A visible node sends ASK on each new connection, and to every connected node when it becomes visible. A visible node answers ASK with ME, once per connection, and answers a ME it has not answered yet with its own ME. A node that is not visible ignores ASK and ME. A node that stops being visible sends GONE, and forgets everyone; the others forget it on GONE or when the connection ends. Adding someone seen nearby as a contact uses their card, as any card would (§5.5).

### 9.8 Wi-Fi Direct

Implemented in the Android app (`WifiDirectLink.kt`), which hands the bytes of each connection to the node like Wi-Fi Aware's. Wi-Fi Direct reaches phones that do not support Wi-Fi Aware: nearly every Android phone has it. Desktops do not take part.

- **Discovery.** Each phone announces a Wi-Fi Direct DNS-SD service, instance `CarryChat`, type `_carrychat._tcp`, whose TXT record holds `s`, the node's session id in hex, and `h`, `1` while the phone hosts a group and `0` otherwise. Each phone looks for that service every 30 seconds.
- **Who hosts.** Joining a Wi-Fi Direct group normally asks the user of the phone that owns it. Here the host makes an autonomous group whose network name, `DIRECT-CC-` followed by its session id in hex, and passphrase, `carrychat-` followed by the same, the other phone derives from the announcement, and joins it with them, so nobody is asked. Of two phones, the one with the higher session id hosts, since the node lets the other connect first (§9.1). A phone hosts when it hears a lower session id, unless it has joined a group; a phone joins a host with a higher session id when the node chooses to connect, unless it hosts itself. A group no one joins within 2 minutes is given up, and the phone does not host again for 10 minutes. A phone in a group talks to no other phone over Wi-Fi Direct.
- **Sync.** The host listens on TCP port 43524; the phone that joined connects to the group owner's address and runs the sync of §9.1 over it. It leaves the group when the sync ends. The passphrase encrypts the link, though everything on it is sealed end to end already.
- **Android** runs the link in the foreground service, from Android 10, which joining with a network name and passphrase needs. It asks for the same permissions as Wi-Fi Aware.

## 10. Security

**v0 provides:**

- confidentiality and integrity of message content end to end, whatever carried it,
- unlinkability for relays: without a pair key they cannot tell sender or recipient, or link a pair's traffic across days or directions,
- delivery receipts that cannot be forged,
- a closed inbox: only someone holding your card can write to you, and their first message is a request you accept or decline,
- group messages that only current members can read and that no member can forge in another member's name (§5.7),
- public channels whose posts cannot be linked to their authors' cards or across channels, and that nobody can forge in another author's name (§5.8).

**v0 does not provide:**

- **Hidden introductions.** An introduction cannot be linked to its sender or recipient, but its size sets it apart from most pair bundles.
- **Forward secrecy for every bundle.** Ratchet messages (§5.6) have it: a key taken from a device later opens neither the recorded messages it already received nor, once a reply has gone each way, the ones after. Introductions, session-less `K_enc` bundles and bundles from before version 1 do not, and whoever obtains a device's X25519 secret can impersonate any of its contacts to it. Forward secrecy protects recorded traffic, not the history kept on the device.
- **Radio metadata privacy.** A LoRa transmission reveals the transmitting Meshtastic or MeshCore node and, to direction-finding equipment, its location. Bundle sizes and timing are visible, because v0 does not pad.
- **Replay protection on the network.** A recorded bundle can be injected again until it expires. Receivers drop duplicates, but replays still spend airtime.
- **Fragment integrity at relays.** Relays cannot authenticate fragments, so a forged fragment spoils reassembly of that bundle until the timeout.
- **Privacy in public channels.** Anyone who knows a channel's name, and its password if it has one, reads its posts, including posts they recorded before. A password is only as strong as it is hard to guess, since the keys come from it directly. Channel posts have no forward secrecy and no delivery receipts.
- **Immediate removal from a group.** Members that have not yet seen a removal keep writing on chains the removed member holds. Group bundles also have no delivery receipts, and a group's admin and member list are visible to every member.
- **Reliable time.** Devices without internet access may have wrong clocks. The tag window absorbs a day of drift, and expiry checks use local time.

## 11. Open questions

1. Downlink for nodes without internet access through a gateway, without telling the gateway who they are.
2. Padding for LoRa frames: a smaller size leak against more airtime.
3. Exchanging contacts at a distance.
4. HF radio. Amateur bands forbid encryption that hides a message's meaning, so HF would need a separate signed but unencrypted message kind, limited to messages the user explicitly marks as public.

## 12. Constants

| Name | Value |
| --- | --- |
| Protocol version | `0` |
| Longest name | 32 bytes |
| Longest bundle | 4096 bytes |
| Bundle overhead | 67 bytes, 104 with a ratchet header |
| Introduction overhead | 164 bytes plus the card |
| Group bundle overhead | 135 bytes |
| Members in a group | 16 at most, the admin included |
| Channel bundle overhead | 164 bytes plus the nickname |
| Channel bundle lifetime | 86400 s (1 day) at most, hop limit 4 |
| Longest channel name and nickname | 32 and 20 bytes |
| Longest file | 1048576 bytes (1 MiB), in chunks of 4000 bytes |
| Default lifetime | 259200 s (3 days) |
| Longest lifetime | 604800 s (7 days) |
| Default hop limit | 8 |
| Tag window | `today − 7` to `today + 1` |
| Meshtastic MTU | 233 bytes |
| MeshCore MTU | 165 bytes |
| MeshCore channel, data type | `#carrychat`, `0xFFCC` |
| LoRa airtime budget | 2.5% of the time, bursts of at most 20 s |
| Most fragments per bundle | 32 |
| Reassembly timeout | 600 s |
| Bluetooth LE service | `43524348-a0c5-4a3e-9b31-6f2d8e7c1a01` |
| Bluetooth LE stream characteristic | `43524348-a0c5-4a3e-9b31-6f2d8e7c1a02` |
| Wi-Fi Aware service name | `carrychat` |
| Wi-Fi Aware data path passphrase | `carrychat-wifi-aware` |
