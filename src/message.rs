use irc::client::prelude::*;
use std::collections::HashMap;
use std::str;
use std::error;
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

use super::http::{get_youtube_user, get_youtube_channel};
use super::config::Rtd;
use super::error::MyError;

static MAX_DOWNLOADERS: usize = 30;

pub fn handle_message(
    client: &IrcClient, message: &Message, rtd: &Rtd
) {
    // print the message if debug flag is set
    if rtd.args.flag_debug {
        eprintln!("{:?}", message.command)
    }

    // match on message type
    let (_target, msg) = match message.command {
        Command::PRIVMSG(ref target, ref msg) => (target, msg),
        _ => return,
    };

    let user = message.source_nickname().unwrap();

    let command_channel = &rtd.conf.params.command_channel;
    if message.response_target() == Some(command_channel) {
        match msg.as_ref() {
            "!help" => {
                client.send_privmsg(command_channel, get_help()).unwrap()
            },
            "!status" => {
                for message in get_status() {
                    client.send_privmsg(command_channel, message).unwrap()
                }
            },
            msg if msg.starts_with("!s ") => {
                let url = msg.splitn(2, ' ').last().unwrap();
                match do_stash_check(&url) {
                    Ok(reply) => client.send_privmsg(command_channel, format!("{}: {}", user, reply)).unwrap(),
                    Err(err)  => client.send_privmsg(command_channel, format!("{}: error: {}", user, err)).unwrap()
                }
            },
            msg if msg.starts_with("!a ") => {
                let url = msg.splitn(2, ' ').last().unwrap();
                match do_archive(&url, &user) {
                    Ok(reply) => client.send_privmsg(command_channel, format!("{}: {}", user, reply)).unwrap(),
                    Err(err)  => client.send_privmsg(command_channel, format!("{}: error: {}", user, err)).unwrap()
                }
            },
            msg if msg.starts_with("!abort ") => {
                let task = msg.splitn(2, ' ').last().unwrap();
                match do_abort(&task) {
                    Ok(reply) => client.send_privmsg(command_channel, format!("{}: {}", user, reply)).unwrap(),
                    Err(err)  => client.send_privmsg(command_channel, format!("{}: error: {}", user, err)).unwrap()
                }
            },
            _other => {},
        }
    }
}

pub fn get_folder(url: &str) -> Result<String, Box<error::Error>> {
    let canonical_url = get_canonical_url(url)?;
    let folder = match folder_for_url(&canonical_url) {
        Some(f) => f,
        None => return Err(MyError::new(format!("Could not get folder for URL {}", canonical_url)).into()),
    };
    Ok(folder)
}

fn do_archive(url: &str, user: &str) -> Result<String, Box<error::Error>> {
    if !url.starts_with("https://www.youtube.com/") {
        return Err(MyError::new(format!("URL must start with https://www.youtube.com/, was {}", url)).into());
    }
    if url.starts_with("https://www.youtube.com/watch?") {
        let channel_url = format!("https://www.youtube.com/channel/{}", get_youtube_channel(url)?);
        let folder = get_folder(&channel_url)?;
        let sessions = get_downloader_sessions()?;
        if let Some(_session) = sessions.iter().find(|session| session.identifier == folder) {
            return Ok(format!("Can't archive {} because another task is running in the same folder {}", &url, &folder));
        }
        let output = process::Command::new("grab-youtube-video")
            .arg(&folder).arg(url)
            .output()?;
        let _stdout_utf8 = str::from_utf8(&output.stdout)?;
        Ok(format!("Grabbing {} -> {}", &url, &folder))
    } else {
        let canonical_url = get_canonical_url(url)?;
        let folder = get_folder(&canonical_url)?;
        make_folder(&folder)?;
        let sessions = get_downloader_sessions()?;
        // This isn't a necessary safety check, just less confusing to the IRC user.
        if let Some(_session) = sessions.iter().find(|session| session.identifier == folder) {
            return Ok(format!("Already archiving {} now", &folder));
        }
        if sessions.len() >= limit_for_user(user) {
            return Ok(format!("Created folder {} but too many downloaders are running, try !a again later", &folder));
        }
        let limit = 999999;
        let output = process::Command::new("grab-youtube-channel")
            .arg(&folder).arg(limit.to_string())
            .output()?;
        let _stdout_utf8 = str::from_utf8(&output.stdout)?;
        Ok(format!("Grabbing {} -> {}", &url, &folder))
    }
}

