#![allow(dead_code)]

use std::fmt::Write;
use std::ops::DerefMut;
use std::sync::RwLock;

use log::error;

/// Returns a human readable String of a Duration
///
/// Example: `3h 12m 53s`
pub fn format_duration(d: &std::time::Duration) -> String {
    let mut s = String::new();
    let mut append_unit = |value, unit| {
        if value > 0 {
            let _ = write!(s, "{value}{unit}");
        }
    };

    let seconds = d.as_secs() % 60;
    let minutes = (d.as_secs() / 60) % 60;
    let hours = (d.as_secs() / 60) / 60;

    append_unit(hours, "h ");
    append_unit(minutes, "m ");
    append_unit(seconds, "s ");

    s.trim_end().to_string()
}

/// Returns a human readable String of milliseconds in the HH:MM:SS format.
pub fn ms_to_hms(duration: u32) -> String {
    let mut formated_time = String::new();

    let total_seconds = duration / 1000;
    let seconds = total_seconds % 60;
    let minutes = (total_seconds / 60) % 60;
    let hours = total_seconds / 3600;

    if hours > 0 {
        formated_time.push_str(&format!("{hours}:{minutes:02}:"));
    } else {
        formated_time.push_str(&format!("{minutes}:"));
    }
    formated_time.push_str(&format!("{seconds:02}"));

    formated_time
}

pub fn cache_path_for_url(url: String) -> std::path::PathBuf {
    let mut path = crate::config::cache_path("covers");
    path.push(url.split('/').last().unwrap());
    path
}

pub fn download(url: String, path: std::path::PathBuf) -> Result<(), std::io::Error> {
    let mut resp = reqwest::blocking::get(url)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

    std::fs::create_dir_all(path.parent().unwrap())?;
    let mut file = std::fs::File::create(path)?;

    std::io::copy(&mut resp, &mut file)?;
    Ok(())
}

macro_rules! named_lock {
    ( $self:ident.$field:ident ) => {
        (stringify!($field), &$self.$field)
    };
}
pub(crate) use named_lock;

pub fn try_acquire_write<T>(
    (name, lock): (&str, &RwLock<T>),
    use_resource: impl FnOnce(&mut T),
) -> bool {
    let locked_resource = lock.write();
    let successful_acquired = locked_resource.is_ok();

    match locked_resource {
        Ok(mut resource) => use_resource(resource.deref_mut()),
        Err(_) => error!("couldn't acquire write lock for {name}"),
    };

    successful_acquired
}

pub fn assign_if_new_value<T: Eq>(field: &mut T, new_value: T) -> bool {
    if new_value.ne(field) {
        *field = new_value;
        true
    } else {
        false
    }
}
