use std::error;
use std::fmt;

#[derive(Debug, Clone)]
pub struct MyError {
    message: String
}

impl MyError {
    pub fn new(message: String) -> MyError {
        MyError { message }
    }
}

impl fmt::Display for MyError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl error::Error for MyError {
    fn description(&self) -> &str {
        &self.message
    }

    fn cause(&self) -> Option<&dyn error::Error> {
        // Generic error, underlying cause isn't tracked.
        None
    }
}
