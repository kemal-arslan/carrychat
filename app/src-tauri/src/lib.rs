//! The desktop app: a thin layer of commands over a [`Node`]. Node events are
//! forwarded to the window as `kervan-event`.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Mutex;

use kervan_node::{
    ChannelView, ContactView, GroupMessageView, GroupView, ImportReport, InternetConfig, LanConfig,
    MessageView, MutedView, Node, PostView, Profile, Settings, Status,
};
use tauri::{AppHandle, Emitter, Manager, State};

struct AppState {
    dir: PathBuf,
    node: Mutex<Option<Node>>,
}

type CommandResult<T> = Result<T, String>;

impl AppState {
    fn node(&self) -> CommandResult<Node> {
        self.node
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .ok_or_else(|| "no identity yet".to_owned())
    }
}

fn text(error: impl ToString) -> String {
    error.to_string()
}

/// Starts the local network and internet links and forwards node events to
/// the window.
fn launch(app: &AppHandle, node: &Node) {
    let mut events = node.subscribe();
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        while let Ok(event) = events.recv().await {
            let _ = handle.emit("kervan-event", event);
        }
    });
    let node = node.clone();
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = node.start(LanConfig::default()).await {
            let _ = handle.emit(
                "kervan-error",
                format!("Local network link is off: {error}"),
            );
        }
        if let Err(error) = node.start_internet(InternetConfig::default()) {
            let _ = handle.emit("kervan-error", format!("Internet link is off: {error}"));
        }
    });
}

#[tauri::command]
fn profile(state: State<'_, AppState>) -> Option<Profile> {
    state.node().ok().map(|node| node.profile())
}

#[tauri::command]
fn create_profile(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
) -> CommandResult<Profile> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Choose a name".to_owned());
    }
    let node = Node::create(&state.dir, name).map_err(text)?;
    launch(&app, &node);
    let profile = node.profile();
    *state
        .node
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(node);
    Ok(profile)
}

#[tauri::command]
fn contacts(state: State<'_, AppState>) -> CommandResult<Vec<ContactView>> {
    state.node()?.contacts().map_err(text)
}

#[tauri::command]
fn add_contact(state: State<'_, AppState>, card: String) -> CommandResult<ContactView> {
    state.node()?.add_contact(&card).map_err(text)
}

#[tauri::command]
fn mark_read(state: State<'_, AppState>, contact_id: i64) -> CommandResult<()> {
    state.node()?.mark_read(contact_id).map_err(text)
}

#[tauri::command]
fn rename(state: State<'_, AppState>, name: String) -> CommandResult<Profile> {
    state.node()?.rename(&name).map_err(text)
}

#[tauri::command]
fn settings(state: State<'_, AppState>) -> CommandResult<Settings> {
    state.node()?.settings().map_err(text)
}

#[tauri::command]
fn set_read_receipts(state: State<'_, AppState>, on: bool) -> CommandResult<()> {
    state.node()?.set_read_receipts(on).map_err(text)
}

/// Deletes the identity, contacts and messages, then starts the app again
/// on the first-run screen.
#[tauri::command]
fn erase_everything(app: AppHandle, state: State<'_, AppState>) -> CommandResult<()> {
    state
        .node
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take();
    kervan_node::erase(&state.dir).map_err(text)?;
    app.restart()
}

#[tauri::command]
fn accept_contact(state: State<'_, AppState>, contact_id: i64) -> CommandResult<ContactView> {
    state.node()?.accept_contact(contact_id).map_err(text)
}

#[tauri::command]
fn decline_contact(state: State<'_, AppState>, contact_id: i64) -> CommandResult<()> {
    state.node()?.decline_contact(contact_id).map_err(text)
}

#[tauri::command]
fn messages(state: State<'_, AppState>, contact_id: i64) -> CommandResult<Vec<MessageView>> {
    state.node()?.messages(contact_id).map_err(text)
}

#[tauri::command]
fn send_message(
    state: State<'_, AppState>,
    contact_id: i64,
    text: String,
) -> CommandResult<MessageView> {
    state
        .node()?
        .send_text(contact_id, &text)
        .map_err(self::text)
}

#[tauri::command]
fn groups(state: State<'_, AppState>) -> CommandResult<Vec<GroupView>> {
    state.node()?.groups().map_err(text)
}

#[tauri::command]
fn group_messages(
    state: State<'_, AppState>,
    group_id: i64,
) -> CommandResult<Vec<GroupMessageView>> {
    state.node()?.group_messages(group_id).map_err(text)
}

#[tauri::command]
fn create_group(
    state: State<'_, AppState>,
    name: String,
    contact_ids: Vec<i64>,
) -> CommandResult<GroupView> {
    state
        .node()?
        .create_group(&name, &contact_ids)
        .map_err(text)
}

#[tauri::command]
fn add_group_member(
    state: State<'_, AppState>,
    group_id: i64,
    contact_id: i64,
) -> CommandResult<GroupView> {
    state
        .node()?
        .add_group_member(group_id, contact_id)
        .map_err(text)
}

#[tauri::command]
fn remove_group_member(
    state: State<'_, AppState>,
    group_id: i64,
    fingerprint: String,
) -> CommandResult<GroupView> {
    state
        .node()?
        .remove_group_member(group_id, &fingerprint)
        .map_err(text)
}

