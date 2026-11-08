//! What the page can ask for. Each call arrives as `{"id", "cmd", "args"}`, runs on its own
//! thread and is answered with `window.__xd.resolve(id, value, error)`; long steps (an
//! install, the game) also send their lines as `window.__xd.emit("log", …)`.
//!
//! Instances, accounts, packs, Java and installs are the Horizon launcher's
//! (`horizon-launcher --json …`, shipped next to this program), so both launchers share the same
//! data. xD Client keeps only its own look and launch preferences (`settings.json`).

use crate::UserEvent;
use base64::Engine;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tao::event_loop::EventLoopProxy;

type Proxy = EventLoopProxy<UserEvent>;

/// Whether a game started from here is still running.
static PLAYING: AtomicBool = AtomicBool::new(false);

pub fn handle(body: String, proxy: Proxy) {
    let msg: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    let id = msg["id"].as_u64().unwrap_or(0);
    let cmd = msg["cmd"].as_str().unwrap_or("").to_owned();
    let args = msg["args"].clone();
    if cmd == "drag" {
        let _ = proxy.send_event(UserEvent::Drag);
        return;
    }
    std::thread::spawn(move || {
        let result = dispatch(&cmd, &args, &proxy);
        let js = match result {
            Ok(v) => format!("window.__xd.resolve({id},{},null)", js_value(&v)),
            Err(e) => format!("window.__xd.resolve({id},null,{})", js_value(&json!(e))),
        };
        let _ = proxy.send_event(UserEvent::Eval(js));
    });
}

/// JSON as a script literal: the two line separators JSON allows and JavaScript strings don't.
fn js_value(v: &Value) -> String {
    v.to_string()
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

fn emit(proxy: &Proxy, name: &str, data: Value) {
    let js = format!(
        "window.__xd.emit({},{})",
        js_value(&json!(name)),
        js_value(&data)
    );
    let _ = proxy.send_event(UserEvent::Eval(js));
}

fn s<'a>(args: &'a Value, key: &str) -> &'a str {
    args[key].as_str().unwrap_or("")
}

fn dispatch(cmd: &str, args: &Value, proxy: &Proxy) -> Result<Value, String> {
    match cmd {
        "state" => Ok(json!({
            "instances": cli(&["instance", "list"])?,
            "accounts": cli(&["account", "list"]).unwrap_or(Value::Null),
            "installed": cli(&["versions", "--installed"]).unwrap_or(Value::Null),
            "horizon": cli(&["settings"]).unwrap_or(Value::Null),
            "settings": load_settings(),
            "ram_gb": total_ram_gb(),
            "launcher": launcher_path().display().to_string(),
            "launcher_found": launcher_path().is_file(),
            "playing": PLAYING.load(Ordering::Relaxed),
        })),
        "instances" => cli(&["instance", "list"]),
        "selectInstance" => cli(&["instance", "select", s(args, "id")]),
        "createInstance" => {
            let mut a = vec!["instance", "create", s(args, "name")];
            if !s(args, "minecraft").is_empty() {
                a.extend(["--minecraft", s(args, "minecraft")]);
                if !s(args, "loader").is_empty() {
                    a.extend(["--loader", s(args, "loader")]);
                }
            }
            cli(&a)
        }
        "importInstance" => {
            let Some(file) = rfd::FileDialog::new()
                .add_filter("Modpack", &["mrpack", "zip"])
                .pick_file()
            else {
                return Ok(Value::Null);
            };
            cli(&["instance", "import", &file.display().to_string()])
        }
        "renameInstance" => cli(&["instance", "rename", s(args, "id"), s(args, "name")]),
        "deleteInstance" => cli(&["instance", "delete", s(args, "id")]),
        "minecraftVersions" => cli(&["instance", "versions"]),

        "mods" => Ok(json!(list_mods(Path::new(s(args, "home")), args["java"] == true))),
        "toggleMod" => toggle_mod(
            Path::new(s(args, "home")),
            args["java"] == true,
            s(args, "id"),
            args["on"] == true,
        )
        .map(|_| Value::Null),
        "addMods" => add_mods(Path::new(s(args, "home")), args["java"] == true),

        "packs" => cli(&["packs", "list", "--instance", s(args, "instance")]),
        "packOn" | "packOff" => cli(&[
            "packs",
            if cmd == "packOn" { "on" } else { "off" },
            s(args, "file"),
            "--kind",
            s(args, "kind"),
            "--instance",
            s(args, "instance"),
        ]),
        "packOrder" => pack_order(args),
        "addPacks" => {
            let Some(files) = rfd::FileDialog::new()
                .add_filter("Paket", &["zip", "hshader"])
                .pick_files()
            else {
                return Ok(Value::Null);
            };
            for f in files {
                cli(&[
                    "packs",
                    "add",
                    &f.display().to_string(),
                    "--instance",
                    s(args, "instance"),
                ])?;
            }
            Ok(Value::Null)
        }

        "accounts" => cli(&["account", "list"]),
        "selectAccount" => cli(&["account", "select", s(args, "who")]),
        "signOut" => cli(&["account", "sign-out", s(args, "who")]),
        "forgetOffline" => cli(&["account", "forget-offline", s(args, "who")]),
        "offline" => cli(&["account", "offline", s(args, "name")]),
        "signIn" => sign_in(proxy),
        "setSkin" => set_skin(args),
        "accountSkin" => account_skin(s(args, "uuid")),

        "javaList" => cli(&["java", "list"]),
        "horizonSetting" => cli(&["settings", s(args, "key"), s(args, "value")]),
        "installLatest" => {
            run_logged(proxy, &["install", "latest"], "install")?;
            cli(&["versions", "--installed"])
        }
        "prepare" => run_logged(proxy, &["prepare"], "prepare").map(|_| Value::Null),

        "loadSettings" => Ok(load_settings()),
        "saveSettings" => save_settings(&args["settings"]).map(|_| Value::Null),
        "pickImage" => pick_image(),
        "pickJava" => Ok(rfd::FileDialog::new()
            .pick_file()
            .map(|p| json!(java_home_of(&p).display().to_string()))
            .unwrap_or(Value::Null)),
        "pickFolder" => Ok(rfd::FileDialog::new()
            .pick_folder()
            .map(|p| json!(p.display().to_string()))
            .unwrap_or(Value::Null)),
        "openPath" => {
            let path = Path::new(s(args, "path"));
            // A pack or mod folder the instance hasn't made yet: made now, so it opens.
            if args["create"] == true && !path.exists() {
                std::fs::create_dir_all(path).map_err(|e| e.to_string())?;
            }
            open(s(args, "path")).map(|_| Value::Null)
        }

        "worlds" => cli(&["worlds", "list", "--instance", s(args, "instance")]),
        "screenshots" => cli(&["screenshots", "list", "--instance", s(args, "instance")]),
        "catalog" => catalog(args),
        "install" => install(args, proxy),
        "openUrl" => open(s(args, "url")).map(|_| Value::Null),

        "launch" => launch(args, proxy),
        _ => Err(format!("Unbekannter Befehl: {cmd}")),
    }
}

