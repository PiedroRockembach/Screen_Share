//! Copies every app's audio, except Discord's, into a silent sink that the
//! sender captures. Apps keep playing where they were (speakers, EasyEffects),
//! so nothing changes for the user. If anything fails, an app just isn't
//! captured: Discord's voice never reaches the stream.

use regex::Regex;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

pub const SINK: &str = "sala_de_tela";

static DISCORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)discord|vesktop|webcord|legcord|armcord").unwrap());

pub struct AudioRouting {
    module: String,
    stop: Arc<AtomicBool>,
    subscriber: Option<Child>,
}

impl AudioRouting {
    pub fn start() -> Option<Self> {
        remove_stale_sinks();
        let output = Command::new("pactl")
            .args([
                "load-module",
                "module-null-sink",
                &format!("sink_name={SINK}"),
                "rate=48000",
                "channels=2",
                "channel_map=front-left,front-right",
                "sink_properties='device.description=Sala-de-Tela priority.session=1 priority.driver=1'",
            ])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let module = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        let stop = Arc::new(AtomicBool::new(false));

        let mut subscriber = Command::new("pactl").arg("subscribe").stdout(Stdio::piped()).spawn().ok();
        if let Some(stdout) = subscriber.as_mut().and_then(|c| c.stdout.take()) {
            let stop = stop.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    if line.contains("sink-input") || line.contains("'server'") {
                        // The app's ports appear right after the event.
                        std::thread::sleep(Duration::from_millis(150));
                        adjust();
                    }
                }
            });
        }
        {
            // Native PipeWire apps don't always generate pulse events.
            let stop = stop.clone();
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    adjust();
                    std::thread::sleep(Duration::from_secs(2));
                }
            });
        }
        Some(Self { module, stop, subscriber })
    }
}

impl Drop for AudioRouting {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(c) = self.subscriber.as_mut() {
            let _ = c.kill();
            let _ = c.wait();
        }
        let _ = Command::new("pactl").args(["unload-module", &self.module]).status();
    }
}

fn remove_stale_sinks() {
    // pactl's JSON module list has no index; the short list does.
    let Ok(out) = Command::new("pactl").args(["list", "short", "modules"]).output() else {
        return;
    };
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() >= 3 && fields[1] == "module-null-sink" && line.contains(&format!("sink_name={SINK} ")) {
            let _ = Command::new("pactl").args(["unload-module", fields[0]]).status();
        }
    }
}

fn ancestor_executables(mut pid: u32) -> Vec<String> {
    let mut names = Vec::new();
    for _ in 0..32 {
        if pid <= 1 {
            break;
        }
        let Ok(cmdline) = std::fs::read(format!("/proc/{pid}/cmdline")) else { break };
        let argv0 = cmdline.split(|b| *b == 0).next().unwrap_or_default();
        let argv0 = String::from_utf8_lossy(argv0);
        names.push(argv0.rsplit('/').next().unwrap_or_default().to_owned());
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else { break };
        // The process name may contain spaces and parentheses: ppid comes after the last ')'.
        let after = stat.rsplit_once(')').map(|(_, a)| a).unwrap_or_default();
        pid = after.split_whitespace().nth(1).and_then(|p| p.parse().ok()).unwrap_or(0);
    }
    names
}

fn is_discord(props: &HashMap<String, String>) -> bool {
    let by_props = props.iter().any(|(k, v)| {
        (k.starts_with("application.") || k == "pipewire.access.portal.app_id") && DISCORD.is_match(v)
    });
    by_props
        || props
            .get("application.process.id")
            .and_then(|p| p.parse().ok())
            .is_some_and(|pid| ancestor_executables(pid).iter().any(|n| DISCORD.is_match(n)))
}

/// Audio programs that aren't apps: copying their output would duplicate sound.
fn is_not_an_app(props: &HashMap<String, String>) -> bool {
    if props.get("node.name").is_some_and(|n| n.starts_with("sala_")) || props.contains_key("node.link-group") {
        return true;
    }
    let ids = format!(
        "{} {} {}",
        props.get("application.process.binary").map(String::as_str).unwrap_or_default(),
        props.get("application.name").map(String::as_str).unwrap_or_default(),
        props.get("application.id").map(String::as_str).unwrap_or_default()
    )
    .to_lowercase();
    ["easyeffects", "pulseeffects", "obs", "gst-launch", "sala-de-tela"].iter().any(|n| ids.contains(n))
}

