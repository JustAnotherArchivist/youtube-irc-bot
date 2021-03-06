/*
 * Application configuration
 *
 */
use std::fs;
use std::fs::File;
use std::io::Write;
use std::collections::HashMap;
use toml;
use std::path::{Path, PathBuf};
use irc::client::data::Config as IrcConfig;
use std::fmt;
use directories::{ProjectDirs, BaseDirs};
use snafu::ResultExt;
use crate::message::{Result, TomlEncode, TomlDecode, Io};

#[derive(Debug, Deserialize, Default)]
pub struct Args {
    pub flag_verbose: bool,
    pub flag_debug: bool,
    pub flag_conf: Option<PathBuf>,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum HighlightMode {
    Normal,
    Fraktur,
    FrakturBold,
    Script,
    Bold,
    Italic,
    BoldItalic,
}

// serde structures defining the configuration file structure
#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Conf {
    pub features: Features,
    #[serde(rename = "parameters")]
    pub params: Parameters,
    #[serde(rename = "connection")]
    pub client: IrcConfig,
    pub user_limits: HashMap<String, usize>,
    pub user_highlights: HashMap<String, HighlightMode>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Features {
    pub mask_highlights: bool,
    pub send_notice: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Parameters {
    pub task_limit: usize,
    pub command_channel: String,
}

impl Default for Parameters {
    fn default() -> Self {
        Self {
            task_limit: 34,
            command_channel: "#youtubearchive".to_string(),
        }
    }
}

impl Conf {
    // load configuration TOML from a file
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let conf = fs::read_to_string(path.as_ref()).context(Io)?;
        let conf: Conf = toml::de::from_str(&conf).context(TomlDecode)?;
        Ok(conf)
    }

    // write configuration to a file
    pub fn write(self, path: impl AsRef<Path>) -> Result<()> {
        let mut file = File::create(path).context(Io)?;
        file.write_all(toml::ser::to_string(&self).context(TomlEncode)?.as_bytes()).context(Io)?;
        Ok(())
    }
}

impl Default for Conf {
    fn default() -> Self {
        Self {
            features: Features::default(),
            params: Parameters::default(),
            client: IrcConfig {
                nickname: Some("botnick".to_string()),
                alt_nicks: Some(vec!["botnick_".to_string()]),
                nick_password: Some("".to_string()),
                username: Some("botnick".to_string()),
                realname: Some("botnick".to_string()),
                server: Some("chat.freenode.net".to_string()),
                port: Some(6697),
                password: Some("".to_string()),
                use_ssl: Some(true),
                encoding: Some("UTF-8".to_string()),
                channels: Some(vec![]),
                user_info: Some("Helpful bot".to_string()),
                source: Some("https://github.com/video-archivist/youtube-irc-bot".to_string()),
                ping_time: Some(180),
                ping_timeout: Some(10),
                burst_window_length: Some(8),
                max_messages_in_burst: Some(15),
                should_ghost: Some(false),
                ..IrcConfig::default()
            },
            user_limits: HashMap::new(),
            user_highlights: HashMap::new(),
        }
    }
}

// run time data structure. this is used to pass around mutable runtime data
// where it's needed, including command line arguments, configuration file
// settings, any parameters defined based on both of these sources, and
// any other data used at runtime
#[derive(Default)]
pub struct Rtd {
    // paths
    pub paths: Paths,
    // configuration file data
    pub conf: Conf,
    // command-line arguments
    pub args: Args,
}

#[derive(Default)]
pub struct Paths {
    pub conf: PathBuf,
}

impl Rtd {
    pub fn from_args(args: Args) -> Result<Self> {
        let mut rtd = Rtd::default();

        // move command line arguments
        rtd.args = args;

        // get a config file path
        let dirs = ProjectDirs::from("org", "", "youtube-irc-bot").unwrap();
        rtd.paths.conf = match rtd.args.flag_conf {
            // configuration file path specified as command line parameter
            Some(ref cp) => expand_tilde(cp),
            // default path
            _ => dirs.config_dir().join("config.toml")
        };

        // check if config directory exists, create it if it doesn't
        create_dir_if_missing(rtd.paths.conf.parent().unwrap())?;

        // create a default config if it doesn't exist
        if !rtd.paths.conf.exists() {
            eprintln!(
                "Configuration `{}` doesn't exist, creating default",
                rtd.paths.conf.to_str().unwrap()
            );
            eprintln!(
                "You should modify this file to include a useful IRC configuration"
            );
            Conf::default().write(&rtd.paths.conf)?;
        }

        // load config file
        rtd.conf = Conf::load(&rtd.paths.conf)?;

        Ok(rtd)
    }
}

// implementation of Display trait for multiple structs above
macro_rules! impl_display {
    ($($t:ty),+) => {
        $(impl fmt::Display for $t {
            fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "{}", toml::ser::to_string(self).unwrap())
            }
        })+
    }
}
impl_display!(Features, Parameters);

fn create_dir_if_missing(dir: &Path) -> Result<bool> {
    let pdir = dir.to_str().unwrap();
    let exists = pdir.is_empty() || dir.exists();
    if !exists {
        eprintln!("Directory `{}` doesn't exist, creating it", pdir);
        fs::create_dir_all(dir).context(Io)?;
    }
    Ok(exists)
}

fn expand_tilde(path: &Path) -> PathBuf {
    match (BaseDirs::new(), path.strip_prefix("~")) {
        (Some(bd), Ok(stripped)) => bd.home_dir().join(stripped),
        _ => path.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_example_conf() {
        // test that the example configuration file parses without error
        let mut args = Args::default();
        args.flag_conf = Some(PathBuf::from("example.config.toml"));
        Rtd::from_args(args).unwrap();
    }

    #[test]
    fn example_conf_data_matches_generated_default_values() {
        let example = fs::read_to_string("example.config.toml").unwrap();
        let default = toml::ser::to_string(&Conf::default()).unwrap();
        assert!(default == example);
    }

    #[test]
    fn test_expand_tilde() {
        let homedir: PathBuf = BaseDirs::new()
            .unwrap()
            .home_dir()
            .to_owned();

        assert_eq!(
            expand_tilde(&PathBuf::from("/")),
            PathBuf::from("/")
        );
        assert_eq!(
            expand_tilde(&PathBuf::from("/abc/~def/ghi/")),
            PathBuf::from("/abc/~def/ghi/")
        );
        assert_eq!(
            expand_tilde(&PathBuf::from("~/")),
            PathBuf::from(format!("{}/", homedir.to_str().unwrap()))
        );
        assert_eq!(
            expand_tilde(&PathBuf::from("~/abc/def/ghi/")),
            PathBuf::from(format!("{}/abc/def/ghi/", homedir.to_str().unwrap()))
        );
    }
}
