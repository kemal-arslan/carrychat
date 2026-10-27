//! The app: a thin layer of commands over a [`Node`]. Node events are
//! forwarded to the window as `carrychat-event`.

use std::collections::BTreeMap;
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use carrychat_node::{
    ChannelView, ContactView, GroupMessageView, GroupView, HeardCard, ImportReport, InternetConfig,
    Journey, LanConfig, LoraDevice, LoraPort, LoraStatus, MessageView, MutedView, NearbyPerson,
    Node, PostView, Profile, ReticulumStatus, Settings, SoundBand, SoundListener, Status,
};
use data_encoding::BASE64;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_fs::{FilePath, FsExt, OpenOptions};

#[cfg(target_os = "android")]
mod android;

struct AppState {
    dir: PathBuf,
    node: Mutex<Option<Node>>,
    /// What the page listens to the microphone for: a card played as sound,
    /// or messages.
    listenings: Mutex<Listenings>,
}

/// Listenings kept at most. A page that went away without ending its
/// listenings leaves them behind; the oldest go first.
const MAX_LISTENINGS: usize = 4;

/// The page's listenings to the microphone, by the number it knows each
/// by. A card listening and a message listening may overlap, and one that
/// the page started or ended late must not end another.
#[derive(Default)]
struct Listenings {
    last: u32,
    listeners: BTreeMap<u32, SoundListener>,
}

impl Listenings {
    fn start(&mut self, rate: u32) -> u32 {
        self.last += 1;
        self.listeners.insert(self.last, SoundListener::new(rate));
        while self.listeners.len() > MAX_LISTENINGS {
            self.listeners.pop_first();
        }
        self.last
    }

    fn get(&mut self, listening: u32) -> Option<&mut SoundListener> {
        self.listeners.get_mut(&listening)
    }

    fn stop(&mut self, listening: u32) {
        self.listeners.remove(&listening);
    }
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

/// Starts the local network, internet and Bluetooth links. `report` hears
/// what could not start.
pub(crate) fn start_links(node: &Node, dir: &Path, report: impl Fn(String) + Send + 'static) {
    let settings = node.settings().ok();
    // Tor keeps its state next to the node's, and is used if chosen.
    let internet = InternetConfig {
        tor: settings
            .as_ref()
            .is_some_and(|settings| settings.tor)
            .then(|| dir.join("tor")),
        ..InternetConfig::default()
    };
    let bluetooth = settings.as_ref().is_none_or(|settings| settings.bluetooth);
    let lora = settings
        .as_ref()
        .and_then(|settings| settings.lora_device.as_ref())
        .and_then(|device| device.parse::<LoraDevice>().ok());
    let reticulum = settings.and_then(|settings| settings.reticulum);
    let node = node.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = node.start(LanConfig::default()).await {
            report(format!("Local network link is off: {error}"));
        }
        if let Err(error) = node.start_internet(internet) {
            report(format!("Internet link is off: {error}"));
        }
        // On Android the service runs Bluetooth, and asks the node for it.
        #[cfg(not(target_os = "android"))]
        if bluetooth {
            if let Err(error) = node.start_bluetooth().await {
                report(format!("Bluetooth is off: {error}"));
            }
        }
        #[cfg(target_os = "android")]
        android::set_bluetooth(&node, bluetooth);
        // On Android the service connects a Bluetooth device, once chosen.
        if let Some(device) = lora {
            if let Err(error) = node.start_lora(device) {
                report(format!("LoRa is off: {error}"));
            }
        }
        if let Some(address) = reticulum {
            if let Err(error) = node.start_reticulum(&address) {
                report(format!("Reticulum is off: {error}"));
            }
        }
    });
}

