//! Android: a foreground service (`CarryChatService`) keeps the process, and
//! the node in it, running after the screen closes, and starts them after
//! the phone starts. These are the Rust pieces of that: one node for the
//! process, opened by the screen or by the service, staying alive without a
//! window, making a window again for a screen that opens later, announcing
//! messages while there is no page to do it, and syncing over the Bluetooth
//! and Wi-Fi Aware connections that the service's `BleLink` and
//! `WifiAwareLink` make (§9.5, §9.6).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

use carrychat_node::{Event, FileView, Link, Node};
use jni::objects::{GlobalRef, JByteArray, JClass, JObject, JString, JValue};
use jni::sys::{jboolean, jbyteArray, jint, JNI_FALSE, JNI_TRUE};
use jni::{JNIEnv, JavaVM};
use tauri::{AppHandle, Manager, RunEvent, WebviewWindowBuilder};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

static APP: OnceLock<AppHandle> = OnceLock::new();

/// The process's node and the folder it keeps its data in.
static NODE: Mutex<Option<Node>> = Mutex::new(None);
static DIR: OnceLock<PathBuf> = OnceLock::new();

/// Whether the links have started; they run once for the process.
static LINKS: AtomicBool = AtomicBool::new(false);

/// The Java side, for posting notifications. Tauri's notification plugin
/// needs a screen, so while there is none they go through `CarryChatService`.
static JAVA: OnceLock<Java> = OnceLock::new();

struct Java {
    vm: JavaVM,
    service: GlobalRef,
}

static NOTIFICATIONS: Mutex<Notifications> = Mutex::new(Notifications {
    on: true,
    turkish: false,
});

/// What the page chose about notifications, kept for when it is gone.
#[derive(Clone, Copy)]
struct Notifications {
    on: bool,
    turkish: bool,
}

pub fn keep(app: &AppHandle) {
    let _ = APP.set(app.clone());
}

/// The node in `dir`, opened unless the service opened it already.
pub fn open(dir: &Path) -> carrychat_node::Result<Option<Node>> {
    let _ = DIR.set(dir.to_owned());
    let mut slot = lock(&NODE);
    if slot.is_none() {
        *slot = Node::open(dir)?;
    }
    Ok(slot.clone())
}

/// A node made on the first run, for the service to find.
pub fn adopt(dir: &Path, node: &Node) {
    let _ = DIR.set(dir.to_owned());
    *lock(&NODE) = Some(node.clone());
}

/// True for the first caller, which then starts the links; from then on the
/// process also announces messages that arrive while no page is open.
pub fn claim_links(node: &Node) -> bool {
    if LINKS.swap(true, Ordering::SeqCst) {
        return false;
    }
    let mut events = node.subscribe();
    let node = node.clone();
    tauri::async_runtime::spawn(async move {
        while let Ok(event) = events.recv().await {
            notify(&node, &event);
        }
    });
    true
}

/// `Core.start(dir)`, which the service calls as it starts, also after the
/// phone starts with no screen at all. Returns whether a node runs: false
/// before an identity exists.
#[no_mangle]
pub extern "system" fn Java_dev_kemalarslan_carrychat_Core_start(
    mut env: JNIEnv,
    _class: JClass,
    dir: JString,
) -> jboolean {
    remember_service(&mut env);
    let Ok(dir) = env
        .get_string(&dir)
        .map(|dir| PathBuf::from(String::from(dir)))
    else {
        return JNI_FALSE;
    };
    let node = match open(&dir) {
        Ok(Some(node)) => node,
        Ok(None) => return JNI_FALSE,
        Err(error) => {
            eprintln!("Could not open the node: {error}");
            return JNI_FALSE;
        }
    };
    load_notifications(&dir);
    if claim_links(&node) {
        crate::start_links(&node, &dir, |error| eprintln!("{error}"));
    }
    JNI_TRUE
}

