//! Checks the files in `resources/` and the manifest against each other and against the code,
//! and runs the real `install.sh` in a throw-away HOME.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

const BIN: &str = "cosmic-todo-widget";

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("cannot read {rel}: {e}"))
}

/// The app id exactly as written in `src/paths.rs`.
fn app_id() -> String {
    let src = read("src/paths.rs");
    let line = src
        .lines()
        .find(|l| l.contains("pub const APP_ID"))
        .expect("APP_ID not found");
    line.split('"')
        .nth(1)
        .expect("APP_ID has no value")
        .to_string()
}

fn res(name_suffix: &str) -> String {
    read(&format!("resources/{}{}", app_id(), name_suffix))
}

fn exec_lines(text: &str) -> Vec<&str> {
    text.lines().filter(|l| l.starts_with("Exec=")).collect()
}

/// A scratch folder deleted at the end of the test.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("todo-widget-it-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn the_package_and_the_binary_have_the_expected_name() {
    assert_eq!(env!("CARGO_PKG_NAME"), BIN);
}

#[test]
fn every_resource_file_is_named_after_the_app_id() {
    let id = app_id();
    for suffix in [".desktop", ".Widget.desktop", ".metainfo.xml", ".svg"] {
        let file = root().join("resources").join(format!("{id}{suffix}"));
        assert!(file.is_file(), "missing {}", file.display());
    }
    assert!(root().join("resources/install.sh").is_file());
    assert!(root().join("resources/autostart.desktop.in").is_file());
}

#[test]
fn the_panel_applet_entry_is_valid() {
    let text = res(".desktop");
    // install.sh rewrites exactly this line to point at the installed binary.
    assert_eq!(exec_lines(&text), [format!("Exec={BIN} --applet")]);
    assert!(text.contains(&format!("Icon={}\n", app_id())));
    assert!(text.contains("X-CosmicApplet=true"));
}

#[test]
fn the_launcher_entry_is_valid() {
    let text = res(".Widget.desktop");
    // install.sh knows exactly these two forms.
    assert_eq!(
        exec_lines(&text),
        [format!("Exec={BIN}"), format!("Exec={BIN} --settings")]
    );
    assert!(text.contains(&format!("Icon={}\n", app_id())));
    assert!(text.contains("Actions=Settings;"));
}

#[test]
fn the_icon_is_an_svg() {
    let svg = res(".svg");
    assert!(svg.contains("<svg") && svg.contains("</svg>"));
}

#[test]
fn the_metainfo_matches_the_app_and_lists_the_current_version() {
    let id = app_id();
    let xml = res(".metainfo.xml");
    assert!(xml.contains(&format!("<id>{id}</id>")));
    assert!(xml.contains(&format!(
        "<launchable type=\"desktop-id\">{id}.Widget.desktop</launchable>"
    )));
    assert!(xml.contains(&format!("<binary>{BIN}</binary>")));
    let version = env!("CARGO_PKG_VERSION");
    assert!(
        xml.contains(&format!("<release version=\"{version}\"")),
        "add a <release version=\"{version}\" .../> entry to the metainfo (`just bump` does it)"
    );
}

#[test]
fn the_autostart_template_has_its_placeholder() {
    assert!(read("resources/autostart.desktop.in").contains("Exec=@EXEC@"));
}

#[test]
fn the_flatpak_manifest_uses_the_same_id_and_files() {
    let id = app_id();
    let manifest = read(&format!("flatpak/{id}.yml"));
    assert!(manifest.contains(&format!("app-id: {id}")));
    assert!(manifest.contains(&format!("command: {BIN}")));
    for suffix in [".desktop", ".Widget.desktop", ".metainfo.xml", ".svg"] {
        assert!(
            manifest.contains(&format!("resources/{id}{suffix}")),
            "manifest does not install {suffix}"
        );
    }
    assert!(root().join("flatpak/cargo-sources.json").is_file());
}

#[test]
fn install_sh_knows_the_same_app_id() {
    assert!(read("resources/install.sh").contains(&format!("app_id={}\n", app_id())));
}

#[test]
fn install_sh_is_valid_bash() {
    let status = Command::new("bash")
        .arg("-n")
        .arg(root().join("resources/install.sh"))
        .status()
        .unwrap();
    assert!(status.success());
}

fn run_installer(home: &Path, bin: &Path, arg: Option<&str>) {
    let mut cmd = Command::new("bash");
    cmd.arg(root().join("resources/install.sh"));
    if let Some(a) = arg {
        cmd.arg(a);
    }
    let out = cmd
        .env("HOME", home)
        .env("RESOURCES_DIR", root().join("resources"))
        .env("BIN_SRC", bin)
        .env("NO_STOP", "1")
        .env("QUIET", "1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "install.sh failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn install_sh_installs_and_removes_everything() {
    let scratch = Scratch::new();
    let home = scratch.0.join("home");
    let apps = home.join(".local/share/applications");
    let icons = home.join(".local/share/icons/hicolor/scalable/apps");
    fs::create_dir_all(&apps).unwrap();
    fs::create_dir_all(&icons).unwrap();
    let id = app_id();

    // A leftover of an older version must be cleaned up.
    let old_launcher = apps.join("todo-widget.desktop");
    fs::write(&old_launcher, "old").unwrap();

    let fake_bin = scratch.0.join(BIN);
    fs::write(&fake_bin, "#!/bin/sh\n").unwrap();

    run_installer(&home, &fake_bin, None);

    let bin = home.join(".local/bin").join(BIN);
    let applet_bin = home.join(".local/bin/todo-widget-applet");
    assert!(bin.is_file());
    assert!(applet_bin.is_symlink());
    assert!(icons.join(format!("{id}.svg")).is_file());
    assert!(!old_launcher.exists(), "the old launcher should be removed");

    let applet = fs::read_to_string(apps.join(format!("{id}.desktop"))).unwrap();
    assert_eq!(
        exec_lines(&applet),
        [format!("Exec={} --applet", applet_bin.display())]
    );

    let launcher = fs::read_to_string(apps.join(format!("{id}.Widget.desktop"))).unwrap();
    assert_eq!(
        exec_lines(&launcher),
        [
            format!(
                "Exec=sh -c 'pkill -x cosmic-todo-wid; exec {}'",
                bin.display()
            ),
            format!(
                "Exec=sh -c 'pkill -x cosmic-todo-wid; exec {} --settings'",
                bin.display()
            ),
        ]
    );

    // Installing again over the top works, and uninstalling leaves nothing behind.
    run_installer(&home, &fake_bin, None);
    run_installer(&home, &fake_bin, Some("uninstall"));
    for gone in [
        bin,
        applet_bin,
        apps.join(format!("{id}.desktop")),
        apps.join(format!("{id}.Widget.desktop")),
        icons.join(format!("{id}.svg")),
    ] {
        assert!(
            !gone.exists() && !gone.is_symlink(),
            "{} is still there",
            gone.display()
        );
    }
}
