extern crate youtube_irc_bot;

use std::io::{self, BufRead};
use youtube_irc_bot::message::get_folder;

fn main() {
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = line.unwrap();
        let out = match get_folder(&line) {
            Ok(url) => url,
            Err(err) => format!("# {:?}", err)
        };
        println!("{}", out);
    }
}
