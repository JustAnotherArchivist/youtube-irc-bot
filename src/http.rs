use htmlescape::decode_html;
use std::time::Duration;
use itertools::Itertools;
use regex::Regex;
use failure::Error;
use reqwest::Client;
use reqwest::header::{USER_AGENT, ACCEPT_LANGUAGE, CONTENT_TYPE, CONTENT_LENGTH};
use std::io::Read;
use image::{gif, jpeg, png, ImageDecoder};
use mime::{Mime, IMAGE, TEXT, HTML};
use humansize::{FileSize, file_size_opts as options};
use super::config::Rtd;

const DL_BYTES: u64 = 100 * 1024; // 100kB

pub fn resolve_url(url: &str, rtd: &Rtd) -> Result<String, Error> {
    eprintln!("RESOLVE {}", url);

    let client = Client::builder()
        .timeout(Duration::from_secs(10)) // per read/write op
        .build()?;

    let resp = client.get(url)
        .header(USER_AGENT, rtd.conf.params.user_agent.as_str())
        .header(ACCEPT_LANGUAGE, rtd.conf.params.accept_lang.as_str())
        .send()?
        .error_for_status()?;

    // get response headers
    let content_type = resp.headers().get(CONTENT_TYPE)
        .and_then(|typ| typ.to_str().ok())
        .and_then(|typ| typ.parse::<Mime>().ok());
    let len = resp.headers().get(CONTENT_LENGTH)
        .and_then(|len| len.to_str().ok())
        .and_then(|len| len.parse().ok())
        .unwrap_or(0);
    let size = len.file_size(options::CONVENTIONAL).unwrap_or_default();

    // print HTTP status and response headers for debugging
    if rtd.args.flag_debug {
        eprintln!("{}", resp.status());
        for (k, v) in resp.headers() {
            eprintln!("{}: {}", k, v.to_str().unwrap());
        }
    }

    // calculate download size based on the response's MIME type
    let bytes = content_type.clone()
        .and_then(|ct| {
            match (ct.type_(), ct.subtype()) {
                (IMAGE, _) => Some(10 * 1024 * 1024), // 10MB
                _ => None
            }})
        .unwrap_or(DL_BYTES);

    // download body
    let mut body = Vec::new();
    resp.take(bytes).read_to_end(&mut body)?;
    let contents = String::from_utf8_lossy(&body);

    // get title or metadata
    let title = match content_type {
        None => parse_title(&contents),
        Some(mime) => {
            match (mime.type_(), mime.subtype()) {
                (TEXT, HTML) => parse_title(&contents),
                (IMAGE, _) => parse_title(&contents)
                    .or_else(|| get_image_metadata(&rtd, &body))
                    .or_else(|| get_mime(&rtd, &mime, &size)),
                _ => parse_title(&contents)
                    .or_else(|| get_mime(&rtd, &mime, &size)),
            }
        },
    }.ok_or_else(|| format_err!("failed to parse title"))?;

    eprintln!("SUCCESS \"{}\"", title);
    Ok(title)
}

pub fn get_youtube_user(url: &str) -> Result<Option<String>, Error> {
    eprintln!("RESOLVE {}", url);

    let client = Client::builder()
        .timeout(Duration::from_secs(10)) // per read/write op
        .build()?;

    let resp = client.get(url)
        .header(USER_AGENT, "curl/7.37.0") // Get the old no-Polymer pages
        .header(ACCEPT_LANGUAGE, "en")
        .send()?
        .error_for_status()?;

    // get response headers
    let content_type = resp.headers().get(CONTENT_TYPE)
        .and_then(|typ| typ.to_str().ok())
        .and_then(|typ| typ.parse::<Mime>().ok());

    // download body
    let mut body = Vec::new();
    resp.take(10 * 1024 * 1024).read_to_end(&mut body)?;
    let contents = String::from_utf8_lossy(&body);

    // get title or metadata
    let user = match content_type {
        None => parse_user(&contents),
        Some(mime) => {
            match (mime.type_(), mime.subtype()) {
                (TEXT, HTML) => parse_user(&contents),
                _ => None
            }
        }
    };
    eprintln!("URL {} mentions user {:?}", url, user);
    Ok(user.clone())
}

fn get_mime(rtd: &Rtd, mime: &Mime, size: &str) -> Option<String> {
    if rtd.conf.features.report_mime {
        Some(format!("{} {}", mime, size.replace(" ", "")))
    } else {
        None
    }
}

