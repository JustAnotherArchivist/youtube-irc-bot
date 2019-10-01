use irc::client::prelude::*;
use std::str;
use std::process;
use regex::Regex;
use snafu::{ensure, ResultExt, Snafu, Backtrace};

use super::config::Rtd;

enum VideoSize {
    Normal,
    VeryBig
}

#[derive(Debug, Snafu)]
pub enum Error {
    Io { source: std::io::Error, backtrace: Backtrace },
    Utf8 { source: std::str::Utf8Error, backtrace: Backtrace },
    #[snafu(display("Unsupported URL: {}", url))]
    UnsupportedUrl { url: String },
    #[snafu(display("Not authorized"))]
    NotAuthorized,
    #[snafu(display("Could not get folder for URL {}", url))]
    CouldNotGetFolder { url: String },
    #[snafu(display("Could not get channel identifier"))]
    CouldNotGetChannelIdentifier,
    #[snafu(display("Could not make a YouTube descriptor from URL {}", url))]
    CouldNotMakeYoutubeDescriptor { url: String },
    #[snafu(display("Invalid task name: {}", task))]
    InvalidTaskName { task: String },
    #[snafu(display("Not implemented: {}", what))]
    NotImplemented { what: String },
    #[snafu(display("Internal error listing files for {}", folder))]
    ErrorListingFiles { folder: String },
    #[snafu(display("Internal error creating folder {}", folder))]
    ErrorCreatingFolder { folder: String },
    #[snafu(display("Channel ID in page {:?} does not match provided channel ID {:?}", page_channel_id, provided_channel_id))]
    PageChannelIdDoesNotMatch { page_channel_id: String, provided_channel_id: String },
    #[snafu(display("Username in page {:?} does not match provided username {:?}", page_username, provided_username))]
    PageUsernameDoesNotMatch { page_username: Option<String>, provided_username: Option<String> },
}

type Result<T, E = Error> = std::result::Result<T, E>;

fn fix_youtube_url(url: &str) -> String {
    let url = url.replace("http://", "https://");
    let url = url.replace("https://m.youtube.com/", "https://www.youtube.com/");
    let url = url.replace("https://youtube.com/", "https://www.youtube.com/");
    let url = url.replace("https://youtu.be/", "https://www.youtube.com/watch?v=");
    // Fix annoying links that fail to load on mobile
    let url = url.replace("?disable_polymer=1", "");
    let url = url.replace("&disable_polymer=1", "");
    url
}

fn contents_for_url(url: &str) -> Result<String> {
    let output = process::Command::new("get-youtube-page").arg(&url).output().context(Io)?;
    let body = str::from_utf8(&output.stdout).context(Utf8)?;
    Ok(body.into())
}

