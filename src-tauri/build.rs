fn main() {
  // Re-embed Windows .exe icon whenever any asset in icons/ changes.
  println!("cargo:rerun-if-changed=icons/icon.ico");
  println!("cargo:rerun-if-changed=icons/icon.png");
  println!("cargo:rerun-if-changed=icons/32x32.png");

  // NOTE: STT models are no longer bundled. They are downloaded inside the app on first use.
  // See model_manager + in-app download UX.

  tauri_build::build()
}
