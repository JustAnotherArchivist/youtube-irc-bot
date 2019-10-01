extern crate youtube_irc_bot;

use irc::client::prelude::*;
use std::process;
use std::path::PathBuf;
use structopt::StructOpt;

use youtube_irc_bot::config::Rtd;
use youtube_irc_bot::config::Args;
use youtube_irc_bot::message::handle_message;

#[derive(StructOpt, Debug)]
#[structopt(name = "youtube-irc-bot")]
/// IRC bot for ingesting URLs to archive
struct Opt {
    /// Show extra information
    #[structopt(short = "v", long)]
    verbose: bool,

    /// Print debugging information
    #[structopt(short = "D", long)]
    debug: bool,

    /// File to read configuration from
    #[structopt(short = "c", long, parse(from_os_str))]
    conf: Option<PathBuf>,
}

fn main() {
    let opt = Opt::from_args();

    let args = Args {
        flag_verbose: opt.verbose,
        flag_debug: opt.debug,
        flag_conf: opt.conf
    };

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
        let _ = handle_message(client, &message, &rtd);
        Ok(())
    });

    reactor.run().unwrap_or_else(|err| {
        eprintln!("IRC client error: {}", err);
        process::exit(1);
    });
}