/// Finds `CarryChatService`, which posts notifications. The app's classes can
/// only be looked up from a thread Java started.
fn remember_service(env: &mut JNIEnv) {
    if JAVA.get().is_some() {
        return;
    }
    let java = env.get_java_vm().and_then(|vm| {
        let class = env.find_class("dev/kemalarslan/carrychat/CarryChatService")?;
        Ok(Java {
            vm,
            service: env.new_global_ref(class)?,
        })
    });
    match java {
        Ok(java) => {
            let _ = JAVA.set(java);
        }
        Err(error) => {
            let _ = env.exception_clear();
            eprintln!("Could not reach the service: {error}");
        }
    }
}

/// Closing the screen closes the last window, which would end the process
/// and the node with it; the service is there to keep both running.
pub fn on_run_event(event: RunEvent) {
    if let RunEvent::ExitRequested {
        code: None, api, ..
    } = event
    {
        api.prevent_exit();
    }
}

pub fn set_notifications(on: bool, language: &str) {
    let settings = Notifications {
        on,
        turkish: language == "tr",
    };
    *NOTIFICATIONS.lock().unwrap_or_else(PoisonError::into_inner) = settings;
    // Kept for after the phone starts, when there is no page to say it.
    if let Some(dir) = DIR.get() {
        let line = format!(
            "{} {}",
            if settings.on { "on" } else { "off" },
            if settings.turkish { "tr" } else { "en" }
        );
        if let Err(error) = std::fs::write(dir.join(NOTIFICATIONS_FILE), line) {
            eprintln!("Could not keep the notification setting: {error}");
        }
    }
}

const NOTIFICATIONS_FILE: &str = "notifications";

fn load_notifications(dir: &Path) {
    let Ok(line) = std::fs::read_to_string(dir.join(NOTIFICATIONS_FILE)) else {
        return;
    };
    let mut words = line.split_whitespace();
    *NOTIFICATIONS.lock().unwrap_or_else(PoisonError::into_inner) = Notifications {
        on: words.next() != Some("off"),
        turkish: words.next() == Some("tr"),
    };
}

/// `MainActivity.attachScreen`, called as each screen is created. The first
/// one gets its window from setup; one that opens in a process that outlived
/// its screen gets the window the configuration describes.
#[no_mangle]
pub extern "system" fn Java_dev_kemalarslan_carrychat_MainActivity_attachScreen(
    mut env: JNIEnv,
    _activity: JObject,
) {
    remember_service(&mut env);
    let Some(app) = APP.get() else {
        return;
    };
    if !app.webview_windows().is_empty() {
        return;
    }
    // The window is made on the event loop: waiting for it here, on the
    // Android UI thread that the web view is created on, would never end.
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let Some(config) = handle.config().app.windows.first().cloned() else {
            return;
        };
        if let Err(error) =
            WebviewWindowBuilder::from_config(&handle, &config).and_then(|window| window.build())
        {
            eprintln!("Could not open the window again: {error}");
        }
    });
}

/// Announces a message while no page is open; an open page does it itself.
fn notify(node: &Node, event: &Event) {
    let settings = *NOTIFICATIONS.lock().unwrap_or_else(PoisonError::into_inner);
    if !settings.on
        || APP
            .get()
            .is_some_and(|app| !app.webview_windows().is_empty())
    {
        return;
    }
    let (title, body) = match event {
        Event::MessageReceived { message } => {
            // A first message from someone new is announced by its request.
            let Some(contact) = node
                .contacts()
                .ok()
                .and_then(|contacts| contacts.into_iter().find(|c| c.id == message.contact_id))
                .filter(|contact| !contact.request)
            else {
                return;
            };
            (
                contact.name,
                body(&message.body, message.file.as_ref(), settings),
            )
        }
        Event::ContactRequest { contact } => (
            if settings.turkish {
                format!("{} sana yazmak istiyor.", contact.name)
            } else {
                format!("{} wants to message you.", contact.name)
            },
            String::new(),
        ),
        Event::GroupMessageReceived { message } => {
            let Some(group) = node
                .groups()
                .ok()
                .and_then(|groups| groups.into_iter().find(|g| g.id == message.group_id))
            else {
                return;
            };
            let sender = message.sender.as_deref().unwrap_or_default();
            (
                format!("{sender} · {}", group.name),
                body(&message.body, message.file.as_ref(), settings),
            )
        }
        _ => return,
    };
    if let Err(error) = post(&title, &body) {
        eprintln!("Could not show a notification: {error}");
    }
}

