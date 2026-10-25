# Kervan

**Offline-first, end-to-end encrypted messaging between computers over any link: local network, LoRa radio, internet or a file on a USB stick.**

Kervan is a store-and-forward messenger. Each message is sealed into a bundle of at most 4 KiB, kept until a link towards the recipient exists, and passed from computer to computer on the way. Computers that carry a bundle can neither read it nor tell who sent it or who it is for. The internet is one link among several, not a requirement.

[![CI](https://github.com/kemal-arslan/kervan/actions/workflows/ci.yml/badge.svg)](https://github.com/kemal-arslan/kervan/actions/workflows/ci.yml) [![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE) [![Buy Me a Coffee](https://img.shields.io/badge/Buy%20me%20a%20coffee-support-FFDD00?logo=buymeacoffee&logoColor=black)](https://buymeacoffee.com/kemalarslan)

> **Status: early.** Kervan works today over the local network, the internet (Nostr relays) and by file, from a desktop app or the command line. The LoRa radio link comes next. Nothing is packaged yet. See the [roadmap](#roadmap).

## How it works

Kervan sends each message over the best link it has, and keeps it until one appears:

| Link | Reach | Used when |
| --- | --- | --- |
| Local network | a building or a campus | the router works but the internet doesn't |
| LoRa radio ([Meshtastic](https://meshtastic.org)) | kilometres per hop, further over community relays | there is no internet at all |
| Internet ([Nostr](https://nostr.com) relays) | anywhere | normal days, or whenever one computer on the path is online |
| File on a USB stick | as far as someone carries it | nothing else reaches |

- **One message, one radio packet.** A bundle adds 104 bytes to a message (67 before a session starts), so up to 128 bytes of text fit into a single 233-byte Meshtastic packet. Longer messages are split.
- **Store and forward.** Every computer keeps its own outgoing messages and carries other people's for up to 7 days, passing them on when it meets a new link.
- **Nothing to learn in transit.** Devices that carry a message see only a tag that changes daily and differs per direction. Without one of the two keys, they cannot tell who it is from or for.
- **Forward secrecy.** Once both contacts have written, keys change with every reply: a Double Ratchet adapted to messages that arrive days late, twice or out of order. A key taken from a device later opens none of the messages before it.
- **Groups of up to 16.** A group message is one bundle that every member opens, so it costs the same airtime for 3 people as for 16, and members carry it on for one another. Members need not know each other. Each member signs what it writes, so nobody can write in another's name, and a removed member reads nothing sent after the others hear of it. Up to 97 bytes of text fit one radio packet.
- **Delivery receipts the network can check.** The recipient publishes a short proof that only the two contacts can produce, and every device that carries a copy deletes it.
- **Only card holders can write.** To write to someone you need their card. Your first message carries your own card, signed, so they see a request with your fingerprint and accept or decline it. Nobody without your card can reach you.

The full specification is in [docs/PROTOCOL.md](docs/PROTOCOL.md). [docs/test-vectors.json](docs/test-vectors.json) lets another implementation check itself byte for byte.

## Try it

You need Rust 1.85 or later and Node.js 20 or later.

### Desktop app

```bash
cd app
npm install
npm run tauri dev
```

Create your identity and give your card (**My card**, also as a QR code) to the people you want to hear from, or add theirs with **Add contact**. Only one side needs the other's card: the first message introduces the sender, and the recipient accepts the request. The **+** next to **Groups** starts a group with contacts you have written to. Kervan computers on the same network find each other by themselves, and while there is internet, messages also go through public Nostr relays, so computers on different networks reach each other. If your network blocks that, use **Connect…** with the other computer's address. **Export file…** writes everything waiting to be delivered to a `.kervan` file that someone can carry to another Kervan computer and import there.

`npm run dev` opens the interface in a browser with sample data, without building the app.

### Command line

```bash
cargo install --path crates/kervan-cli
kervan init Ayşe
kervan card                                  # give this to your contacts
kervan add kervan:…                          # or add theirs: one side is enough
kervan accept Deniz                          # accept someone who wrote to you first
kervan send Deniz "İyiyim, Kadıköy'deyim."
kervan run                                   # deliver, receive and relay (local network and internet)
kervan export messages.kervan                # or carry everything on a USB stick
kervan group create Mahalle Deniz Mehmet     # you become the group's admin
kervan group send Mahalle "Su dağıtımı okulun önünde."
kervan group log Mahalle                     # also: group list, add, remove, leave
kervan rename "Ayşe Yılmaz"                  # contacts get the new name
kervan read-receipts off                     # stop telling contacts what you read
kervan erase --yes                           # delete identity, contacts and messages
```

`kervan run` on a computer without a screen turns it into a relay: it carries other people's messages until they are delivered, without being able to read them.

## Roadmap

1. **Protocol v0, reference implementation, test vectors** (done)
2. **Local network link, message store, desktop app, command line, USB export and import** (done)
3. LoRa link through a Meshtastic radio over USB
4. **Internet link over Nostr** (done). Computers with internet also pass on what they carry for computers without it. Fetching for those computers is still open.
5. Bluetooth link, mobile apps, HF radio

## Build

```bash
cargo test                       # protocol, node and command line
cd app && npm run tauri build    # desktop app installer
```

On Linux the desktop app needs WebKitGTK. See the [Tauri prerequisites](https://tauri.app/start/prerequisites/).

| Crate | What it is |
| --- | --- |
| [`kervan-proto`](crates/kervan-proto) | Wire format and cryptography, no I/O |
| [`kervan-node`](crates/kervan-node) | Message store, forwarding, local network and file links |
| [`kervan-cli`](crates/kervan-cli) | The `kervan` command |
| [`app`](app) | The desktop app (Tauri and React) |

## Security

Your secret keys are stored unencrypted in the app's data folder, which only your user account can read. Anyone with access to your account, or a copy of that folder, can read your messages and send as you.

Messages have forward secrecy once both contacts have written to each other. The first messages, sent before that, do not: anyone who obtains a device's secret key can decrypt those they have recorded. LoRa transmissions also reveal that a radio is transmitting and from roughly where. [docs/PROTOCOL.md §10](docs/PROTOCOL.md#10-security) lists every known limitation. Until v1, do not rely on Kervan where being found out is dangerous.

## License

[MIT](LICENSE)

Kervan is free and open source. If it ever carries a message that mattered, you can [buy me a coffee](https://buymeacoffee.com/kemalarslan).
