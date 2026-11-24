fn main() {
    // Release builds made by GitHub Actions set this so the update checker
    // knows which repository its releases come from.
    println!("cargo:rerun-if-env-changed=ROOMGUARD_REPO");
    println!("cargo:rerun-if-changed=assets/icon/icon.ico");

    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon/icon.ico");
        res.set("FileDescription", "RoomGuard");
        res.set("ProductName", "RoomGuard");
        res.set("LegalCopyright", "Copyright 2026 Kaushal Bhingaradia. Apache License 2.0.");
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed the Windows icon: {}", e);
        }
    }
}
