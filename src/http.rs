use htmlescape::decode_html;
use std::time::Duration;
use itertools::Itertools;
use regex::Regex;
use failure::Error;
use reqwest::Client;
use reqwest::header::{USER_AGENT, ACCEPT_LANGUAGE, CONTENT_TYPE};
use std::io::{Read, BufRead, BufReader};
use std::fs::File;
use mime::{Mime, TEXT, HTML};
use rand::thread_rng;
use rand::seq::SliceRandom;

use super::error::MyError;

fn contents_for_url(url: &str) -> Result<String, Error> {
    let proxies: Vec<String> = BufReader::new(File::open("/home/at/.config/youtube-dl/proxies").unwrap()).lines().map(|l| l.unwrap()).collect();
    let mut rng = thread_rng();
    let proxy_url = proxies.choose(&mut rng).unwrap();

    dbg!(proxy_url);

    let client = Client::builder()
        .proxy(reqwest::Proxy::all(proxy_url)?)
        .timeout(Duration::from_secs(10)) // per read/write op
        .build()?;

    let resp = client.get(url)
        .header(USER_AGENT, "curl/7.37.0") // Get the old no-Polymer pages
        .header(ACCEPT_LANGUAGE, "en")
        .send()?
        .error_for_status()?;

    let content_type = resp.headers().get(CONTENT_TYPE)
        .and_then(|typ| typ.to_str().ok())
        .and_then(|typ| typ.parse::<Mime>().ok());

    match content_type {
        Some(mime) => {
            match (mime.type_(), mime.subtype()) {
                (TEXT, HTML) => (),
                mime => {
                    return Err(MyError::new(format!("Expected text/html mime type but got {:?}", mime)).into());
                }
            }
        },
        None => {
            return Err(MyError::new("Expected text/html mime type but did not get a mime type".into()).into());
        }
    };

    let mut body = Vec::new();
    resp.take(10 * 1024 * 1024).read_to_end(&mut body)?;
    Ok(String::from_utf8_lossy(&body).into())
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
