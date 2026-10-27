//! `carrychat`: a CarryChat node on the command line. Useful as a relay on a
//! computer without a screen, and for trying things out.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use carrychat_node::{
    ChannelView, ContactView, Event, FileView, GroupView, InternetConfig, LanConfig, LanProblem,
    LoraDevice, Node, TorState,
};
use clap::{Parser, Subcommand};
use tokio::io::{AsyncBufReadExt, BufReader};

#[derive(Parser)]
#[command(
    name = "carrychat",
    version,
    about = "Messages that get through without the internet"
)]
struct Cli {
    /// Where this node keeps its identity and messages.
    #[arg(long, env = "CARRYCHAT_DIR", global = true)]
    dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create an identity.
    Init { name: String },
    /// Print your contact card and fingerprint.
    Card,
    /// Add a contact from their card.
    Add { card: String },
    /// List contacts and requests.
    Contacts,
    /// Accept a request from someone who wrote to you first.
    Accept { contact: String },
    /// Decline a request and ignore its sender from now on.
    Decline { contact: String },
    /// Queue a message. It goes out the next time `carrychat run` finds a link.
    Send {
        contact: String,
        #[arg(required = true, num_args = 1..)]
        text: Vec<String>,
    },
    /// Queue a photo or voice message for a contact, at most 1 MiB.
    SendFile { contact: String, path: PathBuf },
    /// Write a file that arrived to disk, by the number the log shows.
    SaveFile { file: i64, path: PathBuf },
    /// Show the conversation with a contact and mark it read.
    Log { contact: String },
    /// Change your name. Contacts get the new name with your new card.
    Rename { name: String },
    /// Groups: create one, change who is in it, write to it, read it.
    #[command(subcommand)]
    Group(GroupCommand),
    /// Public channels: anyone who knows the name reads and writes.
    #[command(subcommand)]
    Channel(ChannelCommand),
    /// Tell contacts when you read their messages, or stop telling them.
    ReadReceipts {
        #[arg(value_parser = ["on", "off"])]
        state: String,
    },
    /// Delete your identity, contacts and messages from this computer.
    Erase {
        /// Required: there is no undo.
        #[arg(long)]
        yes: bool,
    },
    /// Write everything this node holds to a file, to carry to another node.
    Export { file: PathBuf },
    /// Take in a file written by `export`.
    Import { file: PathBuf },
    /// Run on the local network: relay for others and receive messages.
    Run {
        /// Also connect to this node (`host:port`), when discovery does not work.
        #[arg(long)]
        connect: Vec<SocketAddr>,
        /// TCP port to listen on. Picks a free one by default.
        #[arg(long, default_value_t = 0)]
        port: u16,
        /// Turn off multicast discovery.
        #[arg(long)]
        no_discovery: bool,
        /// Stay off the internet: no Nostr relays.
        #[arg(long)]
        no_internet: bool,
        /// Nostr relay to use instead of the defaults. Repeat for more.
        #[arg(long = "relay")]
        relays: Vec<String>,
        /// Reach the relays over Tor only, never directly.
        #[arg(long)]
        tor: bool,
        /// Also connect to nearby phones over Bluetooth LE.
        #[arg(long)]
        bluetooth: bool,
        /// Also use a Meshtastic or MeshCore device for LoRa radio: its serial
        /// port, such as /dev/cu.usbserial-0001, or tcp:host:port (Meshtastic
        /// and meshtasticd: 4403, MeshCore: 5000).
        #[arg(long, value_name = "DEVICE")]
        lora: Option<LoraDevice>,
        /// Also reach CarryChat nodes on a Reticulum network, through the
        /// Reticulum node at this host:port (an rnsd TCP server interface).
        #[arg(long, value_name = "HOST:PORT")]
        reticulum: Option<String>,
        /// Send every line typed on standard input to this contact.
        #[arg(long)]
        chat: Option<String>,
    },
}

