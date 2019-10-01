use std::time::{SystemTime, UNIX_EPOCH};

fn shorthand_duration(start_time: u64) -> String {
    let start = SystemTime::now();
    let since_the_epoch = start.duration_since(UNIX_EPOCH)
        .expect("Time went backwards");
    let time_now = since_the_epoch.as_secs();
    let duration = time_now - start_time;
    duration_to_dhm(duration)
}

/// seconds -> e.g. 2d4h1m
fn duration_to_dhm(secs: u64) -> String {
    match secs {
        secs if secs < 3600  => {
            let minutes = secs / 60;
            format!("{}m", minutes)
        },
        secs if secs < 86400 => {
            let hours = secs / 3600;
            let minutes = (secs - (hours * 3600)) / 60;
            format!("{}h{}m", hours, minutes)
        },
        secs => {
            let days  = secs / 86400;
            let hours = (secs - (days * 86400)) / 3600;
            let minutes = (secs - (days * 86400) - (hours * 3600)) / 60;
            format!("{}d{}h{}m", days, hours, minutes)
        },
    }
}
