# CarryChat

**English** · [Türkçe](README.tr.md)

**Offline-first, end-to-end encrypted messaging between phones and computers over any link: local network, Bluetooth, Wi-Fi Aware, Wi-Fi Direct, LoRa radio, Reticulum, internet or a file on a USB stick.**

CarryChat is a store-and-forward messenger. Each message is sealed into a bundle of at most 4 KiB, kept until a link towards the recipient exists, and passed from device to device on the way. Devices that carry a bundle can neither read it nor tell who sent it or who it is for. The internet is one link among several, not a requirement.

[![CI](https://github.com/kemal-arslan/carrychat/actions/workflows/ci.yml/badge.svg)](https://github.com/kemal-arslan/carrychat/actions/workflows/ci.yml) [![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE) [![Buy Me a Coffee](https://img.shields.io/badge/Buy%20me%20a%20coffee-support-FFDD00?logo=buymeacoffee&logoColor=black)](https://buymeacoffee.com/kemalarslan)

> **Status: early.** CarryChat works today over the local network, Bluetooth, the internet (Nostr relays) and by file, from a desktop app, an Android app or the command line. The LoRa radio (Meshtastic and MeshCore) and Reticulum links are written and wait for their first field test on real devices, as do Wi-Fi Direct and passing cards over NFC, which need two phones. The latest release, [0.2.0](https://github.com/kemal-arslan/carrychat/releases/tag/v0.2.0), has the Android app and the macOS app for Apple silicon. See the [roadmap](#roadmap).

## How it works

CarryChat sends each message over the best link it has, and keeps it until one appears:

| Link | Reach | Used when |
| --- | --- | --- |
| Local network | a building or a campus | the router works but the internet doesn't |
| Bluetooth LE | a few dozen metres, further from phone to phone | there is no network at all |
| Wi-Fi Aware | around a hundred metres, phone to phone, fast enough for photos | there is no router and no internet (Android phones that support it) |
| Wi-Fi Direct | around a hundred metres, phone to phone, fast enough for photos | the same, on Android phones without Wi-Fi Aware |
| LoRa radio ([Meshtastic](https://meshtastic.org) or [MeshCore](https://meshcore.io)) | kilometres per hop, further over community relays | there is no internet at all |
| [Reticulum](https://reticulum.network) | as far as the Reticulum network reaches, over its radios, serial lines and internet links | a Reticulum network is at hand |
| Internet ([Nostr](https://nostr.com) relays) | anywhere | normal days, or whenever one computer on the path is online |
| Sound | a few metres, side by side | nothing else reaches, and the other device listens |
| File on a USB stick | as far as someone carries it | nothing else reaches |

- **One message, one radio packet.** A bundle adds 104 bytes to a message (67 before a session starts), so up to 128 bytes of text fit into a single 233-byte Meshtastic packet. Longer messages are split.
- **Tor built in.** If you choose, CarryChat reaches the internet relays over Tor, so they never see your IP address and your network does not see that you use CarryChat. It never falls back to a direct connection: while Tor does not work, it stays off the internet and keeps using the local network and files.
- **Store and forward.** Every computer keeps its own outgoing messages and carries other people's for up to 7 days, passing them on when it meets a new link.
- **Nothing to learn in transit.** Devices that carry a message see only a tag that changes daily and differs per direction. Without one of the two keys, they cannot tell who it is from or for.
- **Forward secrecy.** Once both contacts have written, keys change with every reply: a Double Ratchet adapted to messages that arrive days late, twice or out of order. A key taken from a device later opens none of the messages before it.
- **Groups of up to 16.** A group message is one bundle that every member opens, so it costs the same airtime for 3 people as for 16, and members carry it on for one another. Members need not know each other. Each member signs what it writes, so nobody can write in another's name, and a removed member reads nothing sent after the others hear of it. Up to 97 bytes of text fit one radio packet.
- **Public channels.** Anyone who knows a channel's name, and its password if it has one, reads and writes there, without exchanging cards. Each author signs with a key for that channel only, so posts cannot be linked to their card or across channels, and nobody can post as someone else. Posts live a day and travel at most 4 hops; you can mute an author, and share your card when you want people to add you.
- **Photos and voice messages.** A photo is shrunk to at most 1280 pixels and 1 MB, sealed once with a key of its own and cut into chunks; the key goes in an ordinary message, so files get the same protection as text. Chunks travel over the local network, the internet and files, never over the radio, and have their own storage quota so they never push out messages.
- **Delivery receipts the network can check.** The recipient publishes a short proof that only the two contacts can produce, and every device that carries a copy deletes it.
- **Only card holders can write.** To write to someone you need their card. Your first message carries your own card, signed, so they see a request with your fingerprint and accept or decline it. Nobody without your card can reach you.

The full specification is in [docs/PROTOCOL.md](docs/PROTOCOL.md) ([Türkçe](docs/PROTOCOL.tr.md)). [docs/test-vectors.json](docs/test-vectors.json) lets another implementation check itself byte for byte.

## Try it

### Install

[Releases](https://github.com/kemal-arslan/carrychat/releases) has the Android app and the macOS app:

- **Android 7 or later:** download `CarryChat-0.2.0.apk` on the phone, open it and allow installing apps from that source. Later releases update it in place.
- **macOS on Apple silicon:** unzip `CarryChat-0.2.0-macos-arm64.zip` and move CarryChat to Applications. The app is not notarized yet, so macOS blocks it the first time: open System Settings → **Privacy & Security** and click **Open Anyway**, or run `xattr -dr com.apple.quarantine /Applications/CarryChat.app`.

Windows, Linux and Intel Macs build from source, as below. Building needs Rust 1.85 or later and Node.js 20 or later.

### Desktop app

```bash
cd app
npm install
npm run tauri dev
```

Create your identity and give your card (**My card**, also as a QR code, under **Me** or the QR button) to the people you want to hear from, or add theirs with **Add contact** on **People**. Side by side, **Play by sound** under **My card** plays the card as a few seconds of tones, and **Listen** under **Add contact** fills it in, with no network between the two devices. Two Android phones with NFC also pass the card when held back to back, one with **My card** open and the other with **Add contact**. Only one side needs the other's card: the first message introduces the sender, and the recipient accepts the request. The pencil button on **Chats** also starts a group with contacts you have written to, or joins a public channel. CarryChat devices on the same network find each other by themselves, and while there is internet, messages also go through public Nostr relays, so devices on different networks reach each other. The **Network** tab shows each link: if your network blocks them, use **Connect to an address** there with the other device's address, and **Export to a file** writes everything waiting to be delivered to a `.carrychat` file that someone can carry to another CarryChat device and import there.

**Be visible nearby**, on **People**, shows the people around you who turned it on too, over the local network, Bluetooth, Wi-Fi Aware or Wi-Fi Direct, and lets you write to them without exchanging cards first. While it is off, no one nearby sees you and you see no one.

**LoRa radio**, on **Network**, uses a [Meshtastic](https://meshtastic.org) or [MeshCore](https://meshcore.io) device: a phone pairs with it over Bluetooth, a computer uses it over USB or the network, and CarryChat finds out which kind it is. Once chosen, it shows the device's region or radio settings and the devices it has heard recently (three hours for Meshtastic, two days for MeshCore, whose repeaters announce themselves less often), so you can tell whether a mesh is in range. Phones and computers have no LoRa radio of their own, so it takes a separate device. On a MeshCore device, CarryChat adds a `#carrychat` channel to a free slot.

**Reticulum**, on **Network**, connects to a [Reticulum](https://reticulum.network) node, such as `rnsd` on the same computer with a TCP server interface, and reaches the other CarryChat nodes on that network however many hops away. It shows the CarryChat nodes that announced themselves in the last six hours.

**Sound**, on **Network**, listens for messages that a device right next to this one plays as sound, while the app is open. In a chat, the speaker button next to the message field sends what you write as sound too, played three times. It is slow, about 30 bytes a second, but needs nothing else: the message is sealed end to end, so those who hear the tones cannot read it. Cards and messages play at 18 to 20 kHz, which most adults do not hear, at about 23 bytes a second. Not every speaker plays that high: if the other device does not hear them, turn off **Above hearing**, the row below **Sound**, to play tones you can hear. Listening hears both.

`npm run dev` opens the interface in a browser with sample data, without building the app.

### Command line

```bash
cargo install --path crates/carrychat-cli
carrychat init Ayşe
carrychat card                                  # give this to your contacts
carrychat add carrychat:…                          # or add theirs: one side is enough
carrychat accept Deniz                          # accept someone who wrote to you first
carrychat send Deniz "İyiyim, Kadıköy'deyim."
carrychat send-file Deniz photo.jpg             # a photo or voice message, up to 1 MB
carrychat save-file 3 photo.jpg                 # a file that arrived, by the number in the log
carrychat run                                   # deliver, receive and relay (local network and internet)
carrychat run --tor                             # the same, with the internet only over Tor
carrychat run --bluetooth                       # also reach nearby phones over Bluetooth
carrychat run --lora /dev/cu.usbserial-0001     # and LoRa through a Meshtastic or MeshCore device (or tcp:host:port)
carrychat run --reticulum 127.0.0.1:4242        # and Reticulum, through an rnsd TCP server interface
carrychat export messages.carrychat                # or carry everything on a USB stick
carrychat group create Mahalle Deniz Mehmet     # you become the group's admin
carrychat group send Mahalle "Su dağıtımı okulun önünde."
carrychat group log Mahalle                     # also: group list, add, remove, leave
carrychat channel join "#kadıköy" --nick Ayşe   # a public channel; --password for a closed one
carrychat channel send kadıköy "Moda'da çadır alanı kuruldu."
carrychat channel log kadıköy                   # also: channel list, share-card, mute, nick, leave
carrychat rename "Ayşe Yılmaz"                  # contacts get the new name
carrychat read-receipts off                     # stop telling contacts what you read
carrychat erase --yes                           # delete identity, contacts and messages
```

`carrychat run` on a computer without a screen turns it into a relay: it carries other people's messages until they are delivered, without being able to read them.

## Roadmap

1. **Protocol v0, reference implementation, test vectors** (done)
2. **Local network link, message store, desktop app, command line, USB export and import** (done)
3. **LoRa link through a Meshtastic or MeshCore radio, and the Reticulum link** (written: the command line and the macOS app use a device over USB or TCP, the Android app over Bluetooth; it shows the region or radio settings and the devices heard. Field tests come next)
4. **Internet link over Nostr** (done). Computers with internet also pass on what they carry for computers without it. Fetching for those computers is still open.
5. **Android app, Bluetooth, Wi-Fi Aware and Wi-Fi Direct links, people nearby, cards over sound and NFC** (done; Wi-Fi Aware, Wi-Fi Direct and NFC wait for a test between two phones). iOS app, HF radio

## Build

```bash
cargo test                       # protocol, node and command line
cd app && npm run tauri build    # desktop app installer
```

On Linux the desktop app needs WebKitGTK. See the [Tauri prerequisites](https://tauri.app/start/prerequisites/).

The Android app builds from the same project. It needs JDK 17, the Android SDK with NDK 27, and the Rust Android targets, set up as the [Tauri Android prerequisites](https://tauri.app/start/prerequisites/#android) describe. OpenSSL is built into the app on Android, and its build looks for `aarch64-linux-android-ranlib`, which current NDKs no longer ship, so point it at the NDK's own `llvm-ranlib` (under `prebuilt/linux-x86_64` on Linux):

```bash
export RANLIB_aarch64_linux_android=$NDK_HOME/toolchains/llvm/prebuilt/darwin-x86_64/bin/llvm-ranlib
cd app && npx tauri android build --target aarch64 --apk
```

On a phone CarryChat keeps running after its screen is closed, with an ongoing notification, so messages still arrive, and are passed on for others, while the app is out of sight. It also starts by itself when the phone starts or the app is updated, without being opened. Messages that arrive then are announced as notifications. **Turn off** in that notification stops CarryChat, also across restarts, until it is opened again.

| Crate | What it is |
| --- | --- |
| [`carrychat-proto`](crates/carrychat-proto) | Wire format and cryptography, no I/O |
| [`carrychat-node`](crates/carrychat-node) | Message store, forwarding, local network and file links |
| [`carrychat-cli`](crates/carrychat-cli) | The `carrychat` command |
| [`app`](app) | The desktop and Android app (Tauri and React); its look follows [DESIGN.md](DESIGN.md) ([Türkçe](DESIGN.tr.md)) |

## Security

Your secret keys are stored unencrypted in the app's data folder, which only your user account can read. Anyone with access to your account, or a copy of that folder, can read your messages and send as you.

Messages have forward secrecy once both contacts have written to each other. The first messages, sent before that, do not: anyone who obtains a device's secret key can decrypt those they have recorded. LoRa transmissions also reveal that a radio is transmitting and from roughly where. [docs/PROTOCOL.md §10](docs/PROTOCOL.md#10-security) lists every known limitation. Until v1, do not rely on CarryChat where being found out is dangerous.

## License

[MIT](LICENSE)

CarryChat is free and open source. If it ever carries a message that mattered, you can [buy me a coffee](https://buymeacoffee.com/kemalarslan).