#[derive(Subcommand)]
enum GroupCommand {
    /// List your groups and their members.
    List,
    /// Create a group with these contacts. You become its admin.
    Create {
        name: String,
        #[arg(required = true, num_args = 1..)]
        contacts: Vec<String>,
    },
    /// Add a contact to a group you are the admin of.
    Add { group: String, contact: String },
    /// Take a member off a group you are the admin of, by name or fingerprint.
    Remove { group: String, member: String },
    /// Leave a group.
    Leave { group: String },
    /// Queue a message to a group. It goes out like any other message.
    Send {
        group: String,
        #[arg(required = true, num_args = 1..)]
        text: Vec<String>,
    },
    /// Queue a photo or voice message for a group, at most 1 MiB.
    SendFile { group: String, path: PathBuf },
    /// Show a group's messages and mark them read.
    Log { group: String },
}

#[derive(Subcommand)]
enum ChannelCommand {
    /// List the channels you are in.
    List,
    /// Join a channel. Posts are signed with a key for that channel only.
    Join {
        name: String,
        /// Only those who know it read and write.
        #[arg(long, default_value = "")]
        password: String,
        /// The name to post under. Your own name by default.
        #[arg(long)]
        nick: Option<String>,
    },
    /// Leave a channel and forget its posts.
    Leave { channel: String },
    /// Post under another name.
    Nick { channel: String, nick: String },
    /// Queue a post. It goes out like any other message.
    Send {
        channel: String,
        #[arg(required = true, num_args = 1..)]
        text: Vec<String>,
    },
    /// Post your contact card, so readers can add you.
    ShareCard { channel: String },
    /// Post a photo or voice message, at most 1 MiB.
    SendFile { channel: String, path: PathBuf },
    /// Show a channel's posts and mark them read.
    Log { channel: String },
    /// Stop showing and passing on an author's posts, by the id in the log.
    Mute { channel: String, author: String },
    /// Show a muted author's posts again.
    Unmute { channel: String, author: String },
    /// List the authors you muted in a channel.
    Muted { channel: String },
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("carrychat: {error}");
            ExitCode::FAILURE
        }
    }
}

type AnyResult = Result<(), Box<dyn std::error::Error>>;

