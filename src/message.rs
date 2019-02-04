use irc::client::prelude::*;
use std::iter;
use std::str;
use std::error;
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};
use unicode_segmentation::UnicodeSegmentation;
use reqwest::Url;
use regex::Regex;
use itertools::Itertools;

use super::http::resolve_url;
use super::sqlite::{Database, NewLogEntry};
use super::config::Rtd;

pub fn handle_message(
    client: &IrcClient, message: &Message, rtd: &Rtd, db: &Database
) {
    // print the message if debug flag is set
    if rtd.args.flag_debug {
        eprintln!("{:?}", message.command)
    }

    // match on message type
    let (target, msg) = match message.command {
        Command::PRIVMSG(ref target, ref msg) => (target, msg),
        _ => return,
    };

    let user = message.source_nickname().unwrap();
    let mut num_processed = 0;

    // look at each space-separated message token
    for token in msg.split_whitespace() {
        // limit the number of processed URLs
        if num_processed == rtd.conf.params.url_limit {
            break;
        }

        // the token must be a valid url
        let url = match token.parse::<Url>() {
            Ok(url) => url,
            _ => continue,
        };

        // the token must not contain unsafe characters
        if contains_unsafe_chars(token) {
            continue;
        }

        // the schema must be http or https
        if !["http", "https"].contains(&url.scheme()) {
            continue;
        }

        // try to get the title from the url
        let title = match resolve_url(token, rtd) {
            Ok(title) => title,
            Err(err) => {
                println!("ERROR {:?}", err);
                continue
            },
        };

        // create a log entry struct
        let entry = NewLogEntry {
            title: &title,
            url: token,
            user,
            channel: target,
        };

        // check for pre-post
        let mut msg = match if rtd.history {
            db.check_prepost(token)
        } else {
            Ok(None)
        } {
            Ok(Some(previous_post)) => {
                let user = if rtd.conf.features.mask_highlights {
                    create_non_highlighting_name(&previous_post.user)
                } else {
                    previous_post.user
                };
                format!("⤷ {} → {} {} ({})",
                    title,
                    previous_post.time_created,
                    user,
                    previous_post.channel
                )
            },
            Ok(None) => {
                // add new log entry to database
                if rtd.history {
                    if let Err(err) = db.add_log(&entry) {
                        eprintln!("SQL error: {}", err);
                    }
                }
                format!("⤷ {}", title)
            },
            Err(err) => {
                eprintln!("SQL error: {}", err);
                continue
            },
        };

        // limit response length, see RFC1459
        msg = utf8_truncate(&msg, 510);

        // send the IRC response
        let target = message.response_target().unwrap_or(target);
        if rtd.conf.features.send_notice {
            client.send_notice(target, &msg).unwrap()
        } else {
            client.send_privmsg(target, &msg).unwrap()
        }

        num_processed += 1;
    };

    let command_channel = &rtd.conf.params.command_channel;
    if message.response_target() == Some(command_channel) {
        if msg.starts_with("!status") {
            client.send_privmsg(command_channel, get_status()).unwrap();
        }
    }
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
    return format!("{}", duration);
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
}
