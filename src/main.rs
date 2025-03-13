use std::{
    collections::HashMap, env, path::{Path, PathBuf}, time::Duration
};

use anyhow::Context;
use chrono::{DateTime, Local, NaiveDateTime, TimeZone, Utc};
use chrono_tz::{OffsetName, Tz};
use clap::Parser;
use iana_time_zone::get_timezone;
use rusqlite::Connection;
use reqwest::blocking::Client;
use serde_json::{
    json,
    Value
};

macro_rules! debug_println {
    ($($arg:tt)*) => (if ::std::cfg!(debug_assertions) { ::std::println!($($arg)*); })
}

#[cfg(target_os = "windows")]
const SUBTRACKER_FOLDER: &str = r#"AppData\Roaming\XIVLauncher\pluginConfigs\SubmarineTracker"#;
#[cfg(target_os = "linux")]
const SUBTRACKER_FOLDER: &str = ".xlcore/pluginConfigs/SubmarineTracker";
#[cfg(target_os = "macos")]
const SUBTRACKER_FOLDER: &str = "Library/Application Support/XIV on Mac/pluginConfigs/SubmarineTracker";

#[derive(Parser, Debug)]
#[command(version)]
struct LaunchArgs {
    #[arg(short, long)]
    daemon: bool,
    #[arg(short, long)]
    update: Option<String>,
}

fn main_daemon() -> anyhow::Result<()> {
    use notify_rust::Notification;

    // Not proud of this but it meets my needs ok
    let bridge_psk: &'static str = env!("PUSHOVER_BRIDGE_PSK");
    let bridge_url: &'static str = env!("PUSHOVER_BRIDGE_URL");
    let client = Client::new();

    let mut notifs_data: HashMap<i64, NotifyMeta> = HashMap::new();
    let db = open_db(None)?;
    loop {
        let subs = get_submarine_info(&db)?;
        let mut bridge_json_payload = serde_json::Map::new();
        let mut subs_in_group: u32 = 0;
        let mut previous_return_time: Option<DateTime<Utc>> = None;
        let mut current_pushover_notif: Option<Value> = None;
        let mut current_id = "".to_string();
        let mut message_count: u32 = 0;
        let has_changes = subs.iter().all(|sub| {
            let meta = notifs_data
            .get(&sub.id)
            .cloned()
            .unwrap_or_else(|| NotifyMeta {
                submarine_id: sub.id,
                will_notify: true,
                last_return_time: Default::default(),
            });
            meta.last_return_time != sub.return_time && sub.return_time > Local::now()
        });
    
        if !has_changes {
            std::thread::sleep(Duration::from_secs(1));
            continue;
        }

        for sub in subs {
            let mut meta = notifs_data
                .get(&sub.id)
                .cloned()
                .unwrap_or_else(|| NotifyMeta {
                    submarine_id: sub.id,
                    will_notify: true,
                    last_return_time: Default::default(),
                });
            if meta.last_return_time != sub.return_time && sub.return_time > Local::now() {
                meta.will_notify = true;
                meta.last_return_time = sub.return_time;
                let time = sub.return_time.with_timezone(&Local);
                debug_println!(
                    "notification scheduled for {subname} {time}",
                    subname = sub.name
                );
            }

            if meta.will_notify && sub.return_time <= Local::now() {
                meta.will_notify = false;
                let summary = format!("{name} returned", name = sub.name);
                let time = sub.return_time.with_timezone(&Local);
                let time_str = time.format("%b %e, %Y, %I:%M%p").to_string();
                let body = format!(
                    "{name} ({char_name} «{tag}») returned on {time_str}",
                    name = sub.name,
                    char_name = sub.character_name,
                    tag = sub.tag
                );
                Notification::new()
                    .summary(&summary)
                    .body(&body)
                    .icon("dialog-information")
                    .show()?;
            }
            notifs_data.insert(sub.id, meta);

            if sub.return_time > Local::now() {
                // Add a notification object to the pushover bridge API JSON payload
                subs_in_group += 1;
                let time = sub.return_time.with_timezone(&Local);
                let time_str = time.format("%b %e, %Y, %I:%M%p").to_string();
                let body = if subs_in_group > 1 {
                    format!(
                        "{name} ({char_name} «{tag}») + {num} others returned on {time_str}",
                        name = sub.name,
                        char_name = sub.character_name,
                        tag = sub.tag,
                        num = subs_in_group - 1
                    )
                } else {
                    format!(
                        "{name} ({char_name} «{tag}») returned on {time_str}",
                        name = sub.name,
                        char_name = sub.character_name,
                        tag = sub.tag
                    )
                };

                let title = if subs_in_group > 1 {
                    format!("{name} (+{num}) returned", name = sub.name, num = subs_in_group - 1)
                } else {
                    format!("{name} returned", name = sub.name)
                };
                
                let pushover_notif = json!({
                    "title": title,
                    "message": body,
                    "timestamp": sub.return_time.timestamp_millis()
                });
                current_id = format!("{char_name}«{tag}»-{message_count}", char_name = sub.character_name, tag = sub.tag);
                if let Some(prev_time) = previous_return_time {
                    if sub.return_time.timestamp_millis() - prev_time.timestamp_millis() > 300000 {
                        bridge_json_payload.insert(current_id.clone(), current_pushover_notif.unwrap());
                        previous_return_time = Some(sub.return_time);
                        current_pushover_notif = Some(pushover_notif);
                        subs_in_group = 1;
                        message_count += 1;
                    } else {
                        previous_return_time = Some(sub.return_time);
                        current_pushover_notif = Some(pushover_notif);
                    }
                } else {
                    previous_return_time = Some(sub.return_time);
                    current_pushover_notif = Some(pushover_notif);
                }
            }
        }
        if let Some(dangling_push_notif) = current_pushover_notif {
            bridge_json_payload.insert(current_id, dangling_push_notif);
        }
        if !bridge_json_payload.is_empty() {
            let payload = Value::Object(bridge_json_payload);
            debug_println!("pushover bridge json: {}", payload);
            client
                .post(bridge_url)
                .header("Authorization", format!("Bearer {}", bridge_psk))
                .json(&payload)
                .send()?;
            // ... and honestly don't care about the response. It either keeps working or it ain't
        }

        std::thread::sleep(Duration::from_secs(1));
    }
}