async fn run(cli: Cli) -> AnyResult {
    let dir = cli.dir.unwrap_or_else(default_dir);
    if let Command::Erase { yes } = &cli.command {
        if !yes {
            return Err(
                "this deletes your identity, contacts and messages for good; add --yes".into(),
            );
        }
        carrychat_node::erase(&dir)?;
        println!("Erased everything in {}", dir.display());
        return Ok(());
    }
    if let Command::Init { name } = &cli.command {
        let node = Node::create(&dir, name)?;
        println!("Created {} in {}", name, dir.display());
        print_card(&node);
        return Ok(());
    }
    let node = Node::open(&dir)?.ok_or_else(|| {
        format!(
            "no identity in {}; create one with `carrychat init <name>`",
            dir.display()
        )
    })?;

    match cli.command {
        Command::Init { .. } | Command::Erase { .. } => unreachable!("handled above"),
        Command::Card => print_card(&node),
        Command::Add { card } => {
            let contact = node.add_contact(&card)?;
            println!("Added {} ({})", contact.name, contact.fingerprint);
            println!("Compare this fingerprint with theirs before you trust it.");
        }
        Command::Contacts => {
            for contact in node.contacts()? {
                let request = if contact.request {
                    "  (request)"
                } else if contact.secure {
                    "  (forward secrecy)"
                } else {
                    ""
                };
                println!(
                    "{:>3}  {:<32}  {}{request}",
                    contact.id, contact.name, contact.fingerprint
                );
            }
        }
        Command::Send { contact, text } => {
            let contact = find_contact(&node, &contact)?;
            node.send_text(contact.id, &text.join(" "))?;
            println!(
                "Queued for {}. It leaves with the next `carrychat run` or `carrychat export`.",
                contact.name
            );
        }
        Command::SendFile { contact, path } => {
            let contact = find_contact(&node, &contact)?;
            let data = std::fs::read(&path)?;
            node.send_file(contact.id, mime_of(&path), &data)?;
            println!(
                "Queued {} for {}. It leaves with the next `carrychat run` or `carrychat export`.",
                path.display(),
                contact.name
            );
        }
        Command::SaveFile { file, path } => {
            let (mime, data) = node
                .file_data(file)?
                .ok_or_else(|| format!("file {file} is not all here yet"))?;
            std::fs::write(&path, &data)?;
            println!("Saved {mime}, {} bytes, to {}", data.len(), path.display());
        }
        Command::Accept { contact } => {
            let contact = find_contact(&node, &contact)?;
            node.accept_contact(contact.id)?;
            println!("{} is a contact now. You can reply.", contact.name);
        }
        Command::Decline { contact } => {
            let contact = find_contact(&node, &contact)?;
            node.decline_contact(contact.id)?;
            println!(
                "Declined {}. Their messages are deleted and new ones are ignored.",
                contact.name
            );
        }
        Command::Log { contact } => {
            let contact = find_contact(&node, &contact)?;
            for message in node.messages(contact.id)? {
                let who = if message.outgoing {
                    "you"
                } else {
                    contact.name.as_str()
                };
                let mark = match (message.outgoing, message.read, message.delivered) {
                    (true, true, _) => " ✓✓",
                    (true, false, true) => " ✓",
                    _ => "",
                };
                println!(
                    "{who}: {}{}{mark}",
                    message.body,
                    describe(message.file.as_ref())
                );
            }
            // Read receipts wait for the next `carrychat run` or `carrychat export`.
            node.mark_read(contact.id)?;
        }
        Command::Rename { name } => {
            let profile = node.rename(&name)?;
            println!("You are {} now. Contacts get the new name with the next `carrychat run` or `carrychat export`.", profile.name);
        }
        Command::Group(command) => group_command(&node, command)?,
        Command::Channel(command) => channel_command(&node, command)?,
        Command::ReadReceipts { state } => {
            node.set_read_receipts(state == "on")?;
            println!("Read receipts are {state}.");
        }
        Command::Export { file } => {
            let frames = node.export_file(&file)?;
            println!("Wrote {frames} item(s) to {}", file.display());
        }
        Command::Import { file } => {
            let report = node.import_file(&file)?;
            println!(
                "{} new message(s) for you, {} delivery receipt(s), {} carried for others, {} unreadable",
                report.received, report.delivered, report.carried, report.skipped
            );
        }
        Command::Run {
            connect,
            port,
            no_discovery,
            no_internet,
            relays,
            tor,
            bluetooth,
            lora,
            reticulum,
            chat,
        } => {
            let chat = chat.map(|name| find_contact(&node, &name)).transpose()?;
            let mut lan = LanConfig {
                listen_port: port,
                ..LanConfig::default()
            };
            if no_discovery {
                lan.discovery = None;
            }
            let internet = (!no_internet).then(|| {
                let mut config = InternetConfig {
                    tor: tor.then(|| dir.join("tor")),
                    ..InternetConfig::default()
                };
                if !relays.is_empty() {
                    config.relays = relays;
                }
                config
            });
            let radio = Radio { lora, reticulum };
            run_node(node, lan, internet, bluetooth, radio, connect, chat).await?;
        }
    }
    Ok(())
}

/// The links that go through a device or a network of their own.
struct Radio {
    lora: Option<LoraDevice>,
    reticulum: Option<String>,
}