// --- the Horizon launcher's command line ---------------------------------------------------

/// `horizon-launcher` next to this program (inside the .app), else `XD_HORIZON_LAUNCHER`,
/// else the one on PATH.
fn launcher_path() -> PathBuf {
    if let Ok(p) = std::env::var("XD_HORIZON_LAUNCHER") {
        return PathBuf::from(p);
    }
    let name = if cfg!(windows) {
        "horizon-launcher.exe"
    } else {
        "horizon-launcher"
    };
    std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join(name)))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from(name))
}

fn launcher(args: &[&str]) -> Command {
    let mut cmd = Command::new(launcher_path());
    cmd.args(args)
        // The bundled copy is updated with xD Client, never by itself.
        .env("HORIZON_LAUNCHER_NO_SELF_UPDATE", "1")
        .stdin(Stdio::null());
    cmd
}

/// Runs a command with `--json` and returns what it printed: JSON when it is, else its text.
fn cli(args: &[&str]) -> Result<Value, String> {
    let mut all = vec!["--json"];
    all.extend_from_slice(args);
    let out = launcher(&all).output().map_err(|e| {
        format!(
            "Der Horizon-Launcher ({}) lässt sich nicht starten: {e}",
            launcher_path().display()
        )
    })?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let line = err
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("Fehlgeschlagen");
        return Err(line.trim().trim_start_matches("error: ").to_owned());
    }
    Ok(serde_json::from_str(&stdout).unwrap_or_else(|_| json!(stdout.trim())))
}

/// Runs a long command, sending each line of its output to the page's console as it comes.
/// Returns the last lines of its error output when it fails.
fn run_logged(proxy: &Proxy, args: &[&str], tag: &str) -> Result<(), String> {
    let mut child = launcher(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{}: {e}", launcher_path().display()))?;
    let tail = stream(proxy, &mut child, tag);
    let status = child.wait().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        let tail = tail.lock().unwrap();
        Err(tail
            .iter()
            .rev()
            .find(|l| l.starts_with("error:"))
            .or(tail.last())
            .map(|l| l.trim_start_matches("error: ").to_owned())
            .unwrap_or_else(|| format!("Beendet mit {status}")))
    }
}