fn as_strings(props: &Value) -> HashMap<String, String> {
    props
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(k, v)| {
            let s = match v {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                _ => return None,
            };
            Some((k.clone(), s))
        })
        .collect()
}

fn target_channels(channel: &str) -> &'static [&'static str] {
    match channel {
        "FL" | "RL" | "SL" | "FLC" | "RLC" => &["FL"],
        "FR" | "RR" | "SR" | "FRC" | "RRC" => &["FR"],
        _ => &["FL", "FR"],
    }
}

fn adjust() {
    let Ok(out) = Command::new("pw-dump").output() else { return };
    let Ok(Value::Array(objects)) = serde_json::from_slice::<Value>(&out.stdout) else { return };

    let mut nodes: HashMap<u64, HashMap<String, String>> = HashMap::new();
    for o in &objects {
        if o["type"] == "PipeWire:Interface:Node" {
            if let Some(id) = o["id"].as_u64() {
                nodes.insert(id, as_strings(&o["info"]["props"]));
            }
        }
    }
    let mut sink_inputs: HashMap<String, u64> = HashMap::new();
    let mut app_outputs: Vec<(u64, u64, String)> = Vec::new();
    for o in &objects {
        if o["type"] != "PipeWire:Interface:Port" {
            continue;
        }
        let p = as_strings(&o["info"]["props"]);
        let (Some(id), Some(node)) = (o["id"].as_u64(), p.get("node.id").and_then(|n| n.parse::<u64>().ok())) else {
            continue;
        };
        let Some(np) = nodes.get(&node) else { continue };
        let direction = p.get("port.direction").map(String::as_str);
        let channel = p.get("audio.channel").cloned().unwrap_or_default();
        if np.get("node.name").map(String::as_str) == Some(SINK) && direction == Some("in") {
            sink_inputs.insert(channel, id);
        } else if np.get("media.class").map(String::as_str) == Some("Stream/Output/Audio")
            && direction == Some("out")
            && p.get("port.monitor").map(String::as_str) != Some("true")
        {
            app_outputs.push((id, node, channel));
        }
    }
    if sink_inputs.is_empty() {
        return;
    }
    let links: HashSet<(u64, u64)> = objects
        .iter()
        .filter(|o| o["type"] == "PipeWire:Interface:Link")
        .filter_map(|o| Some((o["info"]["output-port-id"].as_u64()?, o["info"]["input-port-id"].as_u64()?)))
        .collect();

    let mut discord_cache: HashMap<u64, bool> = HashMap::new();
    for (port, node, channel) in app_outputs {
        let props = &nodes[&node];
        let discord = *discord_cache.entry(node).or_insert_with(|| is_discord(props));
        for target in target_channels(&channel) {
            let Some(&input) = sink_inputs.get(*target) else { continue };
            let linked = links.contains(&(port, input));
            if discord && linked {
                let _ = Command::new("pw-link").args(["-d", &port.to_string(), &input.to_string()]).status();
            } else if !discord && !linked && !is_not_an_app(props) {
                let _ = Command::new("pw-link").args([&port.to_string(), &input.to_string()]).status();
            }
        }
    }
    move_apps_out_of_sink();
}

/// Nothing should play directly into the silent sink; whatever ended up there
/// (remembered by WirePlumber, or chosen by hand) goes back to the default output.
fn move_apps_out_of_sink() {
    let pactl_json = |args: &[&str]| -> Option<Value> {
        let out = Command::new("pactl").args(["-f", "json"]).args(args).output().ok()?;
        serde_json::from_slice(&out.stdout).ok()
    };
    let (Some(sinks), Some(inputs)) = (pactl_json(&["list", "sinks"]), pactl_json(&["list", "sink-inputs"])) else {
        return;
    };
    let Some(sink_index) = sinks
        .as_array()
        .into_iter()
        .flatten()
        .find(|s| s["name"] == SINK)
        .and_then(|s| s["index"].as_u64())
    else {
        return;
    };
    let default = Command::new("pactl").arg("get-default-sink").output().ok();
    let default = default.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned()).unwrap_or_default();
    if default.is_empty() || default == SINK {
        return;
    }
    for input in inputs.as_array().into_iter().flatten() {
        if input["sink"].as_u64() == Some(sink_index) && !is_not_an_app(&as_strings(&input["properties"])) {
            if let Some(index) = input["index"].as_u64() {
                let _ = Command::new("pactl").args(["move-sink-input", &index.to_string(), &default]).status();
            }
        }
    }
}
