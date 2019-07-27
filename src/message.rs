use irc::client::prelude::*;
use std::collections::HashMap;
use std::str;
use std::error;
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};
use regex::Regex;

use super::http::{get_youtube_user, get_youtube_channel};
use super::config::Rtd;
use super::error::MyError;

enum VideoSize {
    Normal,
    Big
}

pub fn handle_message(
    client: &IrcClient, message: &Message, rtd: &Rtd
) -> Result<(), Box<dyn std::error::Error>> {
    // print the message if debug flag is set
    if rtd.args.flag_debug {
        eprintln!("{:?}", message.command)
    }

    // match on message type
    let (_target, msg) = match message.command {
        Command::PRIVMSG(ref target, ref msg) => (target, msg),
        _ => return Ok(()),
    };

    let user = message.source_nickname().unwrap();
    let channel = &rtd.conf.params.command_channel;

    lazy_static! {
        static ref WEBCHAT_RE: Regex = Regex::new(r"\A.+!webchat@.+\z").unwrap();
    }
    let check_authorization = || {
        if let Some(prefix) = &message.prefix {
            if WEBCHAT_RE.is_match(prefix) {
                send_reply(client, channel, user, Ok("webchat users are not authorized; use any other IRC client, or ask someone else to do it".into()));
                return Err(MyError::new("webchat users are not authorized".into()));
            }
        }
        return Ok(());
    };

    if message.response_target() == Some(channel) {
        match msg.as_ref() {
            "!help" => {
                client.send_privmsg(channel, get_help()).unwrap()
            },
            "!status" => {
                send_reply(client, channel, user, get_status(rtd));
            },
            "clear screen" => {
                check_authorization()?;
                for message in get_full_status(rtd) {
                    client.send_privmsg(channel, message).unwrap()
                }
            },
            "!stopscripts" => {
                check_authorization()?;
                send_reply(client, channel, user, stop_scripts());
            },
            "!contscripts" => {
                check_authorization()?;
                send_reply(client, channel, user, cont_scripts());
            },
            msg if msg.starts_with("!s ") => {
                let url = msg.split(' ').take(2).last().unwrap();
                send_reply(client, channel, user, do_stash_check(&url));
            },
            msg if msg.starts_with("!a ") => {
                check_authorization()?;
                let url = msg.split(' ').take(2).last().unwrap();
                send_reply(client, channel, user, do_archive(&url, VideoSize::Normal, &user, &rtd));
            },
            msg if msg.starts_with("!abig ") => {
                check_authorization()?;
                let url = msg.split(' ').take(2).last().unwrap();
                send_reply(client, channel, user, do_archive(&url, VideoSize::Big, &user, &rtd));
            },
            msg if msg.starts_with("!abort ") => {
                check_authorization()?;
                let task = msg.split(' ').take(2).last().unwrap();
                send_reply(client, channel, user, do_abort(&task));
            },
            _other => {},
        }
    }
    Ok(())
}

fn send_reply(client: &IrcClient, channel: &str, user: &str, result: Result<String, Box<dyn error::Error>>) {
    match result {
        Ok(reply) => client.send_privmsg(channel, format!("{}: {}", user, reply)).unwrap(),
        Err(err)  => client.send_privmsg(channel, format!("{}: error: {}", user, err)).unwrap()
    }
}

pub fn get_folder(url: &str) -> Result<String, Box<dyn error::Error>> {
    let canonical_url = get_canonical_url(url)?;
    let folder = match folder_for_url(&canonical_url) {
        Some(f) => f,
        None => return Err(MyError::new(format!("Could not get folder for URL {}", canonical_url)).into()),
    };
    Ok(folder)
}

fn fix_youtube_url(url: &str) -> Result<String, Box<dyn error::Error>> {
    let url = url.replace("http://", "https://");
    let url = url.replace("https://m.youtube.com/", "https://www.youtube.com/");
    let url = url.replace("https://youtube.com/", "https://www.youtube.com/");
    let url = url.replace("https://youtu.be/", "https://www.youtube.com/watch?v=");
    // Fix annoying links that fail to load on mobile
    let url = url.replace("?disable_polymer=1", "");
    let url = url.replace("&disable_polymer=1", "");
    if !url.starts_with("https://www.youtube.com/") {
        return Err(MyError::new(format!("Unsupported URL {}", url)).into());
    }
    Ok(url)
}

fn do_archive(url: &str, video_size: VideoSize, user: &str, rtd: &Rtd) -> Result<String, Box<dyn error::Error>> {
    let url = fix_youtube_url(url)?;
    if url.starts_with("https://www.youtube.com/watch?") {
        let channel_url = format!("https://www.youtube.com/channel/{}", get_youtube_channel(&url)?);
        let folder = get_folder(&channel_url)?;
        let sessions = get_downloader_sessions()?;
        if let Some(_session) = sessions.iter().find(|session| session.identifier == folder) {
            return Ok(format!("Can't archive {} because another task is running in the same folder {}", &url, &folder));
        }
        let command = match video_size {
            VideoSize::Normal => "grab-youtube-video",
            VideoSize::Big    => "grab-youtube-video-big-video"
        };
        let output = process::Command::new(command)
            .arg(&folder).arg(&url)
            .output()?;
        let _stdout_utf8 = str::from_utf8(&output.stdout)?;
        Ok(format!("Grabbing {} -> {}; check https://ya.borg.xyz/logs/dl/{}/ later", &url, &folder, &folder))
    } else {
        let canonical_url = get_canonical_url(&url)?;
        let folder = get_folder(&canonical_url)?;
        make_folder(&folder)?;
        let sessions = get_downloader_sessions()?;
        if let Some(_session) = sessions.iter().find(|session| session.identifier == folder) {
            return Ok(format!("Can't archive {} because another task is running in the same folder {}", &url, &folder));
        }
        let limit = limit_for_user(user, rtd);
        if sessions.len() >= limit {
            return Ok(format!("Can't archive {} because too many downloaders are running (your limit = {}), try again later", &url, limit));
        }
        let limit = 999999;
        let command = match video_size {
            VideoSize::Normal => "grab-youtube-channel",
            VideoSize::Big    => "grab-youtube-channel-big-videos"
        };
        let output = process::Command::new(command)
            .arg(&folder).arg(limit.to_string())
            .output()?;
        let _stdout_utf8 = str::from_utf8(&output.stdout)?;
        Ok(format!("Grabbing {} -> {}; check https://ya.borg.xyz/logs/dl/{}/ later", &url, &folder, &folder))
    }
}

