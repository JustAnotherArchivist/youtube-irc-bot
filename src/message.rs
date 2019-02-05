use irc::client::prelude::*;
use std::collections::HashMap;
use std::iter;
use std::str;
use std::error;
use std::process;
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};
use unicode_segmentation::UnicodeSegmentation;
use regex::Regex;
use itertools::Itertools;

use super::http::get_youtube_user;
use super::sqlite::Database;
use super::config::Rtd;

pub fn handle_message(
    client: &IrcClient, message: &Message, rtd: &Rtd, _db: &Database
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
                client.send_privmsg(command_channel, get_status()).unwrap()
            },
            msg if msg.starts_with("!q ") => {
                let url = msg.splitn(2, ' ').last().unwrap();
                match get_query(&url) {
                    Ok(reply) => client.send_privmsg(command_channel, format!("{}: {}", user, reply)).unwrap(),
                    Err(err)  => client.send_privmsg(command_channel, format!("{}: error: {}", user, err)).unwrap()
                }
            },
            _other => {},
        }
    }
}

#[derive(Debug, Clone)]
struct MyError {
    message: String
}

impl MyError {
    pub fn new(message: String) -> MyError {
        MyError { message }
    }
}

impl fmt::Display for MyError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl error::Error for MyError {
    fn description(&self) -> &str {
        &self.message
    }

    fn cause(&self) -> Option<&error::Error> {
        // Generic error, underlying cause isn't tracked.
        None
    }
}

fn get_query(url: &str) -> Result<String, Box<error::Error>> {
    if !url.starts_with("https://www.youtube.com/") {
        return Err(MyError::new(format!("URL must start with https://www.youtube.com/, was {}", url)).into());
    }
    if url.starts_with("https://www.youtube.com/watch?") {
        return Err(MyError::new("!q on /watch? URL not yet implemented".to_owned()).into());
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
            match s.rsplitn(1, '.').collect::<Vec<&str>>().last() {
                Some(&ext) => ext == "mp4" || ext == "webm" || ext == "flv" || ext == "mkv" || ext == "video",
                None       => false,
            }
        })
        .collect::<Vec<String>>();
    let latest_string = match videos.last() {
        Some(video) => format!(", latest {}", video),
        None        => format!(""),
    };
    Ok(format!("{} has {} videos{}", folder, videos.len(), latest_string))
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
        .arg("ls")
        .arg("-n")
        .arg("YouTube")
        .arg("-j")
        .arg("-rt")
        .arg(folder)
        .output()?;
    let stdout_utf8 = str::from_utf8(&output.stdout)?;
    Ok(stdout_utf8.lines().map(String::from).collect())
}

fn get_help() -> String {
    "Usage: !help | !status | !a <user or channel or playlist or /watch URL> | !q <user or channel or playlist or /watch URL>".into()
}

#[derive(Debug)]
struct DownloaderSession {
    identifier: String,
    start_time: u64,
}

fn get_status() -> String {
    match get_downloader_sessions() {
        Err(e) => format!("{:?}", e),
        Ok(mut sessions) => {
            sessions.sort_by_key(|session| session.start_time);
            sessions.reverse();
            sessions.iter().map(|session| {
                format!("{} ({})", session.identifier, shorthand_duration(session.start_time))
            }).join(", ")
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
                    let identifier   = session_name.replacen("YouTube-", "", 1);
                    Some(DownloaderSession { identifier, start_time })
                } else {
                    None
                }
            }).collect();
    Ok(sessions)
}

// regex for unsafe characters, as defined in RFC 1738
const RE_UNSAFE_CHARS: &str = r"[{}|\\^~\[\]`]";

fn contains_unsafe_chars(token: &str) -> bool {
    lazy_static! {
        static ref UNSAFE: Regex = Regex::new(RE_UNSAFE_CHARS).unwrap();
    }
    UNSAFE.is_match(token)
}

fn create_non_highlighting_name(name: &str) -> String {
    let mut graphemes = name.graphemes(true);
    let first = graphemes.next();

    first
        .into_iter()
        .chain(iter::once("\u{200C}"))
        .chain(graphemes)
        .collect()
}

// truncate to a maximum number of bytes, taking UTF-8 into account
fn utf8_truncate(s: &str, n: usize) -> String {
    s.char_indices()
        .take_while(|(len, c)| len + c.len_utf8() <= n)
        .map(|(_, c)| c)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_utf8_truncate() {
        assert_eq!("",                 utf8_truncate("", 10));
        assert_eq!("",                 utf8_truncate("", 1));
        assert_eq!(" ",                utf8_truncate("  ", 1));
        assert_eq!("\u{2665}",         utf8_truncate("\u{2665}", 4));
        assert_eq!("\u{2665}",         utf8_truncate("\u{2665}", 3));
        assert_eq!("",                 utf8_truncate("\u{2665}", 2));
        assert_eq!("\u{0306}\u{0306}", utf8_truncate("\u{0306}\u{0306}", 4));
        assert_eq!("\u{0306}",         utf8_truncate("\u{0306}\u{0306}", 2));
        assert_eq!("\u{0306}",         utf8_truncate("\u{0306}", 2));
        assert_eq!("",                 utf8_truncate("\u{0306}", 1));
        assert_eq!("hello ",           utf8_truncate("hello \u{1F603} world!", 9));
    }

    #[test]
    fn test_create_non_highlighting_name() {
        assert_eq!("\u{200C}",    create_non_highlighting_name(""));
        assert_eq!("f\u{200C}oo", create_non_highlighting_name("foo"));
        assert_eq!("b\u{200C}ar", create_non_highlighting_name("bar"));
        assert_eq!("b\u{200C}az", create_non_highlighting_name("baz"));
    }

    #[test]
    fn test_contains_unsafe_chars() {
        for c in &['{', '}', '|', '\\', '^', '~', '[', ']', '`'] {
            assert_eq!(contains_unsafe_chars(&format!("http://z/{}", c)), true);
        }
        assert_eq!(contains_unsafe_chars("http://z.zzz/"), false);
    }

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