fn get_image_metadata(rtd: &Rtd, body: &[u8]) -> Option<String> {
    if !rtd.conf.features.report_metadata {
        None
    } else if let Ok((w, h)) = jpeg::JPEGDecoder::new(body).dimensions() {
        Some(format!("image/jpeg {}×{}", w, h))
    } else if let Ok((w, h)) = png::PNGDecoder::new(body).dimensions() {
        Some(format!("image/png {}×{}", w, h))
    } else if let Ok((w, h)) = gif::Decoder::new(body).dimensions() {
        Some(format!("image/gif {}×{}", w, h))
    } else {
        None
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::path::Path;

    #[test]
    fn resolve_urls() {
        let rtd: Rtd = Rtd::default();
        resolve_url("https://youtube.com",  &rtd).unwrap();
        resolve_url("https://google.co.uk", &rtd).unwrap();
    }

    #[test]
    fn parse_titles() {
        assert_eq!(None, parse_title(""));
        assert_eq!(None, parse_title("    "));
        assert_eq!(None, parse_title("<title></title>"));
        assert_eq!(None, parse_title("<title>    </title>"));
        assert_eq!(None, parse_title("<TITLE>    </TITLE>"));
        assert_eq!(
            None,
            parse_title("floofynips, not a real webpage")
        );
        assert_eq!(
            Some(String::from("title caps")),
            parse_title("<TITLE>title caps</TITLE>")
        );
        assert_eq!(
            Some(String::from("title mixed caps")),
            parse_title("<TiTLe>title mixed caps</tItLE>")
        );
        assert_eq!(
            Some(String::from("cheese is nice")),
            parse_title("<title>cheese is nice</title>")
        );
        assert_eq!(
            Some(String::from("squanch")),
            parse_title("<title>     squanch</title>")
        );
        assert_eq!(
            Some(String::from("squanch")),
            parse_title("<title>squanch     </title>")
        );
        assert_eq!(
            Some(String::from("squanch")),
            parse_title("<title>\nsquanch</title>")
        );
        assert_eq!(
            Some(String::from("squanch")),
            parse_title("<title>\n  \n  squanch</title>")
        );
        assert_eq!(
            Some(String::from("we like the moon")),
            parse_title("<title>\n  \n  we like the moon</title>")
        );
        assert_eq!(
            Some(String::from("&hello123&<>''~")),
            parse_title("<title>&amp;hello123&amp;&lt;&gt;''~</title>")
        );
        assert_eq!(
            Some(String::from("CVE - CVE-2018-11235")),
            parse_title("<title>CVE -\nCVE-2018-11235\n</title>")
        );
        assert_eq!(
            Some(String::from("added properties")),
            parse_title("<title id=\"pageTitle\">added properties</title>")
        );
        assert_eq!(
            Some(String::from("\u{2665}")),
            parse_title("<title>\u{2665}</title>")
        );
    }

    #[test]
    fn get_metadata_from_local_images() {
        for test in vec!(
            ("./test/img/test.png", "image/png 800×400"),
            ("./test/img/test.jpg", "image/jpeg 400×200"),
            ("./test/img/test.gif", "image/gif 1920×1080")
        ) {
            get_local_image_metadata(test.0, test.1);
        }
    }

    fn get_local_image_metadata(file: impl AsRef<Path>, result: &str) {
        let mut rtd: Rtd = Rtd::default();

        let mut body = Vec::new();
        let f = File::open(file).unwrap();
        f.take(100 * 1024).read_to_end(&mut body).unwrap();

        rtd.conf.features.report_metadata = true;
        assert_eq!(
            Some(String::from(result)),
            get_image_metadata(&rtd, &body)
        );

        rtd.conf.features.report_metadata = false;
        assert_eq!(
            None,
            get_image_metadata(&rtd, &body)
        );
    }

    #[test]
    fn parse_images() {
        let mut rtd: Rtd = Rtd::default();
        rtd.conf.features.report_metadata = true;
        match resolve_url("https://rynx.org/sebk/_/DSC_5503.jpg", &rtd) {
            Ok(metadata) => assert_eq!(metadata, "image/jpeg 1000×663"),
            Err(_) => assert!(false),
        }
        match resolve_url(
            "https://assets-cdn.github.com/images/modules/logos_page/GitHub-Mark.png",
            &rtd,
        ) {
            Ok(metadata) => assert_eq!(metadata, "image/png 560×560"),
            Err(_) => assert!(false),
        }
        match resolve_url(
            "https://upload.wikimedia.org/wikipedia/commons/2/2b/Seven_segment_display-animated.gif",
            &rtd,
        ) {
            Ok(metadata) => assert_eq!(metadata, "image/gif 600×752"),
            Err(_) => assert!(false),
        }
    }
}