fn main() -> anyhow::Result<()> {
    let args = LaunchArgs::parse();
    if args.daemon {
        return main_daemon();
    }
    if let Some(updated) = args.update {
        let parse_date = NaiveDateTime::parse_from_str(&updated, "%m/%d/%Y %H:%M")
            .with_context(|| format!("Date format incorrect for '{}', FFXIV format expected\n\nExample: 11/14/2024 16:59", updated))?
            .and_local_timezone(Local)
            .unwrap();
        let updated_timestamp = parse_date.timestamp();
        let db = open_db(Some(rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE))?;
        db.execute("UPDATE submarine SET Return = (?1)", [updated_timestamp])?;
        db.close().unwrap();
        println!("All submarine return times updated! These are the new return times...");
    }

    let tz_str = mysql_real_get_timezone().unwrap();
    let tz: Tz = tz_str.parse().unwrap();
    let offset = tz.offset_from_utc_date(&Utc::now().date_naive());
    let tz_abbr = offset.abbreviation();
    let db = open_db(None)?;
    let all_subs = get_submarine_info(&db)?;
    let longest_name = all_subs.iter().map(|s| s.name.len()).max().unwrap_or(0);
    let mut subs_by_char: HashMap<String, Vec<SubInfo>> = HashMap::new();
    for sub in all_subs {
        let char_ident = format!(
            "{name} «{fc_tag}»",
            name = sub.character_name,
            fc_tag = sub.tag
        );
        subs_by_char
            .entry(char_ident)
            .or_insert_with(Vec::new)
            .push(sub);
    }
    for (char, subs) in subs_by_char {
        println!("{char}:");
        for sub in subs {
            let padding = " ".repeat(longest_name - sub.name.len());
            let time = sub.return_time.with_timezone(&Local);
            let time_str = time.format("%e %B %Y at %I:%M:%S %p").to_string();
            println!("  {name}:{padding} {time_str} {tz_abbr}", name = sub.name);
        }
    }

    Ok(())
}

fn mysql_real_get_timezone() -> Option<String> {
    // first check for TZ since upstream doesn't
    let env_tz = env::var("TZ").ok();
    let tz = env_tz.or(get_timezone().ok());
    return tz;
}

fn open_db(flags: Option<rusqlite::OpenFlags>) -> anyhow::Result<Connection> {
    let user_dirs = directories::UserDirs::new().unwrap();
    let sub_db_file: PathBuf = [
        user_dirs.home_dir(),
        Path::new(SUBTRACKER_FOLDER),
        Path::new("submarine-sqlite.db"),
    ]
    .iter()
    .collect();
    let db = Connection::open_with_flags(
        sub_db_file,
        flags.unwrap_or(rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY),
    )?;
    Ok(db)
}

fn get_submarine_info(db: &Connection) -> anyhow::Result<Vec<SubInfo>> {
    let query = "
    SELECT
        submarine.SubmarineId AS id,
        submarine.Name AS name, 
        submarine.Return AS return_time, 
        freecompany.FreeCompanyTag AS tag, 
        freecompany.CharacterName AS character_name
    FROM submarine
    JOIN freecompany
    ON submarine.FreeCompanyId = freecompany.FreeCompanyId
    ORDER BY return_time ASC
    ";
    let mut stmt = db.prepare(query)?;
    let subs: Vec<SubInfo> = stmt
        .query_map([], |row| {
            let timestamp: i64 = row.get(2)?;
            Ok(SubInfo {
                id: row.get(0)?,
                name: row.get(1)?,
                return_time: Utc.timestamp_opt(timestamp, 0).single().unwrap(),
                tag: row.get(3)?,
                character_name: row.get(4)?,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();
    Ok(subs)
}

#[derive(Clone)]
pub struct NotifyMeta {
    pub submarine_id: i64,
    pub will_notify: bool,
    pub last_return_time: DateTime<Utc>,
}

pub struct SubInfo {
    pub id: i64,
    pub name: String,
    pub return_time: DateTime<Utc>,
    pub tag: String,
    pub character_name: String,
}
