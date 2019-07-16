use htmlescape::decode_html;
use itertools::Itertools;
use regex::Regex;
use failure::Error;
use std::process;
use std::str;

use super::error::MyError;

fn contents_for_url(url: &str) -> Result<String, Error> {
    let output = process::Command::new("get-youtube-page")
        .arg(&url)
        .output()?;
    let body = str::from_utf8(&output.stdout)?;
    Ok(body.into())
}

pub fn get_youtube_user(url: &str) -> Result<Option<String>, Error> {
    let contents = contents_for_url(url)?;
    let user = parse_user(&contents);
    Ok(user.clone())
}

pub fn get_youtube_channel(url: &str) -> Result<String, Error> {
    let contents = contents_for_url(url)?;
    let channel = parse_channel(&contents);
    match channel {
        Some(channel) => Ok(channel.clone()),
        None => return Err(MyError::new(format!("Could not find channel identifier in {}", url)).into())
    }
}

fn parse_title(page_contents: &str) -> Option<String> {
    lazy_static! {
        static ref RE: Regex = Regex::new("<(?i:title).*?>((.|\n)*?)</(?i:title)>").unwrap();
    }
    let title_enc = RE.captures(page_contents)?.get(1)?.as_str();
    let title_dec = decode_html(title_enc).ok()?;

    // make any multi-line title string into a single line,
    // trim leading and trailing whitespace
    let title_one_line = title_dec
        .trim()
        .lines()
        .map(|line| line.trim())
        .join(" ");

    if title_one_line.is_empty() {
        return None;
    }

    Some(title_one_line)
}

fn parse_user(page_contents: &str) -> Option<String> {
    lazy_static! {
        static ref RE: Regex = Regex::new(r#"<link itemprop="url" href="http://www.youtube.com/user/([^"]+)">"#).unwrap();
    }
    let user = RE.captures(page_contents)?.get(1)?.as_str();
    Some(user.to_string())
}

fn parse_channel(page_contents: &str) -> Option<String> {
    lazy_static! {
        static ref CHANNEL_META_RE: Regex = Regex::new(r#"<meta itemprop="channelId" content="([^"]+)">"#).unwrap();
        static ref CHANNEL_VIDEO_DETAILS_RE: Regex = Regex::new(r#" ytplayer = .+?\\"channelId\\":\\"([^"]+)\\""#).unwrap();
    }
    match &CHANNEL_META_RE.captures(page_contents) {
        Some(captures) if captures.len() >= 1 => Some(captures.get(1).unwrap().as_str().to_owned()),
        _ => match &CHANNEL_VIDEO_DETAILS_RE.captures(page_contents) {
            Some(captures) if captures.len() >= 1 => Some(captures.get(1).unwrap().as_str().to_owned()),
            _ => None
        }
    }
}