async fn run_node(
    node: Node,
    lan: LanConfig,
    internet: Option<InternetConfig>,
    bluetooth: bool,
    radio: Radio,
    connect: Vec<SocketAddr>,
    chat: Option<ContactView>,
) -> AnyResult {
    let mut events = node.subscribe();
    let port = node.start(lan).await?;
    if let Some(internet) = internet {
        println!("Connecting to {} relay(s).", internet.relays.len());
        node.start_internet(internet)?;
    }
    if bluetooth {
        match node.start_bluetooth().await {
            Ok(()) => println!("Looking for phones over Bluetooth."),
            Err(error) => eprintln!("Bluetooth is off: {error}"),
        }
    }
    if let Some(device) = radio.lora {
        println!("Connecting to the LoRa device {device:?}.");
        node.start_lora(device)?;
    }
    if let Some(address) = radio.reticulum {
        println!("Connecting to the Reticulum node at {address}.");
        node.start_reticulum(&address)?;
    }
    println!(
        "{} is listening on port {port}. Ctrl-C stops.",
        node.profile().name
    );
    for addr in connect {
        if let Err(error) = node.connect(addr).await {
            eprintln!("could not connect to {addr}: {error}");
        }
    }

    let names = |id: i64| {
        node.contacts()
            .ok()
            .and_then(|contacts| contacts.into_iter().find(|contact| contact.id == id))
            .map_or_else(|| format!("contact {id}"), |contact| contact.name)
    };
    // A contact change is a new name or the start of forward secrecy.
    let mut known: HashMap<i64, String> = node
        .contacts()?
        .into_iter()
        .map(|contact| (contact.id, contact.name))
        .collect();
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut stdin_open = chat.is_some();
    loop {
        tokio::select! {
            event = events.recv() => match event {
                Ok(Event::MessageReceived { message }) => {
                    println!("{}: {}{}", names(message.contact_id), message.body, describe(message.file.as_ref()));
                }
                Ok(Event::MessageDelivered { contact_id, .. }) => {
                    println!("✓ delivered to {}", names(contact_id));
                }
                Ok(Event::MessageMoved { contact_id, message_id }) => {
                    if let Some(step) = node.journey(message_id).ok().and_then(|journey| journey.steps.last().cloned()) {
                        println!("→ on its way to {} over {}", names(contact_id), step.via.as_str());
                    }
                }
                Ok(Event::LoraChanged { connected }) => println!(
                    "· LoRa device {}",
                    if connected { "connected" } else { "gone" }
                ),
                Ok(Event::ReticulumChanged { connected }) => println!(
                    "· Reticulum node {}",
                    if connected { "connected" } else { "gone" }
                ),
                Ok(Event::PeersChanged { lan, bluetooth, wifi_aware, wifi_direct }) => {
                    println!("· {} node(s) nearby", lan + bluetooth + wifi_aware + wifi_direct)
                }
                Ok(Event::NearbyChanged { .. }) => {
                    let people: Vec<String> = node.nearby().into_iter().map(|person| person.name).collect();
                    println!("· visible nearby: {}", if people.is_empty() { "nobody".to_owned() } else { people.join(", ") });
                }
                Ok(Event::InternetChanged { relays }) => match node.status().ok().and_then(|status| status.internet_tor) {
                    Some(TorState::Starting) => println!("· starting Tor"),
                    Some(TorState::Failed { detail }) => eprintln!("! Tor did not start: {detail}"),
                    Some(TorState::Ready) => println!("· {relays} internet relay(s) connected over Tor"),
                    None => println!("· {relays} internet relay(s) connected"),
                },
                Ok(Event::MessagesRead { contact_id, message_ids }) => {
                    println!("✓✓ {} read {} message(s)", names(contact_id), message_ids.len());
                }
                Ok(Event::ContactChanged { contact }) => match known.insert(contact.id, contact.name.clone()) {
                    Some(old) if old != contact.name => println!("· {old} is called {} now", contact.name),
                    _ if contact.secure => println!("· forward secrecy with {}", contact.name),
                    _ => {}
                },
                Ok(Event::ContactRequest { contact }) => {
                    known.insert(contact.id, contact.name.clone());
                    println!("? {} ({}) wants to message you.", contact.name, contact.fingerprint);
                    println!("  Accept with `carrychat accept {}`, or `carrychat decline {}`.", contact.id, contact.id);
                }
                Ok(Event::GroupMessageReceived { message }) => {
                    let group = node
                        .groups()
                        .ok()
                        .and_then(|groups| groups.into_iter().find(|group| group.id == message.group_id))
                        .map_or_else(|| format!("group {}", message.group_id), |group| group.name);
                    let sender = message.sender.unwrap_or_default();
                    println!("[{group}] {sender}: {}{}", message.body, describe(message.file.as_ref()));
                }
                Ok(Event::PostReceived { post }) => {
                    let channel = node
                        .channels()
                        .ok()
                        .and_then(|channels| channels.into_iter().find(|channel| channel.id == post.channel_id))
                        .map_or_else(|| format!("channel {}", post.channel_id), |channel| channel.name);
                    match post.shared_card {
                        Some(card) => println!(
                            "[#{channel}] {} ({}) shared a card: {} {}",
                            post.nick, post.author, card.name, card.fingerprint
                        ),
                        None => println!("[#{channel}] {} ({}): {}{}", post.nick, post.author, post.body, describe(post.file.as_ref())),
                    }
                }
                Ok(Event::GroupChanged { group }) => {
                    if group.member {
                        println!("· group {} has {} member(s)", group.name, group.members.len());
                    } else {
                        println!("· you are no longer in group {}", group.name);
                    }
                }
                Ok(Event::FileChanged { file }) if file.ready => {
                    println!("· file {} is here: {}, {} bytes. Save it with `carrychat save-file {}`.", file.id, file.mime, file.size, file.id);
                }
                Ok(Event::FileChanged { .. }) => {}
                Ok(Event::LocalNetworkChanged { problem: None }) => println!("· local network works"),
                Ok(Event::LocalNetworkChanged { problem: Some(LanProblem::Blocked { detail }) }) => {
                    eprintln!("! macOS is blocking local network access ({detail}).");
                    eprintln!("  Allow your terminal app in System Settings → Privacy & Security → Local Network.");
                }
                Ok(Event::LocalNetworkChanged { problem: Some(LanProblem::Failed { detail }) }) => {
                    eprintln!("! local network: {detail}");
                }
                Err(_) => {}
            },
            line = lines.next_line(), if stdin_open => match (line, &chat) {
                (Ok(Some(text)), Some(contact)) if !text.trim().is_empty() => {
                    node.send_text(contact.id, &text)?;
                }
                (Ok(Some(_)), _) => {}
                _ => stdin_open = false,
            },
            _ = tokio::signal::ctrl_c() => return Ok(()),
        }
    }
}

