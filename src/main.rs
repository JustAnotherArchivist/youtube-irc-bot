extern crate irc;
extern crate docopt;
#[macro_use]
extern crate serde_derive;
extern crate itertools;
extern crate regex;
#[macro_use]
extern crate lazy_static;
extern crate htmlescape;
extern crate time;
extern crate reqwest;
extern crate mime;
extern crate humansize;
extern crate unicode_segmentation;
extern crate toml;
extern crate directories;
extern crate url;

mod http;
mod config;
mod message;
mod error;

use docopt::Docopt;
use irc::client::prelude::*;
use std::process;
use std::path::PathBuf;

use self::config::Rtd;
use self::message::handle_message;

// docopt usage string
const USAGE: &str = "
Helpful IRC bot.

Usage:
    youtube-irc-bot [options]

Options:
    -h --help       Show this help message.
    -v --verbose    Show extra information.
    -D --debug      Print debugging information.
    -c --conf=PATH  Use configuration file at PATH.
";

#[derive(Debug, Deserialize, Default)]
pub struct Args {
    flag_verbose: bool,
    flag_debug: bool,
    flag_conf: Option<PathBuf>,
}

fn main() {
    // parse command line arguments with docopt
    let args: Args = Docopt::new(USAGE)
        .unwrap()

        .deserialize()
        .unwrap_or_else(|e| e.exit());

    // get a run-time configuration data structure
    let rtd: Rtd = Rtd::from_args(args).unwrap_or_else(|err| {
        eprintln!("Error loading configuration: {}", err);
        process::exit(1);
    });

    println!("Using configuration: {}", rtd.paths.conf.display());
    if rtd.args.flag_verbose {
        println!("\n[features]\n{}", rtd.conf.features);
        println!("[parameters]\n{}", rtd.conf.params);
    }

    // create IRC reactor
    let mut reactor = IrcReactor::new().unwrap();
    let client = reactor
        .prepare_client_and_connect(&rtd.conf.client)
        .unwrap_or_else(|err| {
        eprintln!("IRC prepare error: {}", err);
        process::exit(1);
    });
    client.identify().unwrap();

    // register handler
    reactor.register_client_with_handler(client, move |client, message| {
        handle_message(client, &message, &rtd);
        Ok(())
    });

    reactor.run().unwrap_or_else(|err| {
        eprintln!("IRC client error: {}", err);
        process::exit(1);
    });
}
