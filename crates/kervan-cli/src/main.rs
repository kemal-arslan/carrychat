//! `kervan`: a Kervan node on the command line. Useful as a relay on a
//! computer without a screen, and for trying things out.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use kervan_node::{ContactView, Event, InternetConfig, LanConfig, LanProblem, Node};
use tokio::io::{AsyncBufReadExt, BufReader};

#[derive(Parser)]
#[command(
    name = "kervan",
    version,
    about = "Messages that get through without the internet"
)]
struct Cli {
    /// Where this node keeps its identity and messages.
    #[arg(long, env = "KERVAN_DIR", global = true)]
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
    /// Queue a message. It goes out the next time `kervan run` finds a link.
    Send {
        contact: String,
        #[arg(required = true, num_args = 1..)]
        text: Vec<String>,
    },
    /// Show the conversation with a contact.
    Log { contact: String },
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
        /// Send every line typed on standard input to this contact.
        #[arg(long)]
        chat: Option<String>,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("kervan: {error}");
            ExitCode::FAILURE
        }
    }
}

type AnyResult = Result<(), Box<dyn std::error::Error>>;

async fn run(cli: Cli) -> AnyResult {
    let dir = cli.dir.unwrap_or_else(default_dir);
    if let Command::Init { name } = &cli.command {
        let node = Node::create(&dir, name)?;
        println!("Created {} in {}", name, dir.display());
        print_card(&node);
        return Ok(());
    }
    let node = Node::open(&dir)?.ok_or_else(|| {
        format!(
            "no identity in {}; create one with `kervan init <name>`",
            dir.display()
        )
    })?;

    match cli.command {
        Command::Init { .. } => unreachable!("handled above"),
        Command::Card => print_card(&node),
        Command::Add { card } => {
            let contact = node.add_contact(&card)?;
            println!("Added {} ({})", contact.name, contact.fingerprint);
            println!("Compare this fingerprint with theirs before you trust it.");
        }
        Command::Contacts => {
            for contact in node.contacts()? {
                let request = if contact.request { "  (request)" } else { "" };
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
                "Queued for {}. It leaves with the next `kervan run` or `kervan export`.",
                contact.name
            );
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
                let mark = if message.outgoing && message.delivered {
                    " ✓"
                } else {
                    ""
                };
                println!("{who}: {}{mark}", message.body);
            }
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
                if relays.is_empty() {
                    InternetConfig::default()
                } else {
                    InternetConfig {
                        relays,
                        ..InternetConfig::default()
                    }
                }
            });
            run_node(node, lan, internet, connect, chat).await?;
        }
    }
    Ok(())
}

async fn run_node(
    node: Node,
    lan: LanConfig,
    internet: Option<InternetConfig>,
    connect: Vec<SocketAddr>,
    chat: Option<ContactView>,
) -> AnyResult {
    let mut events = node.subscribe();
    let port = node.start(lan).await?;
    if let Some(internet) = internet {
        println!("Connecting to {} relay(s).", internet.relays.len());
        node.start_internet(internet)?;
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
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut stdin_open = chat.is_some();
    loop {
        tokio::select! {
            event = events.recv() => match event {
                Ok(Event::MessageReceived { message }) => {
                    println!("{}: {}", names(message.contact_id), message.body);
                }
                Ok(Event::MessageDelivered { contact_id, .. }) => {
                    println!("✓ delivered to {}", names(contact_id));
                }
                Ok(Event::PeersChanged { count }) => println!("· {count} node(s) nearby"),
                Ok(Event::InternetChanged { relays }) => println!("· {relays} internet relay(s) connected"),
                Ok(Event::ContactRequest { contact }) => {
                    println!("? {} ({}) wants to message you.", contact.name, contact.fingerprint);
                    println!("  Accept with `kervan accept {}`, or `kervan decline {}`.", contact.id, contact.id);
                }
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
        .join(".kervan")
}
