//! The native library of the Android app: `android_main`, which
//! android-activity calls on its own thread, and the native methods of
//! `MainActivity` (see `android/`). Empty on other platforms.

#![cfg(target_os = "android")]

use jni::JNIEnv;
use jni::objects::{JClass, JString};
use jni::sys::jint;
use keechest::platform::android as platform;
use winit::platform::android::activity::AndroidApp;

#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("KeeChest"),
    );
    if let Err(e) = keechest::run(app) {
        log::error!("KeeChest stopped: {e}");
    }
}

fn string(env: &mut JNIEnv, s: &JString) -> Option<String> {
    if s.is_null() {
        return None;
    }
    env.get_string(s).ok().map(String::from)
}

#[unsafe(no_mangle)]
extern "system" fn Java_io_github_keechest_MainActivity_nativeFileResult(
    mut env: JNIEnv,
    _class: JClass,
    request: jint,
    fd: jint,
    name: JString,
) {
    let name = string(&mut env, &name);
    platform::on_file_result(request, fd, name);
}

#[unsafe(no_mangle)]
extern "system" fn Java_io_github_keechest_MainActivity_nativeText(
    mut env: JNIEnv,
    _class: JClass,
    text: JString,
) {
    if let Some(text) = string(&mut env, &text) {
        platform::on_text(text);
    }
}

#[unsafe(no_mangle)]
extern "system" fn Java_io_github_keechest_MainActivity_nativeDelete(
    _env: JNIEnv,
    _class: JClass,
    count: jint,
) {
    platform::on_delete(usize::try_from(count).unwrap_or(0));
}

#[unsafe(no_mangle)]
extern "system" fn Java_io_github_keechest_MainActivity_nativeEnter(_env: JNIEnv, _class: JClass) {
    platform::on_enter();
}

#[unsafe(no_mangle)]
extern "system" fn Java_io_github_keechest_MainActivity_nativeBackground(
    _env: JNIEnv,
    _class: JClass,
) {
    platform::on_background();
}