#[tauri::command]
fn leave_group(state: State<'_, AppState>, group_id: i64) -> CommandResult<GroupView> {
    state.node()?.leave_group(group_id).map_err(text)
}

#[tauri::command]
fn send_group_message(
    state: State<'_, AppState>,
    group_id: i64,
    text: String,
) -> CommandResult<GroupMessageView> {
    state
        .node()?
        .send_group_text(group_id, &text)
        .map_err(self::text)
}

#[tauri::command]
fn mark_group_read(state: State<'_, AppState>, group_id: i64) -> CommandResult<()> {
    state.node()?.mark_group_read(group_id).map_err(text)
}

#[tauri::command]
fn channels(state: State<'_, AppState>) -> CommandResult<Vec<ChannelView>> {
    state.node()?.channels().map_err(text)
}

#[tauri::command]
fn join_channel(
    state: State<'_, AppState>,
    name: String,
    password: String,
    nick: String,
) -> CommandResult<ChannelView> {
    state
        .node()?
        .join_channel(&name, &password, &nick)
        .map_err(text)
}

#[tauri::command]
fn leave_channel(state: State<'_, AppState>, channel_id: i64) -> CommandResult<()> {
    state.node()?.leave_channel(channel_id).map_err(text)
}

#[tauri::command]
fn set_channel_nick(
    state: State<'_, AppState>,
    channel_id: i64,
    nick: String,
) -> CommandResult<ChannelView> {
    state
        .node()?
        .set_channel_nick(channel_id, &nick)
        .map_err(text)
}

#[tauri::command]
fn posts(state: State<'_, AppState>, channel_id: i64) -> CommandResult<Vec<PostView>> {
    state.node()?.posts(channel_id).map_err(text)
}

#[tauri::command]
fn send_post(state: State<'_, AppState>, channel_id: i64, text: String) -> CommandResult<PostView> {
    state
        .node()?
        .send_post(channel_id, &text)
        .map_err(self::text)
}

#[tauri::command]
fn share_card(state: State<'_, AppState>, channel_id: i64) -> CommandResult<PostView> {
    state.node()?.share_card(channel_id).map_err(text)
}

#[tauri::command]
fn mute_author(state: State<'_, AppState>, channel_id: i64, author: String) -> CommandResult<()> {
    state.node()?.mute_author(channel_id, &author).map_err(text)
}

#[tauri::command]
fn unmute_author(state: State<'_, AppState>, channel_id: i64, author: String) -> CommandResult<()> {
    state
        .node()?
        .unmute_author(channel_id, &author)
        .map_err(text)
}

#[tauri::command]
fn muted_authors(state: State<'_, AppState>, channel_id: i64) -> CommandResult<Vec<MutedView>> {
    state.node()?.muted_authors(channel_id).map_err(text)
}

#[tauri::command]
fn mark_channel_read(state: State<'_, AppState>, channel_id: i64) -> CommandResult<()> {
    state.node()?.mark_channel_read(channel_id).map_err(text)
}

#[tauri::command]
fn status(state: State<'_, AppState>) -> CommandResult<Status> {
    state.node()?.status().map_err(text)
}

#[tauri::command]
fn export_file(state: State<'_, AppState>, path: PathBuf) -> CommandResult<usize> {
    state.node()?.export_file(&path).map_err(text)
}

#[tauri::command]
fn import_file(state: State<'_, AppState>, path: PathBuf) -> CommandResult<ImportReport> {
    state.node()?.import_file(&path).map_err(text)
}

#[tauri::command]
async fn connect_peer(state: State<'_, AppState>, address: String) -> CommandResult<()> {
    let addr: SocketAddr = address
        .trim()
        .parse()
        .map_err(|_| "Use an address like 192.168.1.20:47500".to_owned())?;
    state.node()?.connect(addr).await.map_err(text)
}

/// Opens the Local Network list in System Settings, where macOS keeps the
/// permission the local network link needs.
#[tauri::command]
fn open_network_settings() -> CommandResult<()> {
    if !cfg!(target_os = "macos") {
        return Err("This setting only exists on macOS".to_owned());
    }
    std::process::Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_LocalNetwork")
        .spawn()
        .map(|_| ())
        .map_err(text)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            // KERVAN_DIR lets two copies run side by side for testing.
            let dir = match std::env::var_os("KERVAN_DIR") {
                Some(dir) => PathBuf::from(dir),
                None => app.path().app_data_dir()?,
            };
            let node = Node::open(&dir)?;
            if let Some(node) = &node {
                launch(app.handle(), node);
            }
            app.manage(AppState {
                dir,
                node: Mutex::new(node),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            profile,
            create_profile,
            contacts,
            add_contact,
            accept_contact,
            decline_contact,
            mark_read,
            rename,
            settings,
            set_read_receipts,
            erase_everything,
            messages,
            send_message,
            groups,
            group_messages,
            create_group,
            add_group_member,
            remove_group_member,
            leave_group,
            send_group_message,
            mark_group_read,
            channels,
            join_channel,
            leave_channel,
            set_channel_nick,
            posts,
            send_post,
            share_card,
            mute_author,
            unmute_author,
            muted_authors,
            mark_channel_read,
            status,
            export_file,
            import_file,
            connect_peer,
            open_network_settings,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Kervan");
}
