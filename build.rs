fn main() {
    // BIOS WMI calls need admin. Embed a manifest that asks for it; the logon
    // task runs with highest privileges so there is no UAC prompt at boot.
    println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg-bins=/MANIFESTUAC:level='requireAdministrator'");
}