/// Reads the child's stdout and stderr on two threads into the console; keeps stderr's last
/// lines. Both threads end with the child's pipes.
fn stream(proxy: &Proxy, child: &mut std::process::Child, tag: &str) -> Arc<Mutex<Vec<String>>> {
    let tail = Arc::new(Mutex::new(Vec::new()));
    let pipes: [(Option<Box<dyn Read + Send>>, &str); 2] = [
        (
            child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>),
            "out",
        ),
        (
            child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>),
            "err",
        ),
    ];
    let mut joins = Vec::new();
    for (pipe, which) in pipes {
        let Some(pipe) = pipe else { continue };
        let (proxy, tail, tag) = (proxy.clone(), tail.clone(), tag.to_owned());
        joins.push(std::thread::spawn(move || {
            // Progress bars rewrite one line with \r: each piece is a line here.
            let mut reader = BufReader::new(pipe);
            let mut buf = Vec::new();
            while reader.read_until(b'\n', &mut buf).map(|n| n > 0).unwrap_or(false) {
                let text = String::from_utf8_lossy(&buf).into_owned();
                for line in text.split('\r').map(str::trim_end).filter(|l| !l.is_empty()) {
                    emit(
                        &proxy,
                        "log",
                        json!({ "line": line, "stream": which, "tag": tag }),
                    );
                    if which == "err" {
                        let mut t = tail.lock().unwrap();
                        t.push(line.to_owned());
                        let n = t.len();
                        if n > 40 {
                            t.drain(..n - 40);
                        }
                    }
                }
                buf.clear();
            }
        }));
    }
    // The caller waits for the child; the readers finish with its pipes.
    std::thread::spawn(move || joins.into_iter().for_each(|j| drop(j.join())));
    tail
}

// --- playing ---------------------------------------------------------------------------------

fn launch(args: &Value, proxy: &Proxy) -> Result<Value, String> {
    if PLAYING.swap(true, Ordering::SeqCst) {
        return Err("Das Spiel läuft schon.".into());
    }
    let result = launch_inner(args, proxy);
    PLAYING.store(false, Ordering::SeqCst);
    emit(proxy, "game", json!({ "state": "exited" }));
    result
}

fn launch_inner(args: &Value, proxy: &Proxy) -> Result<Value, String> {
    let id = s(args, "instance").to_owned();
    let java = args["java"] == true;
    let cfg = &args["settings"];
    let on_launch = cfg["onLaunch"].as_str().unwrap_or("keep");
    let (w, h) = (
        cfg["width"].as_u64().unwrap_or(0),
        cfg["height"].as_u64().unwrap_or(0),
    );
    let fullscreen = cfg["fullscreen"] == true;

    if java {
        // The launcher's own settings for a Java instance, from xD Client's: memory, -Xms and the
        // JVM arguments, the Java to run on, the window size.
        let xmx = (cfg["xmx"].as_f64().unwrap_or(4.0) * 1024.0) as u64;
        let xms = (cfg["xms"].as_f64().unwrap_or(2.0) * 1024.0) as u64;
        let jvm = format!("-Xms{xms}M {}", cfg["jvm"].as_str().unwrap_or("")).trim().to_owned();
        let memory = xmx.to_string();
        let mut a = vec![
            "instance".to_owned(),
            "java".into(),
            id.clone(),
            "--memory".into(),
            memory,
            format!("--args={jvm}"),
        ];
        match cfg["javaHome"].as_str().filter(|h| !h.is_empty()) {
            Some(home) => a.extend(["--home".into(), home.to_owned()]),
            None => a.push("--auto".into()),
        }
        if w > 0 && h > 0 {
            a.push(format!("--window={w}x{h}"));
        }
        let a: Vec<&str> = a.iter().map(String::as_str).collect();
        cli(&a)?;
    } else {
        let installed = cli(&["versions", "--installed"])?;
        if installed["installed"].as_array().is_none_or(|v| v.is_empty()) {
            emit(proxy, "status", json!("Horizon wird installiert …"));
            run_logged(proxy, &["install", "latest"], "install")?;
        }
    }

    let mut play: Vec<String> = vec!["play".into(), "--instance".into(), id.clone()];
    if let Some(world) = args["world"].as_str().filter(|w| !w.is_empty() && !java) {
        play.extend(["--world".into(), world.to_owned()]);
    }
    if !java {
        if w > 0 && h > 0 {
            play.extend([
                "--arg=--width".into(),
                format!("--arg={w}"),
                "--arg=--height".into(),
                format!("--arg={h}"),
            ]);
        }
        if fullscreen {
            play.push("--arg=--fullscreen".into());
        }
    }
    let play: Vec<&str> = play.iter().map(String::as_str).collect();
    emit(
        proxy,
        "log",
        json!({ "line": format!("[xD Client] horizon-launcher {}", play.join(" ")), "stream": "info", "tag": "game" }),
    );

    if on_launch == "close" {
        // Nothing reads its output once xD Client has gone, so it gets none to write to.
        launcher(&play)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        let _ = proxy.send_event(UserEvent::Quit);
        return Ok(Value::Null);
    }

    let mut first = play_once(proxy, &play, on_launch);
    // A Horizon version needs the game's resources from the player's own Minecraft once.
    if !java {
        if let Err(e) = &first {
            if e.to_lowercase().contains("prepare") {
                emit(
                    proxy,
                    "status",
                    json!("Horizon wird vorbereitet (aus deiner Minecraft-Installation) …"),
                );
                run_logged(proxy, &["prepare"], "prepare")?;
                first = play_once(proxy, &play, on_launch);
            }
        }
    }
    first.map(|_| Value::Null)
}