fn group_command(node: &Node, command: GroupCommand) -> AnyResult {
    match command {
        GroupCommand::List => {
            for group in node.groups()? {
                let state = match (group.member, group.admin) {
                    (false, _) => "  (left)",
                    (true, true) => "  (admin)",
                    (true, false) => "",
                };
                println!("{:>3}  {}{state}", group.id, group.name);
                for member in &group.members {
                    let role = match (member.me, member.admin) {
                        (true, _) => " (you)",
                        (false, true) => " (admin)",
                        (false, false) => "",
                    };
                    println!("       {:<32}  {}{role}", member.name, member.fingerprint);
                }
            }
        }
        GroupCommand::Create { name, contacts } => {
            let ids = contacts
                .iter()
                .map(|contact| find_contact(node, contact).map(|contact| contact.id))
                .collect::<Result<Vec<_>, _>>()?;
            let group = node.create_group(&name, &ids)?;
            println!(
                "Created {} with {} member(s). They join with the next `carrychat run` or `carrychat export`.",
                group.name,
                group.members.len()
            );
        }
        GroupCommand::Add { group, contact } => {
            let group = find_group(node, &group)?;
            let contact = find_contact(node, &contact)?;
            node.add_group_member(group.id, contact.id)?;
            println!("Added {} to {}.", contact.name, group.name);
        }
        GroupCommand::Remove { group, member } => {
            let group = find_group(node, &group)?;
            let found = group
                .members
                .iter()
                .find(|m| {
                    !m.me && (m.fingerprint == member || m.name.eq_ignore_ascii_case(&member))
                })
                .ok_or_else(|| format!("{} has no member {member}", group.name))?;
            node.remove_group_member(group.id, &found.fingerprint)?;
            println!(
                "Removed {} from {}. The others start new keys when they hear of it.",
                found.name, group.name
            );
        }
        GroupCommand::Leave { group } => {
            let group = find_group(node, &group)?;
            node.leave_group(group.id)?;
            println!("You left {}.", group.name);
        }
        GroupCommand::Send { group, text } => {
            let group = find_group(node, &group)?;
            node.send_group_text(group.id, &text.join(" "))?;
            println!(
                "Queued for {}. It leaves with the next `carrychat run` or `carrychat export`.",
                group.name
            );
        }
        GroupCommand::SendFile { group, path } => {
            let group = find_group(node, &group)?;
            node.send_group_file(group.id, mime_of(&path), &std::fs::read(&path)?)?;
            println!("Queued {} for {}.", path.display(), group.name);
        }
        GroupCommand::Log { group } => {
            let group = find_group(node, &group)?;
            for message in node.group_messages(group.id)? {
                let who = message.sender.as_deref().unwrap_or("you");
                println!("{who}: {}{}", message.body, describe(message.file.as_ref()));
            }
            node.mark_group_read(group.id)?;
        }
    }
    Ok(())
}

