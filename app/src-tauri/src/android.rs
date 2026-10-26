//! Android: a foreground service (`CarryChatService`) keeps the process, and
//! the node in it, running after the screen closes, and starts them after
//! the phone starts. These are the Rust pieces of that: one node for the
//! process, opened by the screen or by the service, staying alive without a
//! window, making a window again for a screen that opens later, announcing
//! messages while there is no page to do it, and syncing over the Bluetooth
//! connections that the service's `BleLink` makes (§9.5).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
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

/// The node, while it uses Bluetooth.
static BLUETOOTH: Mutex<Option<Node>> = Mutex::new(None);

/// The Kotlin `Ble` class, which sends and closes connections.
static BLE: OnceLock<(JavaVM, GlobalRef)> = OnceLock::new();

/// Each open connection's incoming bytes, by the id Kotlin gave it.
static CONNECTIONS: Mutex<Option<HashMap<i32, mpsc::UnboundedSender<Vec<u8>>>>> = Mutex::new(None);

/// Bytes the stream hands Kotlin at a time; it cuts them to the connection's MTU.
const SEND_LEN: usize = 4096;
const BUFFER_LEN: usize = 64 * 1024;

pub fn set_bluetooth(node: &Node, on: bool) {
    *lock(&BLUETOOTH) = on.then(|| node.clone());
}

fn bluetooth() -> Option<Node> {
    lock(&BLUETOOTH).clone()
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

/// `Ble.session()`: the session id to advertise, or null while the node does
/// not use Bluetooth.
#[no_mangle]
pub extern "system" fn Java_dev_kemalarslan_carrychat_Ble_session(
    env: JNIEnv,
    class: JClass,
) -> jbyteArray {
    if BLE.get().is_none() {
        if let (Ok(vm), Ok(class)) = (env.get_java_vm(), env.new_global_ref(&class)) {
            let _ = BLE.set((vm, class));
        }
    }
    match bluetooth() {
        Some(node) => env
            .byte_array_from_slice(&node.session().to_be_bytes())
            .map(JByteArray::into_raw)
            .unwrap_or(std::ptr::null_mut()),
        None => std::ptr::null_mut(),
    }
}

/// `Ble.shouldConnect(peer)`: whether to connect to a node seen advertising.
#[no_mangle]
pub extern "system" fn Java_dev_kemalarslan_carrychat_Ble_shouldConnect(
    env: JNIEnv,
    _class: JClass,
    peer: JByteArray,
) -> jboolean {
    let yes = bluetooth()
        .zip(session_of(&env, &peer))
        .is_some_and(|(node, peer)| node.should_connect(peer));
    if yes {
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}

/// `Ble.dialEnded(peer)`: a connection to that node could not be made.
#[no_mangle]
pub extern "system" fn Java_dev_kemalarslan_carrychat_Ble_dialEnded(
    env: JNIEnv,
    _class: JClass,
    peer: JByteArray,
) {
    if let (Some(node), Some(peer)) = (bluetooth(), session_of(&env, &peer)) {
        node.dial_ended(peer);
    }
}

/// `Ble.opened(id, peer)`: a connection is ready to carry the stream. `peer`
/// is the session id of the node this side connected to, null if the other
/// side connected.
#[no_mangle]
pub extern "system" fn Java_dev_kemalarslan_carrychat_Ble_opened(
    env: JNIEnv,
    _class: JClass,
    id: jint,
    peer: JByteArray,
) {
    let Some(node) = bluetooth() else {
        close(id);
        return;
    };
    let dialed = session_of(&env, &peer);
    let (incoming, mut received) = mpsc::unbounded_channel::<Vec<u8>>();
    lock(&CONNECTIONS)
        .get_or_insert_with(HashMap::new)
        .insert(id, incoming);
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
                if let Err(error) = send(id, &buffer[..len]) {
                    eprintln!("Could not send over Bluetooth: {error}");
                    break;
                }
            }
        });
        node.sync_over(Link::Bluetooth, sync_reader, sync_writer, dialed)
            .await;
        incoming.abort();
        outgoing.abort();
        forget(id);
        close(id);
    });
}

/// `Ble.received(id, data)`: bytes of a connection's stream.
#[no_mangle]
pub extern "system" fn Java_dev_kemalarslan_carrychat_Ble_received(
    env: JNIEnv,
    _class: JClass,
    id: jint,
    data: JByteArray,
) {
    let Ok(bytes) = env.convert_byte_array(&data) else {
        return;
    };
    if let Some(incoming) = lock(&CONNECTIONS).as_ref().and_then(|map| map.get(&id)) {
        let _ = incoming.send(bytes);
    }
}

/// `Ble.closed(id)`: the connection is gone.
#[no_mangle]
pub extern "system" fn Java_dev_kemalarslan_carrychat_Ble_closed(
    _env: JNIEnv,
    _class: JClass,
    id: jint,
) {
    forget(id);
}

fn forget(id: i32) {
    if let Some(map) = lock(&CONNECTIONS).as_mut() {
        map.remove(&id);
    }
}

/// `Ble.send(id, data)`.
fn send(id: i32, data: &[u8]) -> jni::errors::Result<()> {
    let Some((vm, class)) = BLE.get() else {
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

/// `Ble.close(id)`.
fn close(id: i32) {
    let Some((vm, class)) = BLE.get() else {
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