fn play_once(proxy: &Proxy, play: &[&str], on_launch: &str) -> Result<(), String> {
    let mut child = launcher(play)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    emit(proxy, "game", json!({ "state": "running" }));
    if on_launch == "hide" {
        let _ = proxy.send_event(UserEvent::Hide);
    }
    let tail = stream(proxy, &mut child, "game");
    let status = child.wait().map_err(|e| e.to_string());
    if on_launch == "hide" {
        let _ = proxy.send_event(UserEvent::Show);
    }
    let status = status?;
    if status.success() {
        return Ok(());
    }
    let tail = tail.lock().unwrap();
    Err(tail
        .iter()
        .rev()
        .find(|l| l.starts_with("error:"))
        .or(tail.last())
        .map(|l| l.trim_start_matches("error: ").to_owned())
        .unwrap_or_else(|| format!("Das Spiel wurde beendet ({status}).")))
}

// --- mods ------------------------------------------------------------------------------------

/// A Java instance's jars in `mods/` (`.disabled` when off); a Horizon instance's mods in
/// `horizon-mods/` and `horizon-native-mods/`, named by the mod browser's list where it has them,
/// on when `horizon-mods.txt` says `mod.<id>:true` (as the game's Options > Mods writes it).
fn list_mods(home: &Path, java: bool) -> Vec<Value> {
    let mut out = Vec::new();
    let titles: Vec<(String, String, String)> =
        std::fs::read_to_string(home.join("mod-browser.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| v["entries"].as_array().cloned())
            .unwrap_or_default()
            .iter()
            .map(|e| {
                (
                    e["file"].as_str().unwrap_or("").to_owned(),
                    e["title"].as_str().unwrap_or("").to_owned(),
                    e["number"].as_str().unwrap_or("").to_owned(),
                )
            })
            .collect();
    let named = |file: &str, fallback: &str| {
        titles
            .iter()
            .find(|(f, _, _)| f == file)
            .map(|(_, t, n)| (t.clone(), n.clone()))
            .unwrap_or((fallback.to_owned(), String::new()))
    };
    if java {
        for f in read_dir(&home.join("mods")) {
            let name = f.file_name().unwrap_or_default().to_string_lossy().into_owned();
            let jar = name.strip_suffix(".disabled").unwrap_or(&name);
            if !jar.ends_with(".jar") {
                continue;
            }
            let (title, version) = named(&format!("mods/{jar}"), jar.trim_end_matches(".jar"));
            out.push(json!({ "id": jar, "title": title, "version": version, "on": !name.ends_with(".disabled") }));
        }
    } else {
        let list = std::fs::read_to_string(home.join("horizon-mods.txt")).unwrap_or_default();
        let on = |id: &str| {
            let key = format!("mod.{id}");
            list.lines()
                .filter_map(|l| l.split_once(':'))
                .any(|(k, v)| k.trim() == key && v.trim() == "true")
        };
        for (dir, ext) in [("horizon-mods", ".hmod"), ("horizon-native-mods", ".hnmod")] {
            for f in read_dir(&home.join(dir)) {
                let name = f.file_name().unwrap_or_default().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    continue;
                }
                let id = name.strip_suffix(ext).unwrap_or(&name).to_owned();
                let (title, version) = named(&format!("{dir}/{name}"), &id);
                out.push(json!({ "id": id, "title": title, "version": version, "on": on(&id), "native": dir == "horizon-native-mods" }));
            }
        }
    }
    out.sort_by_key(|m| m["title"].as_str().unwrap_or("").to_lowercase());
    out
}

fn read_dir(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|d| d.flatten().map(|e| e.path()).collect())
        .unwrap_or_default()
}

