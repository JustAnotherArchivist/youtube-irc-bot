use irc::client::prelude::*;
use std::str;
use std::collections::HashMap;
use std::process;
use once_cell::sync::Lazy;
use regex::Regex;
use snafu::{ensure, ResultExt, Snafu, Backtrace};
use ::phf::{Map, phf_map};

use super::config::{Rtd, HighlightMode};

enum VideoSize {
    Normal,
    VeryBig
}

#[derive(Debug, Snafu)]
#[snafu(visibility = "pub(crate)")]
pub enum Error {
    TomlEncode { source: toml::ser::Error },
    TomlDecode { source: toml::de::Error },
    Io { source: std::io::Error, backtrace: Backtrace },
    Utf8 { source: std::str::Utf8Error, backtrace: Backtrace },
    UrlTooLong,
    #[snafu(display("Unsupported URL: {}", url))]
    UnsupportedUrl { url: String },
    #[snafu(display("Not authorized"))]
    NotAuthorized,
    #[snafu(display("Could not get channel identifier"))]
    CouldNotGetChannelIdentifier,
    #[snafu(display("Invalid task name: {}", task))]
    InvalidTaskName { task: String },
    #[snafu(display("Not implemented: {}", what))]
    NotImplemented { what: String },
    #[snafu(display("Internal error listing files for {}", folder))]
    ErrorListingFiles { folder: String },
    #[snafu(display("Internal error creating folder {}", folder))]
    ErrorCreatingFolder { folder: String },
}

pub(crate) type Result<T, E = Error> = std::result::Result<T, E>;

fn contents_for_url(url: &str) -> Result<String> {
    let output = process::Command::new("get-youtube-page").arg(&url).output().context(Io)?;
    let body = str::from_utf8(&output.stdout).context(Utf8)?;
    Ok(body.into())
}

