fn main() {
    // BIOS WMI calls need admin. Release builds embed a manifest that asks for it; the logon
    // task runs with highest privileges so there is no UAC prompt at boot. Debug builds skip
    // it so `cargo test` can run unelevated.
    if std::env::var("PROFILE").as_deref() == Ok("release") {
        println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg-bins=/MANIFESTUAC:level='requireAdministrator'");
    }
}
