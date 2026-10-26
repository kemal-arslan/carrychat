//! Android: a foreground service (`KervanService`) keeps the process, and
//! the node in it, running after the screen closes. These are the Rust
//! pieces of that: staying alive without a window, making a window again
//! for a screen that opens later, and announcing messages while there is
//! no page to do it.

use std::sync::{Mutex, OnceLock, PoisonError};

use jni::objects::{GlobalRef, JClass, JObject, JValue};
use jni::{JNIEnv, JavaVM};
use kervan_node::{Event, FileView, Node};
use tauri::{AppHandle, Manager, RunEvent, WebviewWindowBuilder};

static APP: OnceLock<AppHandle> = OnceLock::new();

/// The Java side, for posting notifications. Tauri's notification plugin
/// needs a screen, so while there is none they go through `KervanService`.
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
    *NOTIFICATIONS.lock().unwrap_or_else(PoisonError::into_inner) = Notifications {
        on,
        turkish: language == "tr",
    };
}

/// `MainActivity.attachScreen`, called as each screen is created. The first
/// one gets its window from setup; one that opens in a process that outlived
/// its screen gets the window the configuration describes.
#[no_mangle]
pub extern "system" fn Java_dev_kemalarslan_kervan_MainActivity_attachScreen(
    mut env: JNIEnv,
    _activity: JObject,
) {
    // The app's classes can only be looked up from a thread Java started.
    if JAVA.get().is_none() {
        let java = env.get_java_vm().and_then(|vm| {
            let class = env.find_class("dev/kemalarslan/kervan/KervanService")?;
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
pub fn notify(app: &AppHandle, node: &Node, event: &Event) {
    let settings = *NOTIFICATIONS.lock().unwrap_or_else(PoisonError::into_inner);
    if !settings.on || !app.webview_windows().is_empty() {
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

/// `KervanService.notifyMessage(title, body)`.
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
