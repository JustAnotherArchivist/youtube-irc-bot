extern crate youtube_irc_bot;

use std::io::{self, BufRead};
use youtube_irc_bot::message::YoutubeDescriptor;

fn main() {
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let url = line.unwrap();
        let descriptor = YoutubeDescriptor::from_url(&url).unwrap().canonicalize().unwrap();
        let folder = descriptor.folder();
        println!("{}", folder);
    }
}