/// Forwards node events to the window, and starts the links unless they
/// already run: on Android the service may have started them at boot.
fn launch(app: &AppHandle, node: &Node, dir: &Path) {
    let mut events = node.subscribe();
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        while let Ok(event) = events.recv().await {
            let _ = handle.emit("carrychat-event", event);
        }
    });
    #[cfg(target_os = "android")]
    if !android::claim_links(node) {
        return;
    }
    let handle = app.clone();
    start_links(node, dir, move |error| {
        let _ = handle.emit("carrychat-error", error);
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
    #[cfg(target_os = "android")]
    android::adopt(&state.dir, &node);
    launch(&app, &node, &state.dir);
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

/// The band sound plays in: above hearing when `inaudible`.
fn sound_band(inaudible: bool) -> SoundBand {
    if inaudible {
        SoundBand::Inaudible
    } else {
        SoundBand::Audible
    }
}

/// One of our messages as sound (§9.10): a WAV file the page plays a few
/// times for a device nearby that listens.
#[tauri::command]
async fn message_sound(
    state: State<'_, AppState>,
    message_id: i64,
    inaudible: bool,
) -> CommandResult<tauri::ipc::Response> {
    let wav = state
        .node()?
        .message_sound(message_id, sound_band(inaudible))
        .map_err(text)?;
    Ok(tauri::ipc::Response::new(wav))
}

/// This device's card as sound (§3.2): one round, a WAV file the page
/// plays over and over.
#[tauri::command]
async fn card_sound(
    state: State<'_, AppState>,
    inaudible: bool,
) -> CommandResult<tauri::ipc::Response> {
    let wav = state
        .node()?
        .card_sound(sound_band(inaudible))
        .map_err(text)?;
    Ok(tauri::ipc::Response::new(wav))
}

/// Starts listening to a microphone giving `rate` samples a second, for a
/// card played as sound or for messages. Returns the listening's number,
/// which the other calls name.
#[tauri::command]
fn listen_start(state: State<'_, AppState>, rate: u32) -> u32 {
    state
        .listenings
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .start(rate)
}

/// The microphone's next samples in the request's body: the listening's
/// number as a little-endian `u32`, then 16-bit little-endian samples.
/// Messages heard go to the node; a card is returned once heard whole.
#[tauri::command]
async fn listen_heard(
    state: State<'_, AppState>,
    request: tauri::ipc::Request<'_>,
) -> CommandResult<Option<HeardCard>> {
    let bytes: Vec<u8> = match request.body() {
        tauri::ipc::InvokeBody::Raw(bytes) => bytes.clone(),
        // Android sends every request as JSON, the bytes as an array of numbers.
        tauri::ipc::InvokeBody::Json(serde_json::Value::Array(values)) => values
            .iter()
            .map(|value| value.as_u64().and_then(|byte| u8::try_from(byte).ok()))
            .collect::<Option<_>>()
            .ok_or("expected samples")?,
        tauri::ipc::InvokeBody::Json(_) => return Err("expected samples".to_owned()),
    };
    let Some((number, samples)) = bytes.split_first_chunk::<4>() else {
        return Err("expected samples".to_owned());
    };
    let samples: Vec<f32> = samples
        .chunks_exact(2)
        .map(|pair| f32::from(i16::from_le_bytes([pair[0], pair[1]])) / 32768.0)
        .collect();
    let node = state.node()?;
    let mut listenings = state
        .listenings
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    Ok(listenings
        .get(u32::from_le_bytes(*number))
        .and_then(|listener| listener.push(&node, &samples)))
}

/// A card read over NFC (§3.3), in its text form, once its signature
/// verifies.
#[tauri::command]
fn read_card(card: String) -> CommandResult<HeardCard> {
    HeardCard::from_text(&card).ok_or_else(|| "The card did not read right".to_owned())
}

/// Ends listening `listening`, unless another started since.
#[tauri::command]
fn listen_stop(state: State<'_, AppState>, listening: u32) {
    state
        .listenings
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .stop(listening);
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
fn set_nearby(state: State<'_, AppState>, on: bool) -> CommandResult<()> {
    state.node()?.set_nearby(on).map_err(text)
}

#[tauri::command]
fn nearby(state: State<'_, AppState>) -> CommandResult<Vec<NearbyPerson>> {
    Ok(state.node()?.nearby())
}

/// How Wi-Fi Aware stands on this phone: "unsupported", "off" or "on".
/// Elsewhere there is none.
#[tauri::command]
fn wifi_aware() -> Option<&'static str> {
    #[cfg(target_os = "android")]
    return Some(android::wifi_aware());
    #[cfg(not(target_os = "android"))]
    None
}

/// How Wi-Fi Direct stands on this phone: "unsupported", "off" or "on".
/// Elsewhere there is none.
#[tauri::command]
fn wifi_direct() -> Option<&'static str> {
    #[cfg(target_os = "android")]
    return Some(android::wifi_direct());
    #[cfg(not(target_os = "android"))]
    None
}

#[tauri::command]
fn lora(state: State<'_, AppState>) -> CommandResult<LoraStatus> {
    Ok(state.node()?.lora())
}

/// Uses a Meshtastic or MeshCore device for LoRa, given as `tcp:host:port`, a serial
/// port or `bluetooth:address`, or none.
#[tauri::command]
fn set_lora(state: State<'_, AppState>, device: Option<String>) -> CommandResult<()> {
    let device = device
        .map(|device| device.trim().parse::<LoraDevice>())
        .transpose()
        .map_err(text)?;
    state.node()?.set_lora(device).map_err(text)
}

/// USB serial ports a LoRa device may be on. Phones connect over
/// Bluetooth instead.
#[tauri::command]
fn lora_ports() -> Vec<LoraPort> {
    #[cfg(not(target_os = "android"))]
    return carrychat_node::lora::ports();
    #[cfg(target_os = "android")]
    Vec::new()
}

/// A Meshtastic or MeshCore device found over Bluetooth.
#[derive(Clone, serde::Serialize)]
pub(crate) struct FoundDevice {
    pub address: String,
    pub name: Option<String>,
}

/// Starts looking for Meshtastic and MeshCore devices over Bluetooth, on a
/// phone.
#[tauri::command]
fn lora_scan() {
    #[cfg(target_os = "android")]
    android::lora_scan();
}

/// The LoRa devices found since the last `lora_scan`.
#[tauri::command]
fn lora_found() -> Vec<FoundDevice> {
    #[cfg(target_os = "android")]
    return android::lora_found();
    #[cfg(not(target_os = "android"))]
    Vec::new()
}

#[tauri::command]
fn reticulum(state: State<'_, AppState>) -> CommandResult<ReticulumStatus> {
    Ok(state.node()?.reticulum())
}

/// Connects to a Reticulum node, given as `host:port`, or to none.
#[tauri::command]
fn set_reticulum(state: State<'_, AppState>, address: Option<String>) -> CommandResult<()> {
    state
        .node()?
        .set_reticulum(address.as_deref())
        .map_err(text)
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
    carrychat_node::erase(&state.dir).map_err(text)?;
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
fn journey(state: State<'_, AppState>, message_id: i64) -> CommandResult<Journey> {
    state.node()?.journey(message_id).map_err(text)
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

/// Where a file goes: a contact, a group or a channel.
#[derive(serde::Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "camelCase")]
enum FileTarget {
    Contact(i64),
    Group(i64),
    Channel(i64),
}

/// Sends a file, given in base64. Returns the message, group message or
/// post that offers it.
#[tauri::command]
fn send_file(
    state: State<'_, AppState>,
    target: FileTarget,
    mime: String,
    data: String,
) -> CommandResult<serde_json::Value> {
    let data = BASE64.decode(data.as_bytes()).map_err(text)?;
    let node = state.node()?;
    let sent = match target {
        FileTarget::Contact(id) => {
            serde_json::to_value(node.send_file(id, &mime, &data).map_err(text)?)
        }
        FileTarget::Group(id) => {
            serde_json::to_value(node.send_group_file(id, &mime, &data).map_err(text)?)
        }
        FileTarget::Channel(id) => {
            serde_json::to_value(node.send_channel_file(id, &mime, &data).map_err(text)?)
        }
    };
    sent.map_err(text)
}

/// A file that arrived, in base64, or `None` while parts are missing.
#[tauri::command]
fn file_data(state: State<'_, AppState>, file_id: i64) -> CommandResult<Option<FileData>> {
    Ok(state
        .node()?
        .file_data(file_id)
        .map_err(text)?
        .map(|(mime, data)| FileData {
            mime,
            data: BASE64.encode(&data),
        }))
}

/// Writes a file that arrived to a path the user chose.
#[tauri::command]
fn save_file(
    app: AppHandle,
    state: State<'_, AppState>,
    file_id: i64,
    path: FilePath,
) -> CommandResult<()> {
    let (_, data) = state
        .node()?
        .file_data(file_id)
        .map_err(text)?
        .ok_or_else(|| "the file is not all here yet".to_owned())?;
    write_to(&app, path, &data)
}

/// Writes to a chosen file. On Android the file picker gives a content URI
/// rather than a path, which only the fs plugin can open.
fn write_to(app: &AppHandle, path: FilePath, data: &[u8]) -> CommandResult<()> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    let mut file = app.fs().open(path, options).map_err(text)?;
    file.write_all(data).map_err(text)
}

#[derive(serde::Serialize)]
struct FileData {
    mime: String,
    data: String,
}

/// Chooses whether the internet link goes over Tor, from the next start.
#[tauri::command]
fn set_tor(state: State<'_, AppState>, on: bool) -> CommandResult<()> {
    state.node()?.set_tor(on).map_err(text)
}

/// Chooses whether the app uses Bluetooth, from its next start.
#[tauri::command]
fn set_bluetooth(state: State<'_, AppState>, on: bool) -> CommandResult<()> {
    state.node()?.set_bluetooth(on).map_err(text)
}

/// The page's notification setting and language, for the notifications
/// Android shows while the page is closed.
#[tauri::command]
fn set_notifications(on: bool, language: String) {
    #[cfg(target_os = "android")]
    android::set_notifications(on, &language);
    #[cfg(not(target_os = "android"))]
    let _ = (on, language);
}

/// Starts the app again, for a setting that applies at start.
#[tauri::command]
fn restart(app: AppHandle) {
    app.restart()
}

#[tauri::command]
fn status(state: State<'_, AppState>) -> CommandResult<Status> {
    state.node()?.status().map_err(text)
}

#[tauri::command]
fn export_file(app: AppHandle, state: State<'_, AppState>, path: FilePath) -> CommandResult<usize> {
    let (data, count) = state.node()?.export_bytes().map_err(text)?;
    write_to(&app, path, &data)?;
    Ok(count)
}

#[tauri::command]
fn import_file(
    app: AppHandle,
    state: State<'_, AppState>,
    path: FilePath,
) -> CommandResult<ImportReport> {
    let data = app.fs().read(path).map_err(text)?;
    state.node()?.import_bytes(&data).map_err(text)
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
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            // CARRYCHAT_DIR lets two copies run side by side for testing.
            let dir = match std::env::var_os("CARRYCHAT_DIR") {
                Some(dir) => PathBuf::from(dir),
                None => app.path().app_data_dir()?,
            };
            #[cfg(target_os = "android")]
            android::keep(app.handle());
            // On Android the service may have opened it already, at boot.
            #[cfg(target_os = "android")]
            let node = android::open(&dir)?;
            #[cfg(not(target_os = "android"))]
            let node = Node::open(&dir)?;
            if let Some(node) = &node {
                launch(app.handle(), node, &dir);
            }
            app.manage(AppState {
                dir,
                node: Mutex::new(node),
                listenings: Mutex::default(),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            profile,
            create_profile,
            contacts,
            add_contact,
            card_sound,
            message_sound,
            listen_start,
            listen_heard,
            listen_stop,
            read_card,
            accept_contact,
            decline_contact,
            mark_read,
            rename,
            settings,
            set_read_receipts,
            set_nearby,
            nearby,
            wifi_aware,
            wifi_direct,
            lora,
            set_lora,
            lora_ports,
            lora_scan,
            lora_found,
            reticulum,
            set_reticulum,
            erase_everything,
            messages,
            journey,
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
            send_file,
            file_data,
            save_file,
            set_tor,
            set_bluetooth,
            set_notifications,
            restart,
            status,
            export_file,
            import_file,
            connect_peer,
            open_network_settings,
        ])
        .build(tauri::generate_context!())
        .expect("error while building CarryChat")
        .run(|_app, _event| {
            #[cfg(target_os = "android")]
            android::on_run_event(_event);
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_listening_ended_late_leaves_the_others_alone() {
        let mut listenings = Listenings::default();
        let first = listenings.start(48_000);
        let second = listenings.start(48_000);
        listenings.stop(first);
        assert!(listenings.get(first).is_none());
        assert!(listenings.get(second).is_some());
        // Listenings never ended leave only the latest few.
        for _ in 0..10 {
            listenings.start(48_000);
        }
        assert_eq!(listenings.listeners.len(), MAX_LISTENINGS);
        assert!(listenings.get(second).is_none());
    }
}