fn do_abort(task: &str) -> Result<String, Box<error::Error>> {
    assert_valid_task_name(task)?;
    let session = format!("YouTube-{}", task);
    let _output = process::Command::new("tmux")
        .arg("send-keys").arg("-t").arg(&session).arg("C-c")
        .output()?;
    Ok(format!("Aborted {}", &task))
}

fn assert_valid_task_name(task: &str) -> Result<(), Box<error::Error>> {
    let re = regex::Regex::new(r"\A[-_A-Za-z0-9]+\z").unwrap();
    if re.is_match(task) {
        Ok(())
    } else {
        Err(MyError::new(format!("Invalid task name: {}", task)).into())
    }
}

fn limit_for_user(user: &str) -> usize {
    match user {
        "Flashfire" => MAX_DOWNLOADERS - 1,
        _           => MAX_DOWNLOADERS,
    }
}

fn make_folder(folder: &str) -> Result<(), Box<error::Error>> {
    let home    = dirs::home_dir().unwrap();
    let youtube = home.as_path().join("YouTube");
    let output  = process::Command::new("timeout")
        .current_dir(youtube)
        .arg("-k").arg("2m").arg("1m")
        .arg("ts").arg("mkdir").arg(folder)
        .output()?;
    let stdout_utf8 = str::from_utf8(&output.stdout)?;
    if stdout_utf8 != "" {
        Err(MyError::new(format!("Unexpected stdout from ts mkdir: {}", stdout_utf8)).into())
    } else {
        Ok(())
    }
}

fn do_stash_check(url: &str) -> Result<String, Box<error::Error>> {
    if !url.starts_with("https://www.youtube.com/") {
        return Err(MyError::new(format!("URL must start with https://www.youtube.com/, was {}", url)).into());
    }
    if url.starts_with("https://www.youtube.com/watch?") {
        return Err(MyError::new("!s on /watch? URL not yet implemented".to_owned()).into());
    }
    let canonical_url = get_canonical_url(url)?;
    let folder = match folder_for_url(&canonical_url) {
        Some(f) => f,
        None => return Err(MyError::new(format!("Could not get folder for URL {}", canonical_url)).into()),
    };
    let listing = match get_file_listing(&folder) {
        Err(e) => return Err(MyError::new(format!("Internal error listing files for {}: {}", canonical_url, e)).into()),
        Ok(files) => files,
    };
    let videos = listing
        .into_iter()
        .filter(|s: &String| {
            match s.rsplitn(2, '.').collect::<Vec<&str>>().first() {
                Some(&ext) => {
                    ext == "mp4" || ext == "webm" || ext == "flv" || ext == "mkv" || ext == "video"
                },
                None => false,
            }
        })
        .collect::<Vec<String>>();
    let latest_videos = videos.iter().take(4).collect::<Vec<_>>();
    Ok(format!("stash has {} videos for {}, latest {:?}", videos.len(), &folder, latest_videos))
}

/// Ensure that a YouTube URL actually exists and convert https://www.youtube.com/channel/*
/// to https://www.youtube.com/user/* when possible.
fn get_canonical_url(url: &str) -> Result<String, Box<error::Error>> {
    let parsed_url = url::Url::parse(url).unwrap();
    let canonical_url: String = match parsed_url.path() {
        "/playlist" => {
            // TODO: validate playlist URLs
            url.to_string()
        },
        p if p.starts_with("/user/") => {
            let user = get_youtube_user(url)?;
            match user {
                Some(user) => format!("https://www.youtube.com/user/{}/videos", user),
                _ => return Err(MyError::new(format!("Canonical URL for {} does not have a /user/", url)).into())
            }
        },
        p if p.starts_with("/channel/") => {
            let user = get_youtube_user(url)?;
            match user {
                Some(user) => format!("https://www.youtube.com/user/{}/videos", user),
                // TODO: validate channel URLs
                _ => url.to_string()
            }
        },
        other => other.to_string()
    };
    Ok(canonical_url)
}