/// `CarryChatService.notifyMessage(title, body)`.
fn post(title: &str, body: &str) -> jni::errors::Result<()> {
    let Some(java) = JAVA.get() else {
        return Ok(());
    };
    let mut env = java.vm.attach_current_thread_as_daemon()?;
    let title = env.new_string(title)?;
    let body = env.new_string(body)?;
    let class: &JClass = java.service.as_obj().into();
    let posted = env.call_static_method(
        class,
        "notifyMessage",
        "(Ljava/lang/String;Ljava/lang/String;)V",
        &[JValue::Object(&title), JValue::Object(&body)],
    );
    if posted.is_err() {
        // A Java exception stays pending on the thread until it is cleared.
        let _ = env.exception_clear();
    }
    posted.map(drop)
}

/// A message's text, or what kind of file it brings.
fn body(text: &str, file: Option<&FileView>, settings: Notifications) -> String {
    let Some(file) = file.filter(|_| text.is_empty()) else {
        return text.to_owned();
    };
    let (en, tr) = if file.mime.starts_with("image/") {
        ("Photo", "Fotoğraf")
    } else if file.mime.starts_with("audio/") {
        ("Voice message", "Sesli mesaj")
    } else {
        ("File", "Dosya")
    };
    if settings.turkish { tr } else { en }.to_owned()
}

/// How Wi-Fi Aware stands, as `WifiAwareLink` last said: 0 unsupported, 1 off, 2 on.
static WIFI_AWARE: AtomicI32 = AtomicI32::new(0);

pub fn wifi_aware() -> &'static str {
    match WIFI_AWARE.load(Ordering::Relaxed) {
        2 => "on",
        1 => "off",
        _ => "unsupported",
    }
}

/// `Aware.state(state)`: Wi-Fi Aware became available or went away.
#[no_mangle]
pub extern "system" fn Java_dev_kemalarslan_carrychat_Aware_state(
    _env: JNIEnv,
    _class: JClass,
    state: jint,
) {
    WIFI_AWARE.store(state, Ordering::Relaxed);
}

/// `Ble.state(on)`: whether the phone's Bluetooth is on, every few seconds.
#[no_mangle]
pub extern "system" fn Java_dev_kemalarslan_carrychat_Ble_state(
    _env: JNIEnv,
    _class: JClass,
    on: jboolean,
) {
    if let Some(node) = lock(&NODE).clone() {
        node.set_bluetooth_radio(on == JNI_TRUE);
    }
}

/// The node, while it uses Bluetooth.
static BLUETOOTH: Mutex<Option<Node>> = Mutex::new(None);

/// The Kotlin classes that move a link's bytes: `Ble` and `Aware`.
static BLE: OnceLock<(JavaVM, GlobalRef)> = OnceLock::new();
static AWARE: OnceLock<(JavaVM, GlobalRef)> = OnceLock::new();

/// Each open connection's incoming bytes, by its bridge and the id Kotlin gave it.
static CONNECTIONS: Mutex<Option<HashMap<(Bridge, i32), mpsc::UnboundedSender<Vec<u8>>>>> =
    Mutex::new(None);

/// Bytes the stream hands Kotlin at a time; it cuts them to the connection's MTU.
const SEND_LEN: usize = 4096;
const BUFFER_LEN: usize = 64 * 1024;

/// A platform link whose connections Kotlin makes: Bluetooth LE (§9.5) or
/// Wi-Fi Aware (§9.6). Both carry the sync stream the same way.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Bridge {
    Ble,
    Aware,
}

