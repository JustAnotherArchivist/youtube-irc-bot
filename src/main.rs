/*
 * url-bot-rs
 *
 * URL parsing IRC bot
 *
 */

extern crate irc;
extern crate hyper;
extern crate curl;
extern crate htmlescape;

use curl::easy::{Easy2, Handler, WriteError};
use irc::client::prelude::*;
use htmlescape::decode_html;

/* Message { tags: None, prefix: Some("edcragg!edcragg@ip"), command: PRIVMSG("#music", "test") } */

fn main() {

    let server = IrcServer::new("config.toml").unwrap();
    server.identify().unwrap();
    server.for_each_incoming(|message| {

        match message.command {

            Command::PRIVMSG(ref target, ref msg) => {

                let tokens: Vec<_> = msg.split_whitespace().collect();

                for t in tokens {
                    let mut title = None;

                    let url;
                    match t.parse::<hyper::Uri>() {
                        Ok(u) => { url = u; }
                        _     => { continue; }
                    }

                    match url.scheme() {
                        Some("http")  => { title = resolve_url(t); }
                        Some("https") => { title = resolve_url(t); }
                        _ => ()
                    }

                    match title {
                        Some(s) => {
                            server.send_privmsg(
                                message.response_target().unwrap_or(target), &s
                            ).unwrap();
                        }
                        _ => ()
                    }
                }
            }

            _ => (),
        }

    }).unwrap()
}

#[derive(Debug)]
struct Collector(Vec<u8>);

impl Handler for Collector {
    fn write(&mut self, data: &[u8]) -> Result<usize, WriteError> {
        self.0.extend_from_slice(data);
        Ok(data.len())
    }
}

fn resolve_url(url: &str) -> Option<String> {

    println!("RESOLVE {}", url);

    let mut easy = Easy2::new(Collector(Vec::new()));

    easy.get(true).unwrap();
    easy.url(url).unwrap();
    easy.follow_location(true).unwrap();
    easy.useragent("url-bot-rs/0.1").unwrap();

    match easy.perform() {
        Err(_) => { return None; }
        _      => ()
    }

    let contents = easy.get_ref();

    let s = String::from_utf8_lossy(&contents.0);

    let s1: Vec<_> = s.split("<title>").collect();
    if s1.len() < 2 { return None }
    let s2: Vec<_> = s1[1].split("</title>").collect();
    if s2.len() < 2 { return None }

    let title_enc = s2[0];

    let mut title_dec = String::new();
    match decode_html(title_enc) {
        Ok(s) => { title_dec = s; }
        _     => ()
    };

    match title_dec.chars().count() {
        0 => None,
        _ => {
            let res = title_dec.to_string();
            println!("SUCCESS \"{}\"", res);
            Some(res)
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_urls() {
        assert_ne!(None, resolve_url("https://youtube.com"));
        assert_ne!(None, resolve_url("https://google.co.uk"));
        assert_eq!(None, resolve_url("https://github.com/nuxeh/url-bot-rs/commit/26cece9bc6d8f469ec7cd8c2edf86e190b5a597e.patch"));
        assert_eq!(None, resolve_url("https://upload.wikimedia.org/wikipedia/commons/5/55/Toad_and_spiny_lumpsuckers.jpg"));
        assert_eq!(None, resolve_url("https://i.redd.it/cvgvwb3bi3c01.jpg"));
    }
}