fn folder_for_url(url: &str) -> Option<String> {
    let parsed_url = url::Url::parse(url).unwrap();
    let folder: Option<String> = match parsed_url.path() {
        "/playlist" => {
            let keys: HashMap<_, _> = parsed_url.query_pairs().into_owned().collect();
            if let Some(playlist) = keys.get("playlist") {
                Some(playlist.clone())
            } else {
                None
            }
        },
        p if p.starts_with("/user/") => {
            if let Some(user) = p.splitn(4, '/').map(String::from).collect::<Vec<String>>().get(2) {
                Some(user.clone())
            } else {
                None
            }
        },
        p if p.starts_with("/channel/") => {
            if let Some(channel) = p.splitn(4, '/').map(String::from).collect::<Vec<String>>().get(2) {
                Some(channel.clone())
            } else {
                None
            }
        },
        _p => None
    };
    let re = regex::Regex::new(r"\A[-_A-Za-z0-9]+\z").unwrap();
    match folder {
        Some(ref f) if re.is_match(f) => Some(f.clone()),
        _ => None
    }
}

fn get_file_listing(folder: &str) -> Result<Vec<String>, Box<error::Error>> {
    let output = process::Command::new("ts")
        .arg("ls").arg("-n").arg("YouTube").arg("-j").arg("-t").arg(folder)
        .output()?;
    let stdout_utf8 = str::from_utf8(&output.stdout)?;
    Ok(stdout_utf8.lines().map(String::from).collect())
}

fn get_help() -> String {
    "Usage: !help | !status | !a <user or channel or watch URL> | !s <user or channel URL> | !abort <task>".into()
}

#[derive(Debug)]
struct DownloaderSession {
    identifier: String,
    start_time: u64,
}

fn get_status() -> Vec<String> {
    match get_downloader_sessions() {
        Err(e) => vec![format!("{:?}", e)],
        Ok(mut sessions) => {
            sessions.sort_by_key(|session| session.start_time);
            sessions.reverse();
            let mut messages: Vec<String> = Vec::new();
            let mut last_message = format!("{}/{} downloaders: ", sessions.len(), MAX_DOWNLOADERS);
            for session in sessions {
                let part = format!("{} ({}), ", session.identifier, shorthand_duration(session.start_time));
                if last_message.len() + part.len() >= 430 {
                    messages.push(last_message);
                    last_message = String::new();
                }
                last_message.push_str(&part);
            }
            if last_message != "" {
               messages.push(last_message);
            }
            messages
        }
    }
}

fn shorthand_duration(start_time: u64) -> String {
    let start = SystemTime::now();
    let since_the_epoch = start.duration_since(UNIX_EPOCH)
        .expect("Time went backwards");
    let time_now = since_the_epoch.as_secs();
    let duration = time_now - start_time;
    duration_to_dhm(duration)
}

/// seconds -> e.g. 2d4h1m
fn duration_to_dhm(secs: u64) -> String {
    match secs {
        secs if secs < 3600  => {
            let minutes = secs / 60;
            format!("{}m", minutes)
        },
        secs if secs < 86400 => {
            let hours = secs / 3600;
            let minutes = (secs - (hours * 3600)) / 60;
            format!("{}h{}m", hours, minutes)
        },
        secs => {
            let days  = secs / 86400;
            let hours = (secs - (days * 86400)) / 3600;
            let minutes = (secs - (days * 86400) - (hours * 3600)) / 60;
            format!("{}d{}h{}m", days, hours, minutes)
        },
    }
}

