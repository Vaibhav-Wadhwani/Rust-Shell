// util.rs

use std::io::Write;

pub fn writeln_ignore_broken_pipe<W: std::io::Write, S: AsRef<str>>(mut w: W, s: S) -> std::io::Result<()> {
    match writeln!(w, "{}", s.as_ref()) {
        Err(ref e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        other => other,
    }
} 