//! The Android side: calls into `MainActivity` (see `android/`) over JNI
//! and the callbacks it makes back, forwarded by the `keechest-android`
//! library.
//!
//! The activity owns what needs Java: the system file picker (Storage
//! Access Framework), the clipboard with its clear timer, and the
//! on-screen keyboard. Typed text comes back here and is fed to egui as
//! input events, since a `NativeActivity` only gets key codes from the
//! keyboard, which covers Latin letters at best.

use std::fs::File;
use std::io;
use std::os::fd::FromRawFd;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Mutex, OnceLock};

use jni::objects::{JObject, JValue};
use jni::{JNIEnv, JavaVM};
use winit::platform::android::activity::AndroidApp;

use super::{FileKind, Picked, PickedFile};
use crate::task::Task;

static APP: OnceLock<AndroidApp> = OnceLock::new();
static CTX: OnceLock<egui::Context> = OnceLock::new();
static NEXT_REQUEST: AtomicI32 = AtomicI32::new(1);
/// Pickers waiting for the activity's answer, by request code.
static PICKERS: Mutex<Vec<(i32, Sender<Picked>)>> = Mutex::new(Vec::new());
/// Keyboard input not yet given to egui.
static INPUT: Mutex<Vec<Input>> = Mutex::new(Vec::new());
static BACKGROUNDED: AtomicBool = AtomicBool::new(false);
static KEYBOARD: Mutex<Keyboard> = Mutex::new(Keyboard {
    shown: false,
    focus: None,
    password: false,
});

enum Input {
    Text(String),
    /// Characters to delete before the cursor.
    Delete(usize),
    Enter,
}

/// What the on-screen keyboard was last asked to do.
struct Keyboard {
    shown: bool,
    focus: Option<egui::Id>,
    password: bool,
}

pub fn init(app: &AndroidApp) {
    let _ = APP.set(app.clone());
}

/// Calls `f` with the JNI environment and the activity. Local references
/// made in `f` are freed after, since this thread never returns to Java.
fn with_activity<R>(f: impl FnOnce(&mut JNIEnv, &JObject) -> jni::errors::Result<R>) -> Option<R> {
    let app = APP.get()?;
    // SAFETY: android-activity keeps the VM and a global reference to the
    // activity valid for as long as the app runs.
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) }.ok()?;
    let mut env = vm.attach_current_thread().ok()?;
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
    let result = env.with_local_frame(16, |env| f(env, &activity));
    match result {
        Ok(r) => Some(r),
        Err(e) => {
            if env.exception_check().unwrap_or(false) {
                let _ = env.exception_describe();
                let _ = env.exception_clear();
            }
            log::warn!("JNI call failed: {e}");
            None
        }
    }
}

fn request(
    ctx: &egui::Context,
    call: impl FnOnce(&mut JNIEnv, &JObject, i32) -> jni::errors::Result<()>,
) -> Task<Picked> {
    let _ = CTX.set(ctx.clone());
    let (tx, task) = Task::channel();
    let id = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
    PICKERS.lock().unwrap().push((id, tx));
    if with_activity(|env, activity| call(env, activity, id)).is_none() {
        let tx = take_picker(id);
        if let Some(tx) = tx {
            let _ = tx.send(Err(io::Error::other("не удалось открыть выбор файла")));
        }
    }
    task
}

fn take_picker(id: i32) -> Option<Sender<Picked>> {
    let mut pickers = PICKERS.lock().unwrap();
    let i = pickers.iter().position(|(r, _)| *r == id)?;
    Some(pickers.swap_remove(i).1)
}

pub fn open_file(ctx: &egui::Context, kind: FileKind) -> Task<Picked> {
    request(ctx, |env, activity, id| {
        let mime = env.new_string(kind.mime(false))?;
        env.call_method(
            activity,
            "openDocument",
            "(ILjava/lang/String;)V",
            &[JValue::Int(id), JValue::Object(&mime)],
        )?;
        Ok(())
    })
}

pub fn create_file(ctx: &egui::Context, kind: FileKind, name: &str) -> Task<Picked> {
    request(ctx, |env, activity, id| {
        let mime = env.new_string(kind.mime(true))?;
        let name = env.new_string(name)?;
        env.call_method(
            activity,
            "createDocument",
            "(ILjava/lang/String;Ljava/lang/String;)V",
            &[
                JValue::Int(id),
                JValue::Object(&mime),
                JValue::Object(&name),
            ],
        )?;
        Ok(())
    })
}

