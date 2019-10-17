// Copied from https://github.com/qryxip/snowchains/blob/dcd76c1dbb87eea239ba17f28b44ee11fdd3fd80/src/macros.rs
macro_rules! lazy_regex {
    ($expr:expr) => {{
        static REGEX: ::once_cell::sync::Lazy<::regex::Regex> =
            ::once_cell::sync::Lazy::new(|| ::regex::Regex::new($expr).unwrap());
        &REGEX
    }};
    ($expr:expr,) => {
        lazy_regex!($expr)
    };
}