fn get_downloader_sessions() -> Result<Vec<DownloaderSession>, Box<error::Error>> {
    let output = process::Command::new("tmux")
        .arg("list-sessions")
        .arg("-F")
        .arg("#{session_created} #S")
        .output()?;
    let stdout_utf8 = str::from_utf8(&output.stdout)?;
    let sessions =
        stdout_utf8.lines()
            .filter_map(|line| {
                let parts = line.splitn(2, ' ').collect::<Vec<&str>>();
                let start_time   = parts.get(0).unwrap().parse::<u64>().unwrap();
                let session_name = parts.get(1).unwrap();
                if session_name.starts_with("YouTube-") {
                    let identifier = session_name.replacen("YouTube-", "", 1);
                    Some(DownloaderSession { identifier, start_time })
                } else {
                    None
                }
            }).collect();
    Ok(sessions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_folder_for_url() {
        assert_eq!(folder_for_url("https://www.youtube.com/channel/UChBBWt5H8uZW1LSOh_aPt2Q/videos"), "UChBBWt5H8uZW1LSOh_aPt2Q");
        assert_eq!(folder_for_url("https://www.youtube.com/user/jblow888/videos"), "jblow888");
        assert_eq!(folder_for_url("https://www.youtube.com/playlist?list=PL5AC656794EE191C1"), "PL5AC656794EE191C1");
        assert_eq!(folder_for_url("https://www.youtube.com/playlist?list=PL78L-9twndz8fMRU3NpiWSmB5IucqWuTF"), "PL78L-9twndz8fMRU3NpiWSmB5IucqWuTF");
    }

    #[test]
    fn test_get_canonical_url() {
        // Playlist is playlist if it exists
        assert_eq!(
            get_canonical_url("https://www.youtube.com/playlist?list=PL5AC656794EE191C1").unwrap(),
            Some("https://www.youtube.com/playlist?list=PL5AC656794EE191C1")
        );
        assert_eq!(
            get_canonical_url("https://www.youtube.com/playlist?list=PL78L-9twndz8fMRU3NpiWSmB5IucqWuTF").unwrap(),
            Some("https://www.youtube.com/playlist?list=PL78L-9twndz8fMRU3NpiWSmB5IucqWuTF")
        );
        // Playlist is None if it doesn't exist
        assert_eq!(
            get_canonical_url("https://www.youtube.com/playlist?list=PL78L-9twndz8fMRU3NpiWSmB5IucqWuTa").unwrap(),
            None
        );
        // User is user if it exists
        assert_eq!(
            get_canonical_url("https://www.youtube.com/user/jblow888/videos").unwrap(),
            Some("https://www.youtube.com/user/jblow888/videos")
        );
        // User is always converted to canonical case
        assert_eq!(
            get_canonical_url("https://www.youtube.com/user/JBlow888/videos").unwrap(),
            Some("https://www.youtube.com/user/jblow888/videos")
        );
        assert_eq!(
            get_canonical_url("https://www.youtube.com/user/cnn/videos").unwrap(),
            Some("https://www.youtube.com/user/CNN/videos")
        );
        // User is None if it doesn't exist
        assert_eq!(
            get_canonical_url("https://www.youtube.com/user/jblow8888/videos").unwrap(),
            None
        );
        // Channel is channel if it exists and has no username
        assert_eq!(
            get_canonical_url("https://www.youtube.com/channel/UChBBWt5H8uZW1LSOh_aPt2Q/videos").unwrap(),
            Some("https://www.youtube.com/channel/UChBBWt5H8uZW1LSOh_aPt2Q/videos")
        );
        // Channel is user if it exists and has a username
        assert_eq!(
            get_canonical_url("https://www.youtube.com/channel/UCupvZG-5ko_eiXAupbDfxWw").unwrap(),
            Some("https://www.youtube.com/user/CNN/videos")
        );
        // Channel is None if it doesn't exist
        assert_eq!(
            get_canonical_url("https://www.youtube.com/channel/UChBBWt5H8uZW1LSOh_aPt2a/videos").unwrap(),
            None
        );
    }
}
