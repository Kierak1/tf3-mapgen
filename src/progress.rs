pub trait Progress: Send + Sync {
    fn info(&self, message: &str);
    fn warn(&self, message: &str);
}

pub struct StderrProgress;

impl Progress for StderrProgress {
    fn info(&self, message: &str) {
        eprintln!("{message}");
    }

    fn warn(&self, message: &str) {
        eprintln!("warning: {message}");
    }
}

pub struct SilentProgress;

impl Progress for SilentProgress {
    fn info(&self, _message: &str) {}

    fn warn(&self, _message: &str) {}
}
