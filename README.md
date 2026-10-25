# Kervan

**Reach the people you care about when the internet is down.**

Kervan is a messenger for computers that keeps working without the internet. It uses the internet when there is one. When there isn't, it uses whatever is left: your local network, a $20 LoRa radio plugged into USB, or a USB stick someone carries to the next city. Messages are end-to-end encrypted, wait until a path appears, and pass through other people's devices without those devices being able to read them or tell who they are for.

[![CI](https://github.com/kemal-arslan/kervan/actions/workflows/ci.yml/badge.svg)](https://github.com/kemal-arslan/kervan/actions/workflows/ci.yml) [![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE) [![Buy Me a Coffee](https://img.shields.io/badge/Buy%20me%20a%20coffee-support-FFDD00?logo=buymeacoffee&logoColor=black)](https://buymeacoffee.com/kemalarslan)

> **Status: early.** The protocol (v0 draft), its reference implementation and its test vectors exist. The app does not yet. See the [roadmap](#roadmap).

## Why

After the earthquakes of 6 February 2023 in Türkiye, mobile networks across the affected region were down or overloaded for days, just when people most needed to reach each other. Ordinary messengers assume the internet is there. Bluetooth mesh apps such as [bitchat](https://github.com/permissionlesstech/bitchat) work without it, but only between phones that are close to each other. Kervan is for the gap: computers, long distances, many kinds of link, one inbox.

## How it works

Kervan sends each message over the best link it has, and keeps it until one appears:

| Link | Reach | Used when |
| --- | --- | --- |
| Local network | a building or a campus | the router works but the internet doesn't |
| LoRa radio ([Meshtastic](https://meshtastic.org)) | kilometres per hop, further over community relays | there is no internet at all |
| Internet ([Nostr](https://nostr.com) relays) | anywhere | normal days, or whenever one computer on the path is online |
| File on a USB stick | as far as someone carries it | nothing else reaches |

- **One message, one radio packet.** A bundle adds 67 bytes to a message, so up to 165 bytes of text fit into a single 233-byte Meshtastic packet. Longer messages are split.
- **Store and forward.** Every computer keeps its own outgoing messages and carries other people's for up to 7 days, passing them on when it meets a new link.
- **Nothing to learn in transit.** Devices that carry a message see only a tag that changes daily and differs per direction. Without one of the two keys, they cannot tell who it is from or for.
- **Delivery receipts the network can check.** The recipient publishes a short proof that only the two contacts can produce, and every device that carries a copy deletes it.
- **Contacts only.** You exchange cards in person, by QR code, and compare fingerprints. Nobody else can message you, so there is no spam.

The full specification is in [docs/PROTOCOL.md](docs/PROTOCOL.md). [docs/test-vectors.json](docs/test-vectors.json) lets another implementation check itself byte for byte.

## Roadmap

1. **Protocol v0, reference implementation, test vectors** (done)
2. Local network link, message store, desktop app (Tauri), USB export and import
3. LoRa link through a Meshtastic radio over USB
4. Internet link over Nostr, and gateways that let offline islands reach it
5. Bridge to the bitchat Bluetooth mesh, mobile apps, HF radio

## Build

Requires Rust 1.85 or later.

```bash
cargo test
cargo run -p kervan-proto --example gen_vectors
```

## Security

Kervan v0 has no forward secrecy. Anyone who obtains a device's secret key can decrypt the traffic of that device that they have recorded. LoRa transmissions also reveal that a radio is transmitting and from roughly where. [docs/PROTOCOL.md §10](docs/PROTOCOL.md#10-security) lists every known limitation. Until v1, do not rely on Kervan where being found out is dangerous.

## License

[MIT](LICENSE)

Kervan is free and open source. If it ever carries a message that mattered, you can [buy me a coffee](https://buymeacoffee.com/kemalarslan).
