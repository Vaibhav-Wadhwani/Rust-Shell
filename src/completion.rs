// completion.rs

use rustyline::completion::{Completer, Pair};
use rustyline::{Helper, Context};
use rustyline::hint::Hinter;
use rustyline::highlight::Highlighter;
use rustyline::validate::{Validator, ValidationContext, ValidationResult};
use rustyline::error::ReadlineError;
use std::cell::RefCell;

pub struct BuiltinCompleter {
    pub last_prefix: RefCell<String>,
    pub tab_count: RefCell<u8>,
    pub last_matches: RefCell<Vec<String>>,
}

impl BuiltinCompleter {
    pub fn new() -> Self {
        Self {
            last_prefix: RefCell::new(String::new()),
            tab_count: RefCell::new(0),
            last_matches: RefCell::new(vec![]),
        }
    }
}

impl Completer for BuiltinCompleter {
    type Candidate = Pair;
    fn complete(&self, line: &str, pos: usize, _ctx: &Context<'_>) -> Result<(usize, Vec<Pair>), ReadlineError> {
        let prefix = &line[..pos];
        let mut names = Vec::new();
        for b in ["echo", "exit", "type", "pwd", "cd", "history"] {
            if b.starts_with(prefix) {
                names.push(b.to_string());
            }
        }
        if let Ok(path_var) = std::env::var("PATH") {
            for dir in path_var.split(':') {
                let path = std::path::Path::new(dir);
                if let Ok(entries) = std::fs::read_dir(path) {
                    for entry in entries.flatten() {
                        let file_type = entry.file_type();
                        if let Ok(ft) = file_type {
                            if ft.is_file() || ft.is_symlink() {
                                let file_name = entry.file_name();
                                let file_name_str = match file_name.to_str() {
                                    Some(s) => s,
                                    None => continue,
                                };
                                if file_name_str.starts_with(prefix) {
                                    let meta = entry.metadata();
                                    if let Ok(m) = meta {
                                        #[cfg(unix)]
                                        let is_exec = m.permissions().mode() & 0o111 != 0;
                                        #[cfg(not(unix))]
                                        let is_exec = true;
                                        if is_exec {
                                            names.push(file_name_str.to_string());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        names.sort();
        names.dedup();
        let completions: Vec<Pair> = names.iter().map(|n| Pair {
            display: n.clone(),
            replacement: format!("{} ", n),
        }).collect();
        *self.last_matches.borrow_mut() = names.clone();
        if prefix == *self.last_prefix.borrow() {
            *self.tab_count.borrow_mut() += 1;
        } else {
            *self.tab_count.borrow_mut() = 1;
            *self.last_prefix.borrow_mut() = prefix.to_string();
        }
        Ok((0, completions))
    }
}

impl Hinter for BuiltinCompleter {
    type Hint = String;
    fn hint(&self, _line: &str, _pos: usize, _ctx: &Context<'_>) -> Option<String> {
        None
    }
}

impl Highlighter for BuiltinCompleter {}

impl Validator for BuiltinCompleter {
    fn validate(&self, _ctx: &mut ValidationContext) -> Result<ValidationResult, ReadlineError> {
        Ok(ValidationResult::Valid(None))
    }
}

impl Helper for BuiltinCompleter {} 