fn do_abort(task: &str) -> Result<String, Box<dyn error::Error>> {
    assert_valid_task_name(task)?;
    let session = format!("YouTube-{}", task);
    let _output = process::Command::new("tmux")
        .arg("send-keys").arg("-t").arg(&session).arg("C-c")
        .output()?;
    Ok(format!("Aborted {}", &task))
}

fn assert_valid_task_name(task: &str) -> Result<(), Box<dyn error::Error>> {
    lazy_static! {
        static ref TASK_NAME_RE: Regex = Regex::new(r"\A[-_A-Za-z0-9]+\z").unwrap();
    }
    if TASK_NAME_RE.is_match(task) {
        Ok(())
    } else {
        Err(MyError::new(format!("Invalid task name: {}", task)).into())
    }
}

fn limit_for_user(user: &str, rtd: &Rtd) -> usize {
    let user_limits = &rtd.conf.user_limits;
    match user_limits.get(user) {
        None         => rtd.conf.params.task_limit,
        Some(&limit) => limit
    }
}

fn make_folder(folder: &str) -> Result<(), Box<dyn error::Error>> {
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

fn do_stash_check(url: &str) -> Result<String, Box<dyn error::Error>> {
    let url = fix_youtube_url(url)?;
    if url.starts_with("https://www.youtube.com/watch?") {
        return Err(MyError::new("!s on /watch? URL not yet implemented".to_owned()).into());
    }
    let canonical_url = get_canonical_url(&url)?;
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
fn get_canonical_url(url: &str) -> Result<String, Box<dyn error::Error>> {
    let parsed_url = url::Url::parse(url).unwrap();
    let canonical_url: String = match parsed_url.path() {
        "/playlist" => {
            // TODO: validate playlist URLs
            url.to_string()
        },
        p if p.starts_with("/user/") => {
            let user = get_youtube_user(url)?;
            match user {
                Some(user) => {
                    format!("https://www.youtube.com/user/{}/videos", user)
                },
                _ => {
                    return Err(MyError::new(format!("Canonical URL for {} does not have a /user/", url)).into())
                }
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
                if user == "TEDxTalks" {
                    Some("UCsT0YIqwnpJCM-mx7-gSA4Q".into())
                } else {
                    Some(user.clone())
                }
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
    lazy_static! {
        static ref FOLDER_RE: Regex = Regex::new(r"\A[-_A-Za-z0-9]+\z").unwrap();
    }
    match folder {
        Some(ref f) if FOLDER_RE.is_match(f) => Some(f.clone()),
        _ => None
    }
}

fn get_file_listing(folder: &str) -> Result<Vec<String>, Box<dyn error::Error>> {
    let output = process::Command::new("ts")
        .arg("ls").arg("-n").arg("YouTube").arg("-j").arg("-t").arg(folder)
        .output()?;
    let stdout_utf8 = str::from_utf8(&output.stdout)?;
    Ok(stdout_utf8.lines().map(String::from).collect())
}

fn get_help() -> String {
    "Usage: !help | !status | !a <user or channel or watch URL> | !abig <user or channel or watch URL with large videos> | !s <user or channel URL> | !abort <task> | !stopscripts | !contscripts".into()
}

#[derive(Debug)]
struct DownloaderSession {
    identifier: String,
    start_time: u64,
}

fn stop_scripts() -> Result<String, Box<dyn error::Error>> {
    let _ = process::Command::new("stop-all-youtube-scripts").output()?;
    Ok("Stopped all scripts".to_owned())
}

fn cont_scripts() -> Result<String, Box<dyn error::Error>> {
    let _ = process::Command::new("cont-all-youtube-scripts").output()?;
    Ok("Continued all scripts".to_owned())
}

fn get_status(rtd: &Rtd) -> Result<String, Box<dyn error::Error>> {
    let sessions    = get_downloader_sessions()?;
    let scripts     = process::Command::new("get-running-youtube-scripts").output()?.stdout;
    let num_scripts = scripts.iter().filter(|&&c| c == b'\n').count();
    Ok(format!("{}/{} downloaders, {} scripts running", sessions.len(), rtd.conf.params.task_limit, num_scripts))
}

fn get_full_status(rtd: &Rtd) -> Vec<String> {
    match get_downloader_sessions() {
        Err(e) => vec![format!("{:?}", e)],
        Ok(mut sessions) => {
            sessions.sort_by_key(|session| session.start_time);
            sessions.reverse();
            let mut messages: Vec<String> = Vec::new();
            let mut last_message = format!("{}/{} downloaders: ", sessions.len(), rtd.conf.params.task_limit);
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

fn get_downloader_sessions() -> Result<Vec<DownloaderSession>, Box<dyn error::Error>> {
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
