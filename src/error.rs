use std::error;

pub type LauncherResult<T> = Result<T, Box<dyn error::Error>>;
