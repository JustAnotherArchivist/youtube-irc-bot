extern crate youtube_irc_bot;

use std::io::{self, BufRead};
use youtube_irc_bot::message::get_folder;
use youtube_irc_bot::http::get_youtube_channel;

fn main() {
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let url = line.unwrap();
        let channel_url = format!("https://www.youtube.com/channel/{}", get_youtube_channel(&url).unwrap());
        let out = match get_folder(&channel_url) {
            Ok(url) => url,
            Err(err) => format!("# {:?}", err)
        };
        println!("{}", out);
    }
}