fn extract_username(page_contents: &str) -> Option<String> {
    lazy_static! {
        static ref RE: Regex = Regex::new(r#"<link itemprop="url" href="http://www.youtube.com/user/([^"]+)">"#).unwrap();
    }
    let user = RE.captures(page_contents)?.get(1)?.as_str();
    Some(user.to_string())
}

fn extract_channel_id(page_contents: &str) -> Result<String> {
    lazy_static! {
        static ref CHANNEL_META_RE:          Regex = Regex::new(r#"<meta itemprop="channelId" content="([^"]+)">"#).unwrap();
        static ref CHANNEL_VIDEO_DETAILS_RE: Regex = Regex::new(r#" ytplayer = .+?\\"channelId\\":\\"([^"]+)\\""#).unwrap();
    }
    match &CHANNEL_META_RE.captures(page_contents) {
        Some(captures) if captures.len() >= 1 => {
            Ok(captures.get(1).unwrap().as_str().to_owned())
        },
        _ => match &CHANNEL_VIDEO_DETAILS_RE.captures(page_contents) {
            Some(captures) if captures.len() >= 1 => {
                Ok(captures.get(1).unwrap().as_str().to_owned())
            },
            _ => Err(Error::CouldNotGetChannelIdentifier)
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum FetchType {
    User,
    Channel,
    Playlist,
    Video,
}

#[derive(Debug, PartialEq, Eq)]
pub struct CanonicalizedYoutubeDescriptor {
    id: String,
    folder: String,
    kind: FetchType,
}

impl CanonicalizedYoutubeDescriptor {
    pub fn to_url(&self) -> String {
        match self.kind {
            FetchType::User     => format!("https://www.youtube.com/user/{}/videos", self.id),
            FetchType::Channel  => format!("https://www.youtube.com/channel/{}/videos", self.id),
            FetchType::Playlist => format!("https://www.youtube.com/playlist?list={}", self.id),
            FetchType::Video    => format!("https://www.youtube.com/watch?v={}", self.id),
        }
    }

    pub fn folder(&self) -> String {
        self.folder.clone()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum YoutubeDescriptor {
    User(String),
    Channel(String),
    Playlist(String),
    Video(String),
}

impl YoutubeDescriptor {
    pub fn from_url(url: &str) -> Result<YoutubeDescriptor> {
        lazy_static! {
            static ref OLD_PLAYLIST_RE: Regex = Regex::new("https://www.youtube.com/playlist?list=(PL[0-9A-F]{16})").unwrap();
            static ref NEW_PLAYLIST_RE: Regex = Regex::new("https://www.youtube.com/playlist?list=(PL[-_A-Za-z0-9]{32})").unwrap();
            static ref WATCH_RE:        Regex = Regex::new("https://www.youtube.com/watch?v=([-_A-Za-z0-9]{11})").unwrap();
            static ref CHANNEL_RE:      Regex = Regex::new("https://www.youtube.com/channel/(UC[-_A-Za-z0-9]{22})").unwrap();
            static ref USER_RE:         Regex = Regex::new("https://www.youtube.com/user/([a-z][-_a-z0-9]{1,31})").unwrap();
        }

        let url = fix_youtube_url(url);
        ensure!(url.starts_with("https://www.youtube.com/"), UnsupportedUrl { url });
        if let Some(matches) = OLD_PLAYLIST_RE.captures(&url) {
            return Ok(YoutubeDescriptor::Playlist(matches.get(1).unwrap().as_str().to_string()));
        }
        if let Some(matches) = NEW_PLAYLIST_RE.captures(&url) {
            return Ok(YoutubeDescriptor::Playlist(matches.get(1).unwrap().as_str().to_string()));
        }
        if let Some(matches) = WATCH_RE.captures(&url) {
            return Ok(YoutubeDescriptor::Video(matches.get(1).unwrap().as_str().to_string()));
        }
        if let Some(matches) = CHANNEL_RE.captures(&url) {
            return Ok(YoutubeDescriptor::Channel(matches.get(1).unwrap().as_str().to_string()));
        }
        if let Some(matches) = USER_RE.captures(&url) {
            return Ok(YoutubeDescriptor::User(matches.get(1).unwrap().as_str().to_string()));
        }
        Err(Error::UnsupportedUrl { url })
    }

    pub fn to_url(&self) -> String {
        match self {
            YoutubeDescriptor::User(id)     => format!("https://www.youtube.com/user/{}/videos", id),
            YoutubeDescriptor::Channel(id)  => format!("https://www.youtube.com/channel/{}/videos", id),
            YoutubeDescriptor::Playlist(id) => format!("https://www.youtube.com/playlist?list={}", id),
            YoutubeDescriptor::Video(id)    => format!("https://www.youtube.com/watch?v={}", id),
        }
    }

    // Turn Channel into User if possible, because that's how our data storage works.
    // Turn User into properly-cased User.
    pub fn canonicalize(&self) -> Result<CanonicalizedYoutubeDescriptor> {
        Ok(match self {
            YoutubeDescriptor::Video(id) => {
                let contents = contents_for_url(&self.to_url())?;
                let channel_id = extract_channel_id(&contents)?;
                let folder = YoutubeDescriptor::Channel(channel_id).canonicalize()?.folder();
                CanonicalizedYoutubeDescriptor { kind: FetchType::Video, id: id.clone(), folder: folder }
            },
            YoutubeDescriptor::Playlist(id) => {
                CanonicalizedYoutubeDescriptor { kind: FetchType::Playlist, id: id.clone(), folder: id.clone() }
            },
            YoutubeDescriptor::User(id) => {
                let contents = contents_for_url(&self.to_url())?;
                let username = extract_username(&contents);
                ensure!(username.as_ref() == Some(id), PageUsernameDoesNotMatch { page_username: username, provided_username: Some(id.clone()) });
                CanonicalizedYoutubeDescriptor { kind: FetchType::User, id: username.clone().unwrap(), folder: username.clone().unwrap() }
            },
            YoutubeDescriptor::Channel(id) => {
                let contents = contents_for_url(&self.to_url())?;
                let username = extract_username(&contents);
                match username {
                    None => {
                        let channel_id = extract_channel_id(&contents)?;
                        ensure!(&channel_id == id, PageChannelIdDoesNotMatch { page_channel_id: channel_id, provided_channel_id: id });
                        CanonicalizedYoutubeDescriptor { kind: FetchType::Channel, id: channel_id.clone(), folder: channel_id.clone() }
                    }
                    Some(username) => {
                        CanonicalizedYoutubeDescriptor { kind: FetchType::User, id: username.clone(), folder: username.clone() }
                    }
                }
            },
        })
    }
}

pub fn handle_message(client: &IrcClient, message: &Message, rtd: &Rtd) -> Result<()> {
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
                return Err(Error::NotAuthorized);
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
            msg if msg.starts_with("!sa ") => {
                check_authorization()?;
                let url = msg.split(' ').take(2).last().unwrap();
                send_reply(client, channel, user, do_stash_check(&url));
                send_reply(client, channel, user, do_archive(&url, VideoSize::Normal, &user, &rtd));
            },
            msg if msg.starts_with("!averybig ") => {
                check_authorization()?;
                let url = msg.split(' ').take(2).last().unwrap();
                send_reply(client, channel, user, do_archive(&url, VideoSize::VeryBig, &user, &rtd));
            },
            msg if msg.starts_with("!saverybig ") => {
                check_authorization()?;
                let url = msg.split(' ').take(2).last().unwrap();
                send_reply(client, channel, user, do_stash_check(&url));
                send_reply(client, channel, user, do_archive(&url, VideoSize::VeryBig, &user, &rtd));
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

fn send_reply(client: &IrcClient, channel: &str, user: &str, result: Result<String>) {
    match result {
        Ok(reply) => client.send_privmsg(channel, format!("{}: {}", user, reply)).unwrap(),
        Err(err)  => client.send_privmsg(channel, format!("{}: error: {}", user, err)).unwrap()
    }
}

fn do_archive(url: &str, video_size: VideoSize, user: &str, rtd: &Rtd) -> Result<String> {
    let descriptor = YoutubeDescriptor::from_url(url)?.canonicalize()?;
    match descriptor.kind {
        FetchType::Video => {
            let folder = descriptor.folder();
            let url = descriptor.to_url();
            let sessions = get_downloader_sessions()?;
            if let Some(_session) = sessions.iter().find(|session| session.identifier == folder) {
                return Ok(format!("Can't archive {} because another task is running in the same folder {}", &url, &folder));
            }
            let command = match video_size {
                VideoSize::Normal  => "grab-youtube-video",
                VideoSize::VeryBig => "grab-youtube-video-big-video"
            };
            let output = process::Command::new(command)
                .arg(&folder).arg(&url)
                .output()
                .context(Io)?;
            let _stdout_utf8 = str::from_utf8(&output.stdout).context(Utf8)?;
            Ok(format!("Grabbing {} -> {}; check https://ya.borg.xyz/logs/dl/{}/ later", &url, &folder, &folder))
        },
        FetchType::Channel | FetchType::User | FetchType::Playlist => {
            let folder = descriptor.folder();
            make_folder(&folder)?;
            let url = descriptor.to_url();
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
                VideoSize::Normal  => "grab-youtube-channel",
                VideoSize::VeryBig => "grab-youtube-channel-big-videos"
            };
            let output = process::Command::new(command)
                .arg(&folder).arg(limit.to_string())
                .output()
                .context(Io)?;
            let _stdout_utf8 = str::from_utf8(&output.stdout).context(Utf8)?;
            Ok(format!("Grabbing {} -> {}; check https://ya.borg.xyz/logs/dl/{}/ later", &url, &folder, &folder))

        }
    }
}

fn do_abort(task: &str) -> Result<String> {
    assert_valid_task_name(task)?;
    let session = format!("YouTube-{}", task);
    let _output = process::Command::new("tmux")
        .arg("send-keys").arg("-t").arg(&session).arg("C-c")
        .output()
        .context(Io)?;
    Ok(format!("Aborted {}", &task))
}

fn assert_valid_task_name(task: &str) -> Result<()> {
    lazy_static! {
        static ref TASK_NAME_RE: Regex = Regex::new(r"\A[-_A-Za-z0-9]+\z").unwrap();
    }
    ensure!(TASK_NAME_RE.is_match(task), InvalidTaskName { task });
    Ok(())
}

fn limit_for_user(user: &str, rtd: &Rtd) -> usize {
    let user_limits = &rtd.conf.user_limits;
    match user_limits.get(user) {
        None         => rtd.conf.params.task_limit,
        Some(&limit) => limit
    }
}

fn make_folder(folder: &str) -> Result<()> {
    let home    = dirs::home_dir().unwrap();
    let youtube = home.as_path().join("YouTube");
    let output  = process::Command::new("timeout")
        .current_dir(youtube)
        .arg("-k").arg("2m").arg("1m")
        .arg("ts").arg("mkdir").arg(folder)
        .output()
        .context(Io)?;
    let stdout_utf8 = str::from_utf8(&output.stdout).context(Utf8)?;
    ensure!(stdout_utf8 == "", ErrorCreatingFolder { folder });
    Ok(())
}

fn do_stash_check(url: &str) -> Result<String> {
    let descriptor = YoutubeDescriptor::from_url(url)?;
    if let YoutubeDescriptor::Video(_) = descriptor {
        return Err(Error::NotImplemented { what: "/s on /watch? URL".into() });
    }
    let folder = descriptor.canonicalize()?.folder();
    let listing = match get_file_listing(&folder) {
        Err(_) => return Err(Error::ErrorListingFiles { folder }),
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

fn get_file_listing(folder: &str) -> Result<Vec<String>> {
    let output = process::Command::new("ts")
        .arg("ls").arg("-n").arg("YouTube").arg("-j").arg("-t").arg(folder)
        .output()
        .context(Io)?;
    let stdout_utf8 = str::from_utf8(&output.stdout).context(Utf8)?;
    Ok(stdout_utf8.lines().map(String::from).collect())
}

fn get_help() -> String {
    "Usage: !help | !status | !a <user or channel or watch URL> | !averybig <user or channel or watch URL with very large videos> | !s <user or channel URL> | !abort <task> | !stopscripts | !contscripts".into()
}

#[derive(Debug)]
struct DownloaderSession {
    identifier: String,
    start_time: u64,
}

fn stop_scripts() -> Result<String> {
    let _ = process::Command::new("stop-all-youtube-scripts").output().context(Io)?;
    Ok("Stopped all scripts".to_string())
}

fn cont_scripts() -> Result<String> {
    let _ = process::Command::new("cont-all-youtube-scripts").output().context(Io)?;
    Ok("Continued all scripts".to_string())
}

fn get_status(rtd: &Rtd) -> Result<String> {
    let sessions    = get_downloader_sessions()?;
    let scripts     = process::Command::new("get-running-youtube-scripts").output().context(Io)?.stdout;
    let num_scripts = scripts.iter().filter(|&&c| c == b'\n').count();
    Ok(format!("{}/{} downloaders, {} scripts running", sessions.len(), rtd.conf.params.task_limit, num_scripts))
}

fn get_downloader_sessions() -> Result<Vec<DownloaderSession>> {
    let output = process::Command::new("tmux")
        .arg("list-sessions")
        .arg("-F")
        .arg("#{session_created} #S")
        .output()
        .context(Io)?;
    let stdout_utf8 = str::from_utf8(&output.stdout).context(Utf8)?;
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
    fn test_descriptor() {
        assert_eq!(YoutubeDescriptor::from_url("https://www.youtube.com/channel/UChBBWt5H8uZW1LSOh_aPt2Q/videos").unwrap(), YoutubeDescriptor::Channel("UChBBWt5H8uZW1LSOh_aPt2Q".to_string()));
        assert_eq!(YoutubeDescriptor::from_url("https://www.youtube.com/user/jblow888/videos").unwrap(), YoutubeDescriptor::User("jblow888".to_string()));
        assert_eq!(YoutubeDescriptor::from_url("https://www.youtube.com/playlist?list=PL5AC656794EE191C1").unwrap(), YoutubeDescriptor::Playlist("PL5AC656794EE191C1".to_string()));
        assert_eq!(YoutubeDescriptor::from_url("https://www.youtube.com/playlist?list=PL78L-9twndz8fMRU3NpiWSmB5IucqWuTF").unwrap(), YoutubeDescriptor::Playlist("PL78L-9twndz8fMRU3NpiWSmB5IucqWuTF".to_string()));
        assert_eq!(YoutubeDescriptor::from_url("https://www.youtube.com/watch?v=YdSdvIRkkDY").unwrap(), YoutubeDescriptor::Video("YdSdvIRkkDY".to_string()));
    }

//    #[test]
//    fn test_folder_for_url() {
//        assert_eq!(folder_for_url("https://www.youtube.com/channel/UChBBWt5H8uZW1LSOh_aPt2Q/videos"), "UChBBWt5H8uZW1LSOh_aPt2Q");
//        assert_eq!(folder_for_url("https://www.youtube.com/user/jblow888/videos"), "jblow888");
//        assert_eq!(folder_for_url("https://www.youtube.com/playlist?list=PL5AC656794EE191C1"), "PL5AC656794EE191C1");
//        assert_eq!(folder_for_url("https://www.youtube.com/playlist?list=PL78L-9twndz8fMRU3NpiWSmB5IucqWuTF"), "PL78L-9twndz8fMRU3NpiWSmB5IucqWuTF");
//    }
//
//    #[test]
//    fn test_get_canonical_url() {
//        // Playlist is playlist if it exists
//        assert_eq!(
//            get_canonical_url("https://www.youtube.com/playlist?list=PL5AC656794EE191C1").unwrap(),
//            Some("https://www.youtube.com/playlist?list=PL5AC656794EE191C1")
//        );
//        assert_eq!(
//            get_canonical_url("https://www.youtube.com/playlist?list=PL78L-9twndz8fMRU3NpiWSmB5IucqWuTF").unwrap(),
//            Some("https://www.youtube.com/playlist?list=PL78L-9twndz8fMRU3NpiWSmB5IucqWuTF")
//        );
//        // Playlist is None if it doesn't exist
//        assert_eq!(
//            get_canonical_url("https://www.youtube.com/playlist?list=PL78L-9twndz8fMRU3NpiWSmB5IucqWuTa").unwrap(),
//            None
//        );
//        // User is user if it exists
//        assert_eq!(
//            get_canonical_url("https://www.youtube.com/user/jblow888/videos").unwrap(),
//            Some("https://www.youtube.com/user/jblow888/videos")
//        );
//        // User is always converted to canonical case
//        assert_eq!(
//            get_canonical_url("https://www.youtube.com/user/JBlow888/videos").unwrap(),
//            Some("https://www.youtube.com/user/jblow888/videos")
//        );
//        assert_eq!(
//            get_canonical_url("https://www.youtube.com/user/cnn/videos").unwrap(),
//            Some("https://www.youtube.com/user/CNN/videos")
//        );
//        // User is None if it doesn't exist
//        assert_eq!(
//            get_canonical_url("https://www.youtube.com/user/jblow8888/videos").unwrap(),
//            None
//        );
//        // Channel is channel if it exists and has no username
//        assert_eq!(
//            get_canonical_url("https://www.youtube.com/channel/UChBBWt5H8uZW1LSOh_aPt2Q/videos").unwrap(),
//            Some("https://www.youtube.com/channel/UChBBWt5H8uZW1LSOh_aPt2Q/videos")
//        );
//        // Channel is user if it exists and has a username
//        assert_eq!(
//            get_canonical_url("https://www.youtube.com/channel/UCupvZG-5ko_eiXAupbDfxWw").unwrap(),
//            Some("https://www.youtube.com/user/CNN/videos")
//        );
//        // Channel is None if it doesn't exist
//        assert_eq!(
//            get_canonical_url("https://www.youtube.com/channel/UChBBWt5H8uZW1LSOh_aPt2a/videos").unwrap(),
//            None
//        );
//    }
}
