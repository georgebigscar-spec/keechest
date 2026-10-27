//! Gives the Windows executable its icon.

fn main() {
    println!("cargo:rerun-if-changed=../../assets/keechest.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("../../assets/keechest.ico")
            .compile()
            .expect("embed the Windows icon");
    }
}