fn toggle_mod(home: &Path, java: bool, id: &str, on: bool) -> Result<(), String> {
    if id.contains('/') || id.contains('\\') || id.contains("..") {
        return Err("Ungültiger Mod-Name".into());
    }
    if java {
        let (jar, off) = (
            home.join("mods").join(id),
            home.join("mods").join(format!("{id}.disabled")),
        );
        let (from, to) = if on { (off, jar) } else { (jar, off) };
        return std::fs::rename(&from, &to).map_err(|e| format!("{}: {e}", from.display()));
    }
    let path = home.join("horizon-mods.txt");
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let key = format!("mod.{id}");
    let mut lines: Vec<String> = text
        .lines()
        .filter(|l| l.split_once(':').is_none_or(|(k, _)| k.trim() != key))
        .map(str::to_owned)
        .collect();
    if on {
        lines.push(format!("{key}:true"));
    }
    let mut out = lines.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    std::fs::write(&path, out).map_err(|e| format!("{}: {e}", path.display()))
}

fn add_mods(home: &Path, java: bool) -> Result<Value, String> {
    let dialog = rfd::FileDialog::new();
    let dialog = if java {
        dialog.add_filter("Java-Mod", &["jar"])
    } else {
        dialog.add_filter("Horizon-Mod", &["hmod"])
    };
    let Some(files) = dialog.pick_files() else {
        return Ok(json!(0));
    };
    let dir = home.join(if java { "mods" } else { "horizon-mods" });
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for f in &files {
        let name = f.file_name().ok_or("Kein Dateiname")?;
        std::fs::copy(f, dir.join(name)).map_err(|e| format!("{}: {e}", f.display()))?;
        if !java {
            let id = name.to_string_lossy();
            toggle_mod(home, false, id.trim_end_matches(".hmod"), true)?;
        }
    }
    Ok(json!(files.len()))
}

// --- packs -----------------------------------------------------------------------------------

/// Puts the instance's resource packs in this order, the first on top: the launcher turns a pack
/// on at the top, so they go on from the bottom up. `off` are turned off.
fn pack_order(args: &Value) -> Result<Value, String> {
    let instance = s(args, "instance");
    let names = |key: &str| -> Vec<String> {
        args[key]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
            .unwrap_or_default()
    };
    for file in names("off") {
        cli(&["packs", "off", &file, "--kind", "resource", "--instance", instance])?;
    }
    for file in names("order").iter().rev() {
        cli(&["packs", "off", file, "--kind", "resource", "--instance", instance])?;
        cli(&["packs", "on", file, "--kind", "resource", "--instance", instance])?;
    }
    Ok(Value::Null)
}

// --- accounts and skins ----------------------------------------------------------------------

/// Microsoft sign-in with a code: the launcher prints a link and a code, the page shows them and
/// the browser opens; the answer comes when the player has signed in there.
fn sign_in(proxy: &Proxy) -> Result<Value, String> {
    let mut child = launcher(&["account", "sign-in"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let err = child.stderr.take().unwrap();
    let mut last = String::new();
    for line in BufReader::new(err).lines().map_while(Result::ok) {
        if let Some(rest) = line.split("open ").nth(1) {
            let mut words = rest.split_whitespace();
            let link = words.next().unwrap_or("").to_owned();
            let code = rest.rsplit(' ').next().unwrap_or("").to_owned();
            if link.starts_with("http") {
                emit(proxy, "signin", json!({ "link": link, "code": code }));
                let _ = open(&link);
                continue;
            }
        }
        if !line.trim().is_empty() {
            last = line;
        }
    }
    let mut out = String::new();
    if let Some(mut o) = child.stdout.take() {
        let _ = o.read_to_string(&mut out);
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(json!(out.trim()))
    } else {
        Err(last.trim_start_matches("error: ").to_owned())
    }
}

/// An offline name's skin: the page sends the PNG as a data URL.
fn set_skin(args: &Value) -> Result<Value, String> {
    let name = s(args, "name");
    let mut a: Vec<String> = vec!["account".into(), "skin".into(), name.into()];
    let mut temp = Vec::new();
    for (key, flag) in [("skin", None), ("cape", Some("--cape"))] {
        let Some(url) = args[key].as_str().filter(|u| !u.is_empty()) else {
            continue;
        };
        let png = data_url_bytes(url)?;
        let path = std::env::temp_dir().join(format!("xd-client-{key}-{}.png", std::process::id()));
        std::fs::write(&path, png).map_err(|e| e.to_string())?;
        if let Some(flag) = flag {
            a.push(flag.into());
        }
        a.push(path.display().to_string());
        temp.push(path);
    }
    if args["noCape"] == true {
        a.push("--no-cape".into());
    }
    if let Some(model) = args["model"].as_str() {
        a.push(format!("--model={model}"));
    }
    let a: Vec<&str> = a.iter().map(String::as_str).collect();
    let r = cli(&a);
    temp.iter().for_each(|p| drop(std::fs::remove_file(p)));
    r
}

/// A Microsoft account's current skin, from Mojang's public profile service, as a data URL.
fn account_skin(uuid: &str) -> Result<Value, String> {
    let uuid: String = uuid.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    let text = ureq::get(&format!(
        "https://sessionserver.mojang.com/session/minecraft/profile/{uuid}"
    ))
    .call()
    .map_err(|e| e.to_string())?
    .into_string()
    .map_err(|e| e.to_string())?;
    let profile: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let textures = profile["properties"]
        .as_array()
        .and_then(|p| p.iter().find(|p| p["name"] == "textures"))
        .and_then(|p| p["value"].as_str())
        .ok_or("Kein Skin")?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(textures)
        .map_err(|e| e.to_string())?;
    let t: Value = serde_json::from_slice(&decoded).map_err(|e| e.to_string())?;
    let url = t["textures"]["SKIN"]["url"].as_str().ok_or("Kein Skin")?;
    let slim = t["textures"]["SKIN"]["metadata"]["model"] == "slim";
    let mut png = Vec::new();
    ureq::get(&url.replace("http://", "https://"))
        .call()
        .map_err(|e| e.to_string())?
        .into_reader()
        .take(1 << 20)
        .read_to_end(&mut png)
        .map_err(|e| e.to_string())?;
    Ok(json!({
        "skin": format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png)),
        "slim": slim,
    }))
}