fn channel_command(node: &Node, command: ChannelCommand) -> AnyResult {
    match command {
        ChannelCommand::List => {
            for channel in node.channels()? {
                let lock = if channel.has_password {
                    "  (password)"
                } else {
                    ""
                };
                println!(
                    "{:>3}  #{} as {}{lock}",
                    channel.id, channel.name, channel.nick
                );
            }
        }
        ChannelCommand::Join {
            name,
            password,
            nick,
        } => {
            let nick = nick.unwrap_or_else(|| node.profile().name);
            let channel = node.join_channel(&name, &password, &nick)?;
            println!(
                "Joined #{} as {}. Anyone who knows the name{} reads what you post there.",
                channel.name,
                channel.nick,
                if channel.has_password {
                    " and password"
                } else {
                    ""
                }
            );
        }
        ChannelCommand::Leave { channel } => {
            let channel = find_channel(node, &channel)?;
            node.leave_channel(channel.id)?;
            println!("Left #{}.", channel.name);
        }
        ChannelCommand::Nick { channel, nick } => {
            let channel = find_channel(node, &channel)?;
            let channel = node.set_channel_nick(channel.id, &nick)?;
            println!("You post in #{} as {} now.", channel.name, channel.nick);
        }
        ChannelCommand::Send { channel, text } => {
            let channel = find_channel(node, &channel)?;
            node.send_post(channel.id, &text.join(" "))?;
            println!(
                "Queued for #{}. It leaves with the next `carrychat run` or `carrychat export`.",
                channel.name
            );
        }
        ChannelCommand::ShareCard { channel } => {
            let channel = find_channel(node, &channel)?;
            node.share_card(channel.id)?;
            println!("Your card is queued for #{}.", channel.name);
        }
        ChannelCommand::Log { channel } => {
            let channel = find_channel(node, &channel)?;
            for post in node.posts(channel.id)? {
                let who = if post.outgoing {
                    "you".to_owned()
                } else {
                    format!("{} ({})", post.nick, post.author)
                };
                match post.shared_card {
                    Some(card) => {
                        println!("{who} shared a card: {} {}", card.name, card.fingerprint)
                    }
                    None => println!("{who}: {}{}", post.body, describe(post.file.as_ref())),
                }
            }
            node.mark_channel_read(channel.id)?;
        }
        ChannelCommand::SendFile { channel, path } => {
            let channel = find_channel(node, &channel)?;
            node.send_channel_file(channel.id, mime_of(&path), &std::fs::read(&path)?)?;
            println!("Queued {} for #{}.", path.display(), channel.name);
        }
        ChannelCommand::Mute { channel, author } => {
            let channel = find_channel(node, &channel)?;
            node.mute_author(channel.id, &author)?;
            println!("Muted {author} in #{}.", channel.name);
        }
        ChannelCommand::Unmute { channel, author } => {
            let channel = find_channel(node, &channel)?;
            node.unmute_author(channel.id, &author)?;
            println!(
                "{author} is not muted in #{} any more. Posts dropped meanwhile come with the next sync.",
                channel.name
            );
        }
        ChannelCommand::Muted { channel } => {
            let channel = find_channel(node, &channel)?;
            for muted in node.muted_authors(channel.id)? {
                println!("{}  {}", muted.author, muted.nick);
            }
        }
    }
    Ok(())
}

