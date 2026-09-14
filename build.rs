fn main() {
    println!("cargo:rerun-if-changed=assets/snake-charger.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/snake-charger.ico")
            .set("ProductName", "Snake Charger")
            .set("FileDescription", "Snake Charger")
            .set("CompanyName", "va0z-code");
        res.compile().expect("failed to embed Windows resources");
    }
}
