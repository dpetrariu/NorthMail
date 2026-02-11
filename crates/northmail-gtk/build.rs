use std::process::Command;
use std::path::Path;
use std::env;

fn main() {
    // Compile GResource
    let out_dir = env::var("OUT_DIR").unwrap();
    let project_root = env::var("CARGO_MANIFEST_DIR").unwrap();
    let data_dir = Path::new(&project_root).parent().unwrap().parent().unwrap().join("data");

    let gresource_xml = data_dir.join("resources.gresource.xml");
    let gresource_out = Path::new(&out_dir).join("resources.gresource");

    if gresource_xml.exists() {
        let status = Command::new("glib-compile-resources")
            .arg("--sourcedir")
            .arg(&data_dir)
            .arg("--target")
            .arg(&gresource_out)
            .arg(&gresource_xml)
            .status()
            .expect("Failed to compile resources");

        if !status.success() {
            panic!("glib-compile-resources failed");
        }

        println!("cargo:rerun-if-changed={}", gresource_xml.display());
        println!("cargo:rustc-env=GRESOURCE_FILE={}", gresource_out.display());
    }
}
