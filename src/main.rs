use std::{
    ffi::OsStr,
    fs::read_to_string,
    fs::metadata,
    path::{Path, PathBuf}, 
    collections::HashMap,
    time::SystemTime,
    time::Duration,
};

use clap::Parser;
use chrono::{DateTime, Local, Utc, TimeZone};
use chrono_tz::{OffsetName, Tz};
use iana_time_zone::get_timezone;

macro_rules! debug_println {
    ($($arg:tt)*) => (if ::std::cfg!(debug_assertions) { ::std::println!($($arg)*); })
}

#[cfg(target_os = "windows")]
const SUBTRACKER_FOLDER: &str = r#"AppData\Roaming\XIVLauncher\pluginConfigs\SubmarineTracker"#;
#[cfg(target_os = "linux")]
const SUBTRACKER_FOLDER: &str = ".xlcore/pluginConfigs/SubmarineTracker";

#[derive(Parser, Debug)]
#[command(version)]
struct LaunchArgs {
    #[arg(short, long)]
    daemon: bool,
}

fn main_daemon() -> anyhow::Result<()> {
    use notify_rust::Notification;

    let user_dirs = directories::UserDirs::new().unwrap();
    let mut mtimes: HashMap<PathBuf, SystemTime> = HashMap::new();
    let mut chars: HashMap<u64, Character> = HashMap::new();
    loop {
        let sub_folder: PathBuf = [user_dirs.home_dir(), Path::new(SUBTRACKER_FOLDER)].iter().collect();
    
        for entry in sub_folder.read_dir()? {
            let Ok(entry) = entry else { continue };
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if !kind.is_file() {
                continue;
            }
            let path = entry.path();
            if path.extension() != Some(OsStr::new("json")) {
                continue;
            }
            let Ok(meta) = metadata(&path) else {
                eprintln!("Failed to stat {:?}", path);
                continue;
            };
            let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let last_mtime = mtimes.get(&path);
            if last_mtime.is_none() || last_mtime.is_some_and(|t| mtime > *t) {
                mtimes.insert(path.clone(), mtime);
                let Ok(contents) = read_to_string(&path) else {
                    eprintln!("Failed to open {:?}", path);
                    continue;
                };
                let Ok(mut data) = serde_json::from_str::<Character>(&contents) else {
                    eprintln!("Failed to deserialize {:?}", path);
                    continue;
                };
                debug_println!("reloading: {name}", name = data.character_name);
                for sub in &mut data.submarines {
                    if sub.return_time > Local::now() {
                        sub.will_notify = true;
                        let time = sub.return_time.with_timezone(&Local);
                        debug_println!("notification scheduled for {subname} {time}", subname = sub.name);
                    }
                }
                chars.insert(data.local_content_id, data);
            }
        }

        for (_, char_data) in &mut chars {
            for sub in &mut char_data.submarines {
                if sub.will_notify && sub.return_time <= Local::now() {
                    sub.will_notify = false;
                    let summary = format!("{name} returned", name = sub.name);
                    let time = sub.return_time.with_timezone(&Local);
                    let time_str = time.format("%b %e, %Y, %I:%M%p").to_string();
                    let body = format!(
                        "{name} ({char_name} «{tag}») returned on {time_str}", 
                        name = sub.name, 
                        char_name = char_data.character_name, 
                        tag = char_data.tag
                    );
                    Notification::new()
                        .summary(&summary)
                        .body(&body)
                        .icon("dialog-information")
                        .show()?;
                }
            }
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn main() -> anyhow::Result<()> {
    let args = LaunchArgs::parse();
    if args.daemon {
        return main_daemon();
    }
    let user_dirs = directories::UserDirs::new().unwrap();
    let sub_folder: PathBuf = [user_dirs.home_dir(), Path::new(SUBTRACKER_FOLDER)].iter().collect();
    let tz_str = get_timezone().unwrap();
    let tz: Tz = tz_str.parse().unwrap();
    let offset = tz.offset_from_utc_date(&Utc::now().date_naive());
    let tz_abbr = offset.abbreviation();

    for entry in sub_folder.read_dir()? {
        let Ok(entry) = entry else { continue };
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if !kind.is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension() != Some(OsStr::new("json")) {
            continue;
        }
        let Ok(contents) = read_to_string(&path) else {
            eprintln!("Failed to open {:?}", path);
            continue;
        };
        let Ok(mut data) = serde_json::from_str::<Character>(&contents) else {
            eprintln!("Failed to deserialize {:?}", path);
            continue;
        };

        data.submarines.sort_by(|a, b| a.return_time.cmp(&b.return_time));
        println!("{char} «{tag}»:", char = data.character_name, tag = data.tag);
        let longest_name = data.submarines
            .iter()
            .map(|s| s.name.len())
            .max()
            .unwrap_or(0);
        for sub in data.submarines {
            let padding = " ".repeat(longest_name - sub.name.len());
            let time = sub.return_time.with_timezone(&Local);
            let time_str = time.format("%e %B %Y at %I:%M:%S %p").to_string();
            println!("  {name}:{padding} {time_str} {tz_abbr}", name = sub.name);
        }
    }

    Ok(())
}

#[derive(serde::Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct Character {
    pub character_name: String,
    pub world: String,
    pub tag: String,
    pub local_content_id: u64,
    pub submarines: Vec<Submarine>,
}

#[derive(serde::Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct Submarine {
    pub name: String,
    pub return_time: DateTime<Utc>,
    #[serde(skip)]
    pub will_notify: bool,
}