/// The answer to `openDocument` or `createDocument`: a descriptor the
/// activity detached for us, -1 if the user cancelled, or -2 with an error
/// message in `name`.
pub fn on_file_result(request: i32, fd: i32, name: Option<String>) {
    let result = match fd {
        -1 => Ok(None),
        fd if fd >= 0 => Ok(Some(PickedFile {
            name: name.unwrap_or_default(),
            // SAFETY: the activity detached the descriptor and hands its
            // ownership to us.
            file: unsafe { File::from_raw_fd(fd) },
            path: None,
        })),
        _ => Err(io::Error::other(name.unwrap_or_default())),
    };
    match take_picker(request) {
        Some(tx) => {
            let _ = tx.send(result);
        }
        None => log::warn!("file result for unknown request {request}"),
    }
    repaint();
}

/// Text committed by the keyboard.
pub fn on_text(text: String) {
    INPUT.lock().unwrap().push(Input::Text(text));
    repaint();
}

/// The keyboard deleted `count` characters before the cursor.
pub fn on_delete(count: usize) {
    INPUT.lock().unwrap().push(Input::Delete(count));
    repaint();
}

/// The keyboard's action key (Done, Go).
pub fn on_enter() {
    INPUT.lock().unwrap().push(Input::Enter);
    repaint();
}

/// The activity was paused for anything but our own picker.
pub fn on_background() {
    BACKGROUNDED.store(true, Ordering::Relaxed);
    repaint();
}

fn repaint() {
    if let Some(ctx) = CTX.get() {
        ctx.request_repaint();
    }
}

pub fn config_dir() -> Option<PathBuf> {
    APP.get()?.internal_data_path()
}

pub fn vault_dir() -> PathBuf {
    config_dir().unwrap_or_default()
}

/// Hands the keyboard input that arrived since the last frame to egui.
pub fn begin_frame(ctx: &egui::Context) {
    let _ = CTX.set(ctx.clone());
    let input = std::mem::take(&mut *INPUT.lock().unwrap());
    if input.is_empty() {
        return;
    }
    let key = |key, pressed| egui::Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    ctx.input_mut(|i| {
        for input in input {
            match input {
                Input::Text(text) => i.events.push(egui::Event::Text(text)),
                Input::Delete(count) => {
                    for _ in 0..count {
                        i.events.push(key(egui::Key::Backspace, true));
                        i.events.push(key(egui::Key::Backspace, false));
                    }
                }
                Input::Enter => {
                    i.events.push(key(egui::Key::Enter, true));
                    i.events.push(key(egui::Key::Enter, false));
                }
            }
        }
    });
}

/// Shows the keyboard while a text field has focus, hides it after.
pub fn end_frame(ctx: &egui::Context) {
    let ime = ctx.output(|o| o.ime);
    let focus = ctx.memory(|m| m.focused());
    let tapped = ctx.input(|i| i.pointer.any_released());
    let mut kb = KEYBOARD.lock().unwrap();
    match ime {
        Some(ime) => {
            let password = ime.purpose == egui::IMEPurpose::Password;
            // A new field starts a new input session, so the keyboard
            // forgets the previous field's text. A tap brings the keyboard
            // back if the user hid it.
            let restart = !kb.shown || kb.focus != focus || kb.password != password;
            if restart || tapped || ime.should_interrupt_composition {
                kb.shown = true;
                kb.focus = focus;
                kb.password = password;
                show_keyboard(true, password, restart || ime.should_interrupt_composition);
            }
        }
        None if kb.shown => {
            kb.shown = false;
            kb.focus = None;
            show_keyboard(false, false, false);
        }
        None => {}
    }
}

fn show_keyboard(visible: bool, password: bool, restart: bool) {
    with_activity(|env, activity| {
        env.call_method(
            activity,
            "showKeyboard",
            "(ZZZ)V",
            &[
                JValue::Bool(visible.into()),
                JValue::Bool(password.into()),
                JValue::Bool(restart.into()),
            ],
        )?;
        Ok(())
    });
}

pub fn take_backgrounded() -> bool {
    BACKGROUNDED.swap(false, Ordering::Relaxed)
}

pub fn leave() {
    with_activity(|env, activity| {
        env.call_method(activity, "moveTaskToBack", "(Z)Z", &[JValue::Bool(1)])?;
        Ok(())
    });
}

/// Copies `text` so that the system hides it from previews, and clears it
/// after `clear_after_ms` (0 never), even if the app is in the background
/// by then.
pub fn copy_secret(text: &str, clear_after_ms: u64) -> bool {
    with_activity(|env, activity| {
        let text = env.new_string(text)?;
        env.call_method(
            activity,
            "copySecret",
            "(Ljava/lang/String;J)V",
            &[
                JValue::Object(&text),
                JValue::Long(i64::try_from(clear_after_ms).unwrap_or(i64::MAX)),
            ],
        )?;
        Ok(())
    })
    .is_some()
}

/// Clears what [`copy_secret`] copied, if it is still on the clipboard.
pub fn clear_secret() {
    with_activity(|env, activity| {
        env.call_method(activity, "clearSecret", "()V", &[])?;
        Ok(())
    });
}

pub fn choose_vault_location(_current: &std::path::Path) -> Option<PathBuf> {
    None
}
