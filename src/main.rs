use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::Duration,
};

use chrono::{DateTime, Local, TimeZone, Utc};
use chrono_tz::{OffsetName, Tz};
use clap::Parser;
use iana_time_zone::get_timezone;
use rusqlite::Connection;

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

    let mut notifs_data: HashMap<i64, NotifyMeta> = HashMap::new();
    let db = open_db()?;
    loop {
        let subs = get_submarine_info(&db)?;
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
        }

        std::thread::sleep(Duration::from_secs(1));
    }
}

fn main() -> anyhow::Result<()> {
    let args = LaunchArgs::parse();
    if args.daemon {
        return main_daemon();
    }
    let tz_str = get_timezone().unwrap();
    let tz: Tz = tz_str.parse().unwrap();
    let offset = tz.offset_from_utc_date(&Utc::now().date_naive());
    let tz_abbr = offset.abbreviation();
    let db = open_db()?;
    let subs = get_submarine_info(&db)?;
    let longest_name = subs.iter().map(|s| s.name.len()).max().unwrap_or(0);
    for sub in subs {
        let padding = " ".repeat(longest_name - sub.name.len());
        let time = sub.return_time.with_timezone(&Local);
        let time_str = time.format("%e %B %Y at %I:%M:%S %p").to_string();
        println!("{name}:{padding} {time_str} {tz_abbr}", name = sub.name);
    }

    Ok(())
}

fn open_db() -> anyhow::Result<Connection> {
    let user_dirs = directories::UserDirs::new().unwrap();
    let sub_db_file: PathBuf = [
        user_dirs.home_dir(),
        Path::new(SUBTRACKER_FOLDER),
        Path::new("submarine-sqlite.db"),
    ]
    .iter()
    .collect();
    let db = Connection::open_with_flags(sub_db_file, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    Ok(db)
}

fn get_submarine_info(db: &Connection) -> anyhow::Result<Vec<SubInfo>> {
    let query = "
    SELECT
        submarine.SubmarineId as id,
        submarine.Name AS name, 
        submarine.Return AS return_time, 
        freecompany.FreeCompanyTag as tag, 
        freecompany.CharacterName as character_name
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