fn extract_username(page_contents: &str) -> Option<String> {
    static USER_RE: &Lazy<Regex> = lazy_regex!(r#"<link itemprop="url" href="http://www.youtube.com/user/([^"]+)">"#);
    let user = USER_RE.captures(page_contents)?.get(1)?.as_str();
    Some(user.to_string())
}

fn extract_channel_id(page_contents: &str) -> Result<String> {
    static CHANNEL_META_RE: &Lazy<Regex> = lazy_regex!(r#"<meta itemprop="channelId" content="([^"]+)">"#);
    static CHANNEL_VIDEO_DETAILS_RE: &Lazy<Regex> = lazy_regex!(r#" ytplayer = .+?\\"channelId\\":\\"([^"]+)\\""#);
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

#[allow(clippy::let_and_return)]
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

// Map of username -> folder for channels for which we do not want to store
// videos in the folder `username`
static FOLDER_EXCEPTIONS: Map<&'static str, &'static str> = phf_map! {
    "TEDxTalks" => "UCsT0YIqwnpJCM-mx7-gSA4Q",
};

#[derive(Debug, PartialEq, Eq)]
pub enum YoutubeDescriptor {
    User(String),
    Channel(String),
    Playlist(String),
    Video(String),
}

impl YoutubeDescriptor {
    pub fn from_url(url: &str) -> Result<YoutubeDescriptor> {
        static WATCH_RE:        &Lazy<Regex> = lazy_regex!(r#"\Ahttps://www.youtube.com/watch.*[\?&]v=([-_A-Za-z0-9]{11})([#\&].*)?\z"#);
        static OLD_PLAYLIST_RE: &Lazy<Regex> = lazy_regex!(r#"\Ahttps://www.youtube.com/playlist.*[\?&]list=(PL[0-9A-F]{16})([#\&].*)?\z"#);
        static NEW_PLAYLIST_RE: &Lazy<Regex> = lazy_regex!(r#"\Ahttps://www.youtube.com/playlist.*[\?&]list=(PL[-_A-Za-z0-9]{32})([#\&].*)?\z"#);
        static CHANNEL_RE:      &Lazy<Regex> = lazy_regex!(r#"\Ahttps://www.youtube.com/channel/(UC[-_A-Za-z0-9]{22})([/#\?].*)?\z"#);
        static USER_RE:         &Lazy<Regex> = lazy_regex!(r#"\Ahttps://www.youtube.com/user/([A-Za-z0-9]{1,20})([/#\?].*)?\z"#);

        let url = fix_youtube_url(url);
        ensure!(url.starts_with("https://www.youtube.com/"), UnsupportedUrl { url });
        if let Some(matches) = WATCH_RE.captures(&url) {
            return Ok(YoutubeDescriptor::Video(matches.get(1).unwrap().as_str().to_string()));
        }
        if let Some(matches) = OLD_PLAYLIST_RE.captures(&url) {
            return Ok(YoutubeDescriptor::Playlist(matches.get(1).unwrap().as_str().to_string()));
        }
        if let Some(matches) = NEW_PLAYLIST_RE.captures(&url) {
            return Ok(YoutubeDescriptor::Playlist(matches.get(1).unwrap().as_str().to_string()));
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
                CanonicalizedYoutubeDescriptor { kind: FetchType::Video, id: id.clone(), folder }
            },
            YoutubeDescriptor::Playlist(id) => {
                CanonicalizedYoutubeDescriptor { kind: FetchType::Playlist, id: id.clone(), folder: id.clone() }
            },
            YoutubeDescriptor::Channel(_) | YoutubeDescriptor::User(_) => {
                let contents = contents_for_url(&self.to_url())?;
                let username = extract_username(&contents);
                match username {
                    None => {
                        let channel_id = extract_channel_id(&contents)?;
                        CanonicalizedYoutubeDescriptor { kind: FetchType::Channel, id: channel_id.clone(), folder: channel_id }
                    }
                    Some(username) => {
                        let mut folder = username.clone();
                        if let Some(&custom_folder) = FOLDER_EXCEPTIONS.get(username.as_str()) {
                            folder = custom_folder.to_string();
                        }
                        CanonicalizedYoutubeDescriptor { kind: FetchType::User, id: username, folder }
                    }
                }
            },
        })
    }
}

fn archive(original_url: &str, descriptor: &CanonicalizedYoutubeDescriptor, video_size: VideoSize, user: &str, rtd: &Rtd) -> Result<String> {
    let folder = descriptor.folder();
    let sessions = get_downloader_sessions()?;
    if let Some(_session) = sessions.iter().find(|session| session.identifier == folder) {
        return Ok(format!("Can't archive {} because another task is running in the same folder {}", &original_url, &folder));
    }
    match descriptor.kind {
        FetchType::Video => {
            let command = match video_size {
                VideoSize::Normal  => "grab-youtube-video",
                VideoSize::VeryBig => "grab-youtube-video-big-video"
            };
            let output = process::Command::new(command)
                .arg(&folder).arg(&descriptor.to_url())
                .output()
                .context(Io)?;
            let _ = str::from_utf8(&output.stdout).context(Utf8)?;
        },
        FetchType::Channel | FetchType::User | FetchType::Playlist => {
            let tasks_limit = limit_for_user(user, rtd);
            if sessions.len() >= tasks_limit {
                return Ok(format!("Can't archive {} because too many tasks are running (your limit = {}), try again later", &original_url, tasks_limit));
            }
            let videos_limit = 999_999;
            let command = match video_size {
                VideoSize::Normal  => "grab-youtube-channel",
                VideoSize::VeryBig => "grab-youtube-channel-big-videos"
            };
            let output = process::Command::new(command)
                .arg(&folder).arg(videos_limit.to_string())
                .output()
                .context(Io)?;
            let _ = str::from_utf8(&output.stdout).context(Utf8)?;
        }
    }
    Ok(format!("Grabbing {} -> {}; check {} later", &original_url, &folder, logs_url(&folder)))
}

fn assert_valid_task_name(task: &str) -> Result<()> {
    static TASK_NAME_RE: &Lazy<Regex> = lazy_regex!(r"\A[-_A-Za-z0-9]+\z");
    ensure!(TASK_NAME_RE.is_match(task), InvalidTaskName { task });
    Ok(())
}

fn abort(task: &str) -> Result<String> {
    assert_valid_task_name(task)?;
    let session = format!("YouTube-{}", task);
    let _output = process::Command::new("tmux")
        .arg("send-keys").arg("-t").arg(&session).arg("C-c")
        .output()
        .context(Io)?;
    Ok(format!("Aborted {}", &task))
}

fn limit_for_user(user: &str, rtd: &Rtd) -> usize {
    let user_limits = &rtd.conf.user_limits;
    match user_limits.get(user) {
        None         => rtd.conf.params.task_limit,
        Some(&limit) => limit
    }
}

fn logs_url(folder: &str) -> String {
    format!("https://ya.borg.xyz/logs/dl/{}/", &folder)
}

fn check_folder(folder: &str) -> Result<String> {
    assert_valid_task_name(folder)?;
    let listing = match get_file_listing(&folder) {
        Err(_) => return Err(Error::ErrorListingFiles { folder: folder.to_string() }),
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
    Ok(format!("stash has {} videos for {} ({}); latest {:?}", videos.len(), &folder, logs_url(&folder), latest_videos))
}

fn check_stash(descriptor: &CanonicalizedYoutubeDescriptor) -> Result<String> {
    if descriptor.kind == FetchType::Video {
        return Err(Error::NotImplemented { what: "/s on /watch? URL".into() });
    }
    let folder = descriptor.folder();
    check_folder(&folder)
}

fn get_file_listing(folder: &str) -> Result<Vec<String>> {
    let output = process::Command::new("ts")
        .arg("ls").arg("-n").arg("YouTube").arg("-j").arg("-t").arg(folder)
        .output()
        .context(Io)?;
    let stdout_utf8 = str::from_utf8(&output.stdout).context(Utf8)?;
    Ok(stdout_utf8.lines().map(String::from).collect())
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
    let sessions = get_downloader_sessions()?;
    let scripts = process::Command::new("get-running-youtube-scripts").output().context(Io)?.stdout;
    let num_scripts = bytecount::count(&scripts, b'\n');
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

fn get_help() -> Result<String> {
    Ok(
        "Usage: \
        !help | \
        !status | \
        !s <URL or folder> | \
        !a <URL> | \
        !sa <URL> | \
        !averybig <URL w/ very large videos> | \
        !saverybig <URL w/ very large videos> | \
        !abort <task> | \
        !stopscripts | \
        !contscripts".to_string()
    )
}

const ALPHA_REGULAR: &str      = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const ALPHA_FRAKTUR: &str      = "𝔄𝔅ℭ𝔇𝔈𝔉𝔊ℌℑ𝔍𝔎𝔏𝔐𝔑𝔒𝔓𝔔ℜ𝔖𝔗𝔘𝔙𝔚𝔛𝔜ℨ𝔞𝔟𝔠𝔡𝔢𝔣𝔤𝔥𝔦𝔧𝔨𝔩𝔪𝔫𝔬𝔭𝔮𝔯𝔰𝔱𝔲𝔳𝔴𝔵𝔶𝔷";
const ALPHA_FRAKTUR_BOLD: &str = "𝕬𝕭𝕮𝕯𝕰𝕱𝕲𝕳𝕴𝕵𝕶𝕷𝕸𝕹𝕺𝕻𝕼𝕽𝕾𝕿𝖀𝖁𝖂𝖃𝖄𝖅𝖆𝖇𝖈𝖉𝖊𝖋𝖌𝖍𝖎𝖏𝖐𝖑𝖒𝖓𝖔𝖕𝖖𝖗𝖘𝖙𝖚𝖛𝖜𝖝𝖞𝖟";
const ALPHA_SCRIPT: &str       = "𝒜ℬ𝒞𝒟ℰℱ𝒢ℋℐ𝒥𝒦ℒℳ𝒩𝒪𝒫𝒬ℛ𝒮𝒯𝒰𝒱𝒲𝒳𝒴𝒵𝒶𝒷𝒸𝒹ℯ𝒻ℊ𝒽𝒾𝒿𝓀𝓁𝓂𝓃ℴ𝓅𝓆𝓇𝓈𝓉𝓊𝓋𝓌𝓍𝓎𝓏";
const ALPHA_BOLD: &str         = "𝐀𝐁𝐂𝐃𝐄𝐅𝐆𝐇𝐈𝐉𝐊𝐋𝐌𝐍𝐎𝐏𝐐𝐑𝐒𝐓𝐔𝐕𝐖𝐗𝐘𝐙𝐚𝐛𝐜𝐝𝐞𝐟𝐠𝐡𝐢𝐣𝐤𝐥𝐦𝐧𝐨𝐩𝐪𝐫𝐬𝐭𝐮𝐯𝐰𝐱𝐲𝐳";
const ALPHA_ITALIC: &str       = "𝐴𝐵𝐶𝐷𝐸𝐹𝐺𝐻𝐼𝐽𝐾𝐿𝑀𝑁𝑂𝑃𝑄𝑅𝑆𝑇𝑈𝑉𝑊𝑋𝑌𝑍𝑎𝑏𝑐𝑑𝑒𝑓𝑔ℎ𝑖𝑗𝑘𝑙𝑚𝑛𝑜𝑝𝑞𝑟𝑠𝑡𝑢𝑣𝑤𝑥𝑦𝑧";
const ALPHA_BOLD_ITALIC: &str  = "𝑨𝑩𝑪𝑫𝑬𝑭𝑮𝑯𝑰𝑱𝑲𝑳𝑴𝑵𝑶𝑷𝑸𝑹𝑺𝑻𝑼𝑽𝑾𝑿𝒀𝒁𝒂𝒃𝒄𝒅𝒆𝒇𝒈𝒉𝒊𝒋𝒌𝒍𝒎𝒏𝒐𝒑𝒒𝒓𝒔𝒕𝒖𝒗𝒘𝒙𝒚𝒛";

fn replace_matching_characters(input_str: &str, from_set: &str, to_set: &str) -> String {
    let mapping = from_set.chars().zip(to_set.chars()).collect::<HashMap<_, _>>();
    input_str
        .chars()
        .map(|c| *mapping.get(&c).unwrap_or(&c))
        .collect::<String>()
}

fn highlight_for_user(user: &str, rtd: &Rtd) -> String {
    let user_highlights = &rtd.conf.user_highlights;
    if let Some(mode) = user_highlights.get(user) {
        match mode {
            HighlightMode::Normal      => user.to_string(),
            HighlightMode::Fraktur     => replace_matching_characters(user, ALPHA_REGULAR, ALPHA_FRAKTUR),
            HighlightMode::FrakturBold => replace_matching_characters(user, ALPHA_REGULAR, ALPHA_FRAKTUR_BOLD),
            HighlightMode::Script      => replace_matching_characters(user, ALPHA_REGULAR, ALPHA_SCRIPT),
            HighlightMode::Bold        => replace_matching_characters(user, ALPHA_REGULAR, ALPHA_BOLD),
            HighlightMode::Italic      => replace_matching_characters(user, ALPHA_REGULAR, ALPHA_ITALIC),
            HighlightMode::BoldItalic  => replace_matching_characters(user, ALPHA_REGULAR, ALPHA_BOLD_ITALIC),
        }
    } else {
        user.to_string()
    }
}

fn send_reply(client: &IrcClient, channel: &str, user: &str, result: Result<String>, rtd: &Rtd) {
    let user = highlight_for_user(user, rtd);
    match result {
        Ok(reply) => client.send_privmsg(channel, format!("{}: {}", user, reply)).unwrap(),
        Err(err)  => client.send_privmsg(channel, format!("{}: error: {}", user, err)).unwrap(),
    }
}

fn extract_url(msg: &str) -> Result<&str> {
    let url = msg.split(' ').take(2).last().unwrap();
    ensure!(url.len() <= 200, UrlTooLong);
    Ok(url)
}

pub fn dispatch_message(message: &str, user: &str, rtd: &Rtd, check_authorization: impl Fn() -> Result<()>) -> Result<Vec<Result<String>>> {
    Ok(match message {
        "!help" => {
            vec![get_help()]
        },
        "!status" => {
            vec![get_status(rtd)]
        },
        "!stopscripts" => {
            check_authorization()?;
            vec![stop_scripts()]
        },
        "!contscripts" => {
            check_authorization()?;
            vec![cont_scripts()]
        },
        msg if msg.starts_with("!s ") => {
            let url_or_folder = extract_url(msg)?;
            if url_or_folder.starts_with("https://") || url_or_folder.starts_with("http://") {
                let descriptor = YoutubeDescriptor::from_url(&url_or_folder)?.canonicalize()?;
                vec![check_stash(&descriptor)]
            } else {
                vec![check_folder(&url_or_folder)]
            }
        },
        msg if msg.starts_with("!a ") => {
            check_authorization()?;
            let url = extract_url(msg)?;
            let descriptor = YoutubeDescriptor::from_url(&url)?.canonicalize()?;
            vec![archive(&url, &descriptor, VideoSize::Normal, &user, &rtd)]
        },
        msg if msg.starts_with("!sa ") => {
            check_authorization()?;
            let url = extract_url(msg)?;
            let descriptor = YoutubeDescriptor::from_url(&url)?.canonicalize()?;
            vec![
                check_stash(&descriptor),
                archive(&url, &descriptor, VideoSize::Normal, &user, &rtd)
            ]
        },
        msg if msg.starts_with("!averybig ") => {
            check_authorization()?;
            let url = extract_url(msg)?;
            let descriptor = YoutubeDescriptor::from_url(&url)?.canonicalize()?;
            vec![archive(&url, &descriptor, VideoSize::VeryBig, &user, &rtd)]
        },
        msg if msg.starts_with("!saverybig ") => {
            check_authorization()?;
            let url = extract_url(msg)?;
            let descriptor = YoutubeDescriptor::from_url(&url)?.canonicalize()?;
            vec![
                check_stash(&descriptor),
                archive(&url, &descriptor, VideoSize::VeryBig, &user, &rtd)
            ]
        },
        msg if msg.starts_with("!abort ") => {
            check_authorization()?;
            let task = extract_url(msg)?;
            vec![abort(&task)]
        },
        _other => vec![],
    })
}

const NO_WEBCHAT_MESSAGE: &str =
    "webchat users are not authorized; use any other IRC client, or ask someone else to do it";

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

    static WEBCHAT_RE: &Lazy<Regex> = lazy_regex!(r"\A.+!webchat@.+\z");
    let check_authorization = || {
        if let Some(prefix) = &message.prefix {
            if WEBCHAT_RE.is_match(prefix) {
                send_reply(client, channel, user, Ok(NO_WEBCHAT_MESSAGE.into()), rtd);
                return Err(Error::NotAuthorized);
            }
        }
        Ok(())
    };

    if message.response_target() == Some(channel) {
        // Special handling
        if user == "Ryz" && msg.starts_with("!archive ") {
            client.send(format!("KICK {} Ryz unknown command", channel).as_str()).unwrap();
        }

        let replies = dispatch_message(&msg, &user, &rtd, &check_authorization);
        match replies {
            Err(err) => {
                client.send_privmsg(channel, format!("{}: error: {}", user, err)).unwrap()
            },
            Ok(replies) => {
                for reply in replies.into_iter() {
                    send_reply(client, channel, user, reply, rtd);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_descriptor() {
        for trailing_crud in ["", "?", "?stuff", "#", "#stuff"].iter() {
            assert_eq!(
                YoutubeDescriptor::from_url(&format!("https://www.youtube.com/channel/UChBBWt5H8uZW1LSOh_aPt2Q{}", trailing_crud)).unwrap(),
                YoutubeDescriptor::Channel("UChBBWt5H8uZW1LSOh_aPt2Q".to_string()));
            assert_eq!(
                YoutubeDescriptor::from_url(&format!("https://www.youtube.com/channel/UChBBWt5H8uZW1LSOh_aPt2Q/videos{}", trailing_crud)).unwrap(),
                YoutubeDescriptor::Channel("UChBBWt5H8uZW1LSOh_aPt2Q".to_string()));
            assert_eq!(
                YoutubeDescriptor::from_url(&format!("https://www.youtube.com/user/jblow888{}", trailing_crud)).unwrap(),
                YoutubeDescriptor::User("jblow888".to_string()));
            assert_eq!(
                YoutubeDescriptor::from_url(&format!("https://www.youtube.com/user/jblow888/videos{}", trailing_crud)).unwrap(),
                YoutubeDescriptor::User("jblow888".to_string()));
        }

        for trailing_crud in ["", "&", "&stuff", "#", "#stuff"].iter() {
            assert_eq!(
                YoutubeDescriptor::from_url(&format!("https://www.youtube.com/playlist?list=PL5AC656794EE191C1{}", trailing_crud)).unwrap(),
                YoutubeDescriptor::Playlist("PL5AC656794EE191C1".to_string()));
            assert_eq!(
                YoutubeDescriptor::from_url(&format!("https://www.youtube.com/playlist?list=PL78L-9twndz8fMRU3NpiWSmB5IucqWuTF{}", trailing_crud)).unwrap(),
                YoutubeDescriptor::Playlist("PL78L-9twndz8fMRU3NpiWSmB5IucqWuTF".to_string()));
            assert_eq!(
                YoutubeDescriptor::from_url(&format!("https://www.youtube.com/watch?v=YdSdvIRkkDY{}", trailing_crud)).unwrap(),
                YoutubeDescriptor::Video("YdSdvIRkkDY".to_string()));
        }

        for bad_url in [
            "https://www.youtube.com/channel/UChBBWt5H8uZW1LSOh_aPt2",
            "https://www.youtube.com/channel/UChBBWt5H8uZW1LSOh_aPt2Qa",
        ].iter() {
            let error = format!("{:?}", YoutubeDescriptor::from_url(bad_url));
            assert!(error.starts_with("Err(UnsupportedUrl {"), error);
        }
    }

    #[test]
    fn test_replace_matching_characters() {
        assert_eq!(replace_matching_characters("user", ALPHA_REGULAR, ALPHA_FRAKTUR), "𝔲𝔰𝔢𝔯");
        assert_eq!(replace_matching_characters("User", ALPHA_REGULAR, ALPHA_FRAKTUR), "𝔘𝔰𝔢𝔯");
        assert_eq!(replace_matching_characters("_User0", ALPHA_REGULAR, ALPHA_FRAKTUR), "_𝔘𝔰𝔢𝔯0");
    }
}