fn data_url_bytes(url: &str) -> Result<Vec<u8>, String> {
    let b64 = url.split_once(',').map(|(_, b)| b).ok_or("Kein Bild")?;
    base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| e.to_string())
}

fn pick_image() -> Result<Value, String> {
    let Some(file) = rfd::FileDialog::new()
        .add_filter("Bild", &["png", "gif", "webp"])
        .pick_file()
    else {
        return Ok(Value::Null);
    };
    let bytes = std::fs::read(&file).map_err(|e| e.to_string())?;
    if bytes.len() > 1 << 20 {
        return Err("Das Bild ist größer als 1 MB.".into());
    }
    let mime = match file.extension().and_then(|e| e.to_str()).unwrap_or("png") {
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "image/png",
    };
    Ok(json!({
        "name": file.file_stem().map(|s| s.to_string_lossy().into_owned()),
        "url": format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)),
    }))
}

// --- xD Client's own settings and the system ----------------------------------------------------

fn settings_path() -> PathBuf {
    let base = if cfg!(target_os = "macos") {
        std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join("Library/Application Support"))
            .unwrap_or_default()
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_default()
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .unwrap_or_default()
    };
    let dir = std::env::var_os("XD_CLIENT_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| base.join("xD Client"));
    dir.join("settings.json")
}

fn load_settings() -> Value {
    std::fs::read_to_string(settings_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| json!({}))
}

fn save_settings(v: &Value) -> Result<(), String> {
    let path = settings_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let part = path.with_extension("json.part");
    std::fs::write(&part, v.to_string()).map_err(|e| e.to_string())?;
    std::fs::rename(&part, &path).map_err(|e| e.to_string())
}