/// A channel by id or by name, with or without `#`.
fn find_channel(node: &Node, query: &str) -> Result<ChannelView, Box<dyn std::error::Error>> {
    let channels = node.channels()?;
    if let Ok(id) = query.parse::<i64>() {
        if let Some(channel) = channels.iter().find(|channel| channel.id == id) {
            return Ok(channel.clone());
        }
    }
    let name = carrychat_proto::channel::normalize_name(query)?;
    channels
        .into_iter()
        .find(|channel| channel.name == name)
        .ok_or_else(|| format!("you are not in #{name}").into())
}

/// A group by id or by name, ignoring case.
fn find_group(node: &Node, query: &str) -> Result<GroupView, Box<dyn std::error::Error>> {
    let groups = node.groups()?;
    if let Ok(id) = query.parse::<i64>() {
        if let Some(group) = groups.iter().find(|group| group.id == id) {
            return Ok(group.clone());
        }
    }
    let mut matches = groups
        .into_iter()
        .filter(|group| group.name.eq_ignore_ascii_case(query));
    match (matches.next(), matches.next()) {
        (Some(group), None) => Ok(group),
        (Some(_), Some(_)) => {
            Err(format!("more than one group is called {query}; use the id").into())
        }
        (None, _) => Err(format!("no group called {query}").into()),
    }
}

/// A file offered in a message, as the log shows it.
fn describe(file: Option<&FileView>) -> String {
    let Some(file) = file else {
        return String::new();
    };
    let state = if file.ready {
        format!("file {}", file.id)
    } else {
        format!("{}/{} chunks", file.received, file.chunks)
    };
    format!("[{}, {} KB, {state}]", file.mime, file.size.div_ceil(1024))
}

/// The media type of a file to send, from its extension.
fn mime_of(path: &Path) -> &'static str {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    match extension.as_deref() {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        Some("m4a" | "mp4" | "aac") => "audio/mp4",
        Some("webm") => "audio/webm",
        Some("ogg" | "opus") => "audio/ogg",
        _ => "application/octet-stream",
    }
}

fn print_card(node: &Node) {
    let profile = node.profile();
    println!(
        "Your card (give it to your contacts):\n\n{}\n",
        profile.card
    );
    println!("Fingerprint: {}", profile.fingerprint);
}

/// A contact by id or by name, ignoring case.
fn find_contact(node: &Node, query: &str) -> Result<ContactView, Box<dyn std::error::Error>> {
    let contacts = node.contacts()?;
    if let Ok(id) = query.parse::<i64>() {
        if let Some(contact) = contacts.iter().find(|contact| contact.id == id) {
            return Ok(contact.clone());
        }
    }
    let mut matches = contacts
        .into_iter()
        .filter(|contact| contact.name.eq_ignore_ascii_case(query));
    match (matches.next(), matches.next()) {
        (Some(contact), None) => Ok(contact),
        (Some(_), Some(_)) => {
            Err(format!("more than one contact is called {query}; use the id").into())
        }
        (None, _) => Err(format!("no contact called {query}").into()),
    }
}

fn default_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(".").to_path_buf())
        .join(".carrychat")
}