impl Bridge {
    fn class(self) -> &'static OnceLock<(JavaVM, GlobalRef)> {
        match self {
            Bridge::Ble => &BLE,
            Bridge::Aware => &AWARE,
        }
    }

    fn link(self) -> Link {
        match self {
            Bridge::Ble => Link::Bluetooth,
            Bridge::Aware => Link::WifiAware,
        }
    }

    /// The node, while it uses this link. Wi-Fi Aware has no switch of its own.
    fn node(self) -> Option<Node> {
        match self {
            Bridge::Ble => lock(&BLUETOOTH).clone(),
            Bridge::Aware => lock(&NODE).clone(),
        }
    }
}

pub fn set_bluetooth(node: &Node, on: bool) {
    *lock(&BLUETOOTH) = on.then(|| node.clone());
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn session_of(env: &JNIEnv, bytes: &JByteArray) -> Option<u64> {
    if bytes.is_null() {
        return None;
    }
    let bytes = env.convert_byte_array(bytes).ok()?;
    Some(u64::from_be_bytes(bytes.try_into().ok()?))
}

/// The session id to announce, or null while the node does not use the link.
fn session(bridge: Bridge, env: &JNIEnv, class: &JClass) -> jbyteArray {
    if bridge.class().get().is_none() {
        if let (Ok(vm), Ok(class)) = (env.get_java_vm(), env.new_global_ref(class)) {
            let _ = bridge.class().set((vm, class));
        }
    }
    match bridge.node() {
        Some(node) => env
            .byte_array_from_slice(&node.session().to_be_bytes())
            .map(JByteArray::into_raw)
            .unwrap_or(std::ptr::null_mut()),
        None => std::ptr::null_mut(),
    }
}

/// Whether to connect to a node seen announcing itself.
fn should_connect(bridge: Bridge, env: &JNIEnv, peer: &JByteArray) -> jboolean {
    let yes = bridge
        .node()
        .zip(session_of(env, peer))
        .is_some_and(|(node, peer)| node.should_connect(peer));
    if yes {
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}

/// A connection to that node could not be made.
fn dial_ended(bridge: Bridge, env: &JNIEnv, peer: &JByteArray) {
    if let (Some(node), Some(peer)) = (bridge.node(), session_of(env, peer)) {
        node.dial_ended(peer);
    }
}

/// A connection is ready to carry the stream. `peer` is the session id of
/// the node this side connected to, null if the other side connected.
fn opened(bridge: Bridge, env: &JNIEnv, id: jint, peer: &JByteArray) {
    let Some(node) = bridge.node() else {
        close(bridge, id);
        return;
    };
    let dialed = session_of(env, peer);
    let (incoming, mut received) = mpsc::unbounded_channel::<Vec<u8>>();
    lock(&CONNECTIONS)
        .get_or_insert_with(HashMap::new)
        .insert((bridge, id), incoming);
    let (sync_end, link_end) = tokio::io::duplex(BUFFER_LEN);
    let (sync_reader, sync_writer) = tokio::io::split(sync_end);
    let (mut link_reader, mut link_writer) = tokio::io::split(link_end);
    tauri::async_runtime::spawn(async move {
        let incoming = tauri::async_runtime::spawn(async move {
            while let Some(bytes) = received.recv().await {
                if link_writer.write_all(&bytes).await.is_err() {
                    break;
                }
            }
            // The connection closed: the sync reads the end of the stream.
            let _ = link_writer.shutdown().await;
        });
        let outgoing = tauri::async_runtime::spawn(async move {
            let mut buffer = vec![0; SEND_LEN];
            while let Ok(len @ 1..) = link_reader.read(&mut buffer).await {
                if let Err(error) = send(bridge, id, &buffer[..len]) {
                    eprintln!("Could not send over a nearby link: {error}");
                    break;
                }
            }
        });
        node.sync_over(bridge.link(), sync_reader, sync_writer, dialed)
            .await;
        incoming.abort();
        outgoing.abort();
        forget(bridge, id);
        close(bridge, id);
    });
}

/// Bytes of a connection's stream.
fn received(bridge: Bridge, env: &JNIEnv, id: jint, data: &JByteArray) {
    let Ok(bytes) = env.convert_byte_array(data) else {
        return;
    };
    if let Some(incoming) = lock(&CONNECTIONS)
        .as_ref()
        .and_then(|map| map.get(&(bridge, id)))
    {
        let _ = incoming.send(bytes);
    }
}

fn forget(bridge: Bridge, id: i32) {
    if let Some(map) = lock(&CONNECTIONS).as_mut() {
        map.remove(&(bridge, id));
    }
}

/// `Ble.send(id, data)` or `Aware.send(id, data)`.
fn send(bridge: Bridge, id: i32, data: &[u8]) -> jni::errors::Result<()> {
    let Some((vm, class)) = bridge.class().get() else {
        return Ok(());
    };
    let mut env = vm.attach_current_thread_as_daemon()?;
    let data = env.byte_array_from_slice(data)?;
    let class: &JClass = class.as_obj().into();
    let sent = env.call_static_method(
        class,
        "send",
        "(I[B)V",
        &[JValue::Int(id), JValue::Object(&data)],
    );
    if sent.is_err() {
        let _ = env.exception_clear();
    }
    sent.map(drop)
}

/// `Ble.close(id)` or `Aware.close(id)`.
fn close(bridge: Bridge, id: i32) {
    let Some((vm, class)) = bridge.class().get() else {
        return;
    };
    let Ok(mut env) = vm.attach_current_thread_as_daemon() else {
        return;
    };
    let class: &JClass = class.as_obj().into();
    if env
        .call_static_method(class, "close", "(I)V", &[JValue::Int(id)])
        .is_err()
    {
        let _ = env.exception_clear();
    }
}

/// The JNI entry points of one bridge's Kotlin class.
macro_rules! bridge_entry_points {
    ($bridge:expr, $session:ident, $should_connect:ident, $dial_ended:ident, $opened:ident, $received:ident, $closed:ident) => {
        #[no_mangle]
        pub extern "system" fn $session(env: JNIEnv, class: JClass) -> jbyteArray {
            session($bridge, &env, &class)
        }

        #[no_mangle]
        pub extern "system" fn $should_connect(
            env: JNIEnv,
            _class: JClass,
            peer: JByteArray,
        ) -> jboolean {
            should_connect($bridge, &env, &peer)
        }

        #[no_mangle]
        pub extern "system" fn $dial_ended(env: JNIEnv, _class: JClass, peer: JByteArray) {
            dial_ended($bridge, &env, &peer)
        }

        #[no_mangle]
        pub extern "system" fn $opened(env: JNIEnv, _class: JClass, id: jint, peer: JByteArray) {
            opened($bridge, &env, id, &peer)
        }

        #[no_mangle]
        pub extern "system" fn $received(env: JNIEnv, _class: JClass, id: jint, data: JByteArray) {
            received($bridge, &env, id, &data)
        }

        #[no_mangle]
        pub extern "system" fn $closed(_env: JNIEnv, _class: JClass, id: jint) {
            forget($bridge, id)
        }
    };
}

bridge_entry_points!(
    Bridge::Ble,
    Java_dev_kemalarslan_carrychat_Ble_session,
    Java_dev_kemalarslan_carrychat_Ble_shouldConnect,
    Java_dev_kemalarslan_carrychat_Ble_dialEnded,
    Java_dev_kemalarslan_carrychat_Ble_opened,
    Java_dev_kemalarslan_carrychat_Ble_received,
    Java_dev_kemalarslan_carrychat_Ble_closed
);

bridge_entry_points!(
    Bridge::Aware,
    Java_dev_kemalarslan_carrychat_Aware_session,
    Java_dev_kemalarslan_carrychat_Aware_shouldConnect,
    Java_dev_kemalarslan_carrychat_Aware_dialEnded,
    Java_dev_kemalarslan_carrychat_Aware_opened,
    Java_dev_kemalarslan_carrychat_Aware_received,
    Java_dev_kemalarslan_carrychat_Aware_closed
);