fn total_ram_gb() -> f64 {
    #[cfg(target_os = "macos")]
    {
        if let Ok(out) = Command::new("sysctl").args(["-n", "hw.memsize"]).output() {
            if let Ok(b) = String::from_utf8_lossy(&out.stdout).trim().parse::<f64>() {
                return (b / 1073741824.0).round();
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(t) = std::fs::read_to_string("/proc/meminfo") {
            if let Some(kb) = t
                .lines()
                .find(|l| l.starts_with("MemTotal:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|n| n.parse::<f64>().ok())
            {
                return (kb / 1048576.0).round();
            }
        }
    }
    8.0
}

/// The Java home of a picked `java` program (`…/bin/java`) or folder.
fn java_home_of(p: &Path) -> PathBuf {
    if p.file_name().is_some_and(|n| n == "java" || n == "java.exe") {
        if let Some(home) = p.parent().and_then(Path::parent) {
            return home.to_path_buf();
        }
    }
    p.to_path_buf()
}

/// Opens a folder, a file or a link the way the Finder would. On macOS straight through
/// NSWorkspace: starting `open` takes seconds now and then.
fn open(target: &str) -> Result<(), String> {
    if target.is_empty() {
        return Err("Nichts zu öffnen".into());
    }
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSWorkspace;
        use objc2_foundation::{NSString, NSURL};
        let url = if target.starts_with("http://") || target.starts_with("https://") {
            NSURL::URLWithString(&NSString::from_str(target))
        } else {
            NSURL::from_file_path(target)
        };
        let url = url.ok_or_else(|| format!("{target} lässt sich nicht öffnen"))?;
        return match NSWorkspace::sharedWorkspace().openURL(&url) {
            true => Ok(()),
            false => Err(format!("{target} lässt sich nicht öffnen")),
        };
    }
    #[allow(unreachable_code)]
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    Command::new(program)
        .arg(target)
        .spawn()
        .map(drop)
        .map_err(|e| e.to_string())
}

// --- the catalogs: Horizon's registry and Modrinth -------------------------------------------

/// Sent with every request to Modrinth and the registry, as Modrinth asks of every client.
const AGENT: &str = concat!("xd-client/", env!("CARGO_PKG_VERSION"), " (github.com/MaKn06/horizon)");
const REGISTRY: &str = "https://horizon-rust.github.io/horizon-mods/index.json";
const MODRINTH: &str = "https://api.modrinth.com/v2";

fn get_json(url: &str) -> Result<Value, String> {
    let text = ureq::get(url)
        .set("User-Agent", AGENT)
        .timeout(std::time::Duration::from_secs(20))
        .call()
        .map_err(|e| format!("{url}: {e}"))?
        .into_string()
        .map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

/// Horizon's registry index, fetched at most every ten minutes.
fn registry() -> Result<Value, String> {
    static CACHE: Mutex<Option<(std::time::Instant, Value)>> = Mutex::new(None);
    let mut cache = CACHE.lock().unwrap();
    if let Some((at, v)) = cache.as_ref() {
        if at.elapsed().as_secs() < 600 {
            return Ok(v.clone());
        }
    }
    let v = get_json(REGISTRY)?;
    *cache = Some((std::time::Instant::now(), v.clone()));
    Ok(v)
}

/// Projects for one page of the browser: `{id, slug, title, summary, author, downloads, icon,
/// page}`. Horizon's registry for Horizon instances; Modrinth, for the instance's Minecraft
/// version (and its loader, for mods), for Java instances. `sort` is "relevance" or
/// "downloads".
fn catalog(args: &Value) -> Result<Value, String> {
    let kind = s(args, "kind");
    let query = s(args, "query").to_lowercase();
    let limit = args["limit"].as_u64().unwrap_or(30).min(100);
    let offset = args["offset"].as_u64().unwrap_or(0);
    if s(args, "source") == "horizon" {
        let index = registry()?;
        let mut found: Vec<Value> = index["projects"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|p| p["kind"] == kind)
            .filter(|p| {
                query.is_empty()
                    || [&p["name"], &p["summary"], &p["id"]]
                        .iter()
                        .any(|f| f.as_str().unwrap_or("").to_lowercase().contains(&query))
                    || p["tags"]
                        .as_array()
                        .is_some_and(|t| t.iter().any(|t| t.as_str() == Some(query.as_str())))
            })
            .map(|p| {
                json!({
                    "id": p["id"], "slug": p["id"], "title": p["name"], "summary": p["summary"],
                    "author": p["authors"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")),
                    "downloads": p["downloads"].as_u64().unwrap_or(0), "icon": p["icon"],
                    "page": format!("https://playhorizon.dev/mods/{}", p["id"].as_str().unwrap_or("")),
                })
            })
            .collect();
        if s(args, "sort") == "downloads" || query.is_empty() {
            found.sort_by_key(|p| std::cmp::Reverse(p["downloads"].as_u64().unwrap_or(0)));
        }
        return Ok(json!(found
            .into_iter()
            .skip(offset as usize)
            .take(limit as usize)
            .collect::<Vec<_>>()));
    }
    let project_type = match kind {
        "resourcepack" => "resourcepack",
        "shader" => "shader",
        _ => "mod",
    };
    let mut facets = vec![json!([format!("project_type:{project_type}")])];
    if !s(args, "mc").is_empty() {
        facets.push(json!([format!("versions:{}", s(args, "mc"))]));
    }
    if project_type == "mod" && !s(args, "loader").is_empty() {
        facets.push(json!([format!("categories:{}", s(args, "loader"))]));
    }
    let index = if s(args, "sort") == "downloads" || query.is_empty() {
        "downloads"
    } else {
        "relevance"
    };
    let url = format!(
        "{MODRINTH}/search?query={}&facets={}&index={index}&limit={limit}&offset={offset}",
        enc(&query),
        enc(&Value::Array(facets).to_string())
    );
    let found = get_json(&url)?;
    Ok(json!(found["hits"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|h| json!({
            "id": h["project_id"], "slug": h["slug"], "title": h["title"], "summary": h["description"],
            "author": h["author"], "downloads": h["downloads"], "icon": h["icon_url"],
            "page": format!("https://modrinth.com/{project_type}/{}", h["slug"].as_str().unwrap_or("")),
        }))
        .collect::<Vec<_>>()))
}

/// Percent-encoding for a query string value.
fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Installs a project into an instance. Mods and resource packs go through the Horizon
/// launcher (`mods install`: the version for the instance, what it needs, the checksums and
/// the registry's signatures). A Java instance's shader pack, which the launcher doesn't
/// install, is downloaded here: Modrinth's newest file for its Minecraft version, checked
/// against its SHA-512, into `shaderpacks/`.
fn install(args: &Value, proxy: &Proxy) -> Result<Value, String> {
    let (source, id, kind, instance) = (
        s(args, "source"),
        s(args, "id"),
        s(args, "kind"),
        s(args, "instance"),
    );
    if source == "modrinth" && kind == "shader" {
        return install_modrinth_file(id, s(args, "mc"), &Path::new(s(args, "home")).join("shaderpacks"));
    }
    run_logged(
        proxy,
        &["mods", "install", id, "--source", source, "--instance", instance],
        "install",
    )
    .map(|_| Value::Null)
}

fn install_modrinth_file(id: &str, mc: &str, dir: &Path) -> Result<Value, String> {
    use sha2::Digest;
    let id: String = id.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    let mut url = format!("{MODRINTH}/project/{id}/version");
    if !mc.is_empty() {
        url.push_str(&format!("?game_versions={}", enc(&json!([mc]).to_string())));
    }
    let versions = get_json(&url)?;
    let version = versions
        .as_array()
        .and_then(|v| v.first())
        .ok_or("Keine Version für diese Minecraft-Version")?;
    let files = version["files"].as_array().ok_or("Keine Datei")?;
    let file = files
        .iter()
        .find(|f| f["primary"] == true)
        .or(files.first())
        .ok_or("Keine Datei")?;
    let (name, link, sha) = (
        file["filename"].as_str().unwrap_or(""),
        file["url"].as_str().unwrap_or(""),
        file["hashes"]["sha512"].as_str().unwrap_or(""),
    );
    if name.is_empty() || name.contains('/') || name.contains('\\') || name.starts_with('.') {
        return Err("Ungültiger Dateiname".into());
    }
    if !link.starts_with("https://cdn.modrinth.com/") {
        return Err("Die Datei liegt nicht bei Modrinth".into());
    }
    let mut bytes = Vec::new();
    ureq::get(link)
        .set("User-Agent", AGENT)
        .call()
        .map_err(|e| e.to_string())?
        .into_reader()
        .take(256 << 20)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let got: String = sha2::Sha512::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if got != sha.to_lowercase() {
        return Err("Die Prüfsumme stimmt nicht: die Datei wurde nicht gespeichert".into());
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(name), bytes).map_err(|e| e.to_string())?;
    Ok(json!(name))
}

// --- local pictures (screenshots, world icons) for the page ----------------------------------

/// Answers `xdfile://localhost/<path>`: a PNG or JPEG inside a game folder's `screenshots/` or
/// `saves/`, and nothing else.
pub fn local_file(
    req: wry::http::Request<Vec<u8>>,
) -> wry::http::Response<std::borrow::Cow<'static, [u8]>> {
    let raw = req.uri().path();
    let path = PathBuf::from(percent_decode(raw));
    let allowed = path.is_absolute()
        && !path.components().any(|c| matches!(c, std::path::Component::ParentDir))
        && path
            .components()
            .any(|c| c.as_os_str() == "screenshots" || c.as_os_str() == "saves")
        && matches!(
            path.extension().and_then(|e| e.to_str()).map(str::to_lowercase).as_deref(),
            Some("png" | "jpg" | "jpeg")
        );
    let body = allowed.then(|| std::fs::read(&path).ok()).flatten();
    let mime = if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")) {
        "image/png"
    } else {
        "image/jpeg"
    };
    match body {
        Some(bytes) => wry::http::Response::builder()
            .header("Content-Type", mime)
            .header("Cache-Control", "max-age=600")
            .body(std::borrow::Cow::Owned(bytes))
            .unwrap(),
        None => wry::http::Response::builder()
            .status(404)
            .body(std::borrow::Cow::Borrowed(&b""[..]))
            .unwrap(),
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
