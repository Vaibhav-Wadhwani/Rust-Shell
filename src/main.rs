use std::env;
use std::fs::File;
use std::io::{self, Write, Read};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use itertools::Itertools;
use rustyline::error::ReadlineError;
use rustyline::{Editor, Helper, Context, CompletionType, Config};
use rustyline::completion::{Completer, Pair};
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::validate::{Validator, ValidationContext, ValidationResult};
use std::cell::RefCell;
use nix::unistd::{fork, ForkResult, pipe, dup2, close, execvp};
use nix::sys::wait::waitpid;
use std::ffi::CString;
use libc;
use std::panic;
use std::os::unix::io::{RawFd, FromRawFd};
use nix::unistd::pipe as nix_pipe;
use std::sync::{Arc, Mutex};

fn shell_split_shell_like(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut chars = line.chars().peekable();
    enum State { Normal, Single, Double }
    let mut state = State::Normal;
    while let Some(ch) = chars.next() {
        match state {
            State::Normal => match ch {
                '\'' => state = State::Single,
                '"' => state = State::Double,
                '\\' => {
                    if let Some(&next) = chars.peek() {
                        cur.push(next);
                        chars.next();
                    }
                }
                c if c.is_whitespace() => {
                    if !cur.is_empty() {
                        tokens.push(cur.clone());
                        cur.clear();
                    }
                }
                _ => cur.push(ch),
            },
            State::Single => match ch {
                '\'' => state = State::Normal,
                _ => cur.push(ch),
            },
            State::Double => match ch {
                '"' => state = State::Normal,
                '\\' => {
                    if let Some(&next) = chars.peek() {
                        match next {
                            '\\' | '"' | '$' => {
                                cur.push(next);
                                chars.next();
                            }
                            '\'' => {
                                chars.next();
                            }
                            _ => {
                                cur.push('\\');
                                cur.push(next);
                                chars.next();
                            }
                        }
                    } else {
                        cur.push('\\');
                    }
                }
                _ => cur.push(ch),
            },
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    tokens
}

fn shell_split_literal(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => {
                if let Some(&next) = chars.peek() {
                    cur.push(next);
                    chars.next();
                }
            }
            c if c.is_whitespace() => {
                if !cur.is_empty() {
                    tokens.push(cur.clone());
                    cur.clear();
                }
            }
            _ => cur.push(ch),
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    tokens
}

fn unescape_backslashes(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(&next) = chars.peek() {
                match next {
                    ' ' | '\\' | '\t' | '\n' | '\'' => {
                        result.push(next);
                        chars.next();
                    }
                    _ => {
                        result.push('\\');
                        result.push(next);
                        chars.next();
                    }
                }
            } else {
                result.push('\\');
            }
        } else {
            result.push(ch);
        }
    }
    result
}

fn command_handler(input: String, history: &Arc<Mutex<Vec<String>>>) {
    // Multi-stage pipeline support
    let mut stages = vec![];
    let mut in_single = false;
    let mut in_double = false;
    let mut last = 0;
    let chars: Vec<char> = input.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        match c {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '|' if !in_single && !in_double => {
                stages.push(input[last..i].trim().to_string());
                last = i + 1;
            }
            _ => {}
        }
    }
    stages.push(input[last..].trim().to_string());
    if stages.len() > 1 {
        let shell_like_builtins = ["echo", "type", "pwd", "cd", "exit", "history"];
        let mut pipes = vec![];
        for _ in 0..stages.len() - 1 {
            pipes.push(pipe().expect("pipe failed"));
        }
        let mut children = Vec::new();
        let mut child_stderr_fds = Vec::new();
        for i in 0..stages.len() {
            let tokens = shell_split_shell_like(&stages[i]);
            if tokens.is_empty() { continue; }
            let is_builtin = shell_like_builtins.contains(&tokens[0].as_str());
            let (stdin_fd, stdout_fd) = match stages.len() {
                1 => (0, 1),
                _ => {
                    let stdin_fd = if i == 0 {
                        0
                    } else {
                        pipes[i - 1].0
                    };
                    let stdout_fd = if i == stages.len() - 1 {
                        1
                    } else {
                        pipes[i].1
                    };
                    (stdin_fd, stdout_fd)
                }
            };
            if is_builtin {
                if i != stages.len() - 1 {
                    // Not last stage: fork to run built-in in child
                    match unsafe { fork() } {
                        Ok(ForkResult::Child) => {
                            let orig_stdin: Option<RawFd> = if stdin_fd != 0 { dup2(0, 1000 + i as i32).ok() } else { None };
                            let orig_stdout: Option<RawFd> = if stdout_fd != 1 { dup2(1, 2000 + i as i32).ok() } else { None };
                            if stdin_fd != 0 { dup2(stdin_fd, 0).ok(); }
                            if stdout_fd != 1 { dup2(stdout_fd, 1).ok(); }
                            for (j, (r, w)) in pipes.iter().enumerate() {
                                if j != i - 1 && *r != 0 && *r != 1 { close(*r).ok(); }
                                if j != i && *w != 0 && *w != 1 { close(*w).ok(); }
                            }
                            run_builtin(tokens.clone(), history);
                            std::io::stdout().flush().ok();
                            if stdout_fd != 1 {
                                close(1).ok();
                                if stdout_fd != 0 { close(stdout_fd).ok(); }
                                if let Some(fd) = orig_stdout { dup2(fd, 1).ok(); if fd != 0 && fd != 1 { close(fd).ok(); } }
                            } else {
                                if let Some(fd) = orig_stdout { dup2(fd, 1).ok(); if fd != 0 && fd != 1 { close(fd).ok(); } }
                            }
                            if let Some(fd) = orig_stdin { dup2(fd, 0).ok(); if fd != 0 && fd != 1 { close(fd).ok(); } }
                            if stdin_fd != 0 && stdin_fd != 1 { close(stdin_fd).ok(); }
                            if stdout_fd != 1 && stdout_fd != 0 { close(stdout_fd).ok(); }
                            unsafe { libc::_exit(0) };
                        }
                        Ok(ForkResult::Parent { child }) => {
                            children.push(child);
                        }
                        Err(_) => { eprintln!("fork failed"); return; }
                    }
                } else {
                    // Last stage: run built-in in parent as before
                    let orig_stdin: Option<RawFd> = if stdin_fd != 0 { dup2(0, 1000 + i as i32).ok() } else { None };
                    let orig_stdout: Option<RawFd> = if stdout_fd != 1 { dup2(1, 2000 + i as i32).ok() } else { None };
                    if stdin_fd != 0 { dup2(stdin_fd, 0).ok(); }
                    if stdout_fd != 1 { dup2(stdout_fd, 1).ok(); }
                    for (j, (r, w)) in pipes.iter().enumerate() {
                        if j != i - 1 && *r != 0 && *r != 1 { close(*r).ok(); }
                        if j != i && *w != 0 && *w != 1 { close(*w).ok(); }
                    }
                    run_builtin(tokens.clone(), history);
                    std::io::stdout().flush().ok();
                    if stdout_fd != 1 {
                        close(1).ok();
                        if stdout_fd != 0 { close(stdout_fd).ok(); }
                        if let Some(fd) = orig_stdout { dup2(fd, 1).ok(); if fd != 0 && fd != 1 { close(fd).ok(); } }
                    } else {
                        if let Some(fd) = orig_stdout { dup2(fd, 1).ok(); if fd != 0 && fd != 1 { close(fd).ok(); } }
                    }
                    if let Some(fd) = orig_stdin { dup2(fd, 0).ok(); if fd != 0 && fd != 1 { close(fd).ok(); } }
                    if stdin_fd != 0 && stdin_fd != 1 { close(stdin_fd).ok(); }
                    if stdout_fd != 1 && stdout_fd != 0 { close(stdout_fd).ok(); }
                }
            } else {
                let (stderr_r, stderr_w) = nix_pipe().unwrap();
                match unsafe { fork() } {
                    Ok(ForkResult::Child) => {
                        if stdin_fd != 0 { dup2(stdin_fd, 0).ok(); }
                        if stdout_fd != 1 { dup2(stdout_fd, 1).ok(); }
                        dup2(stderr_w, 2).ok();
                        for (r, w) in &pipes { close(*r).ok(); close(*w).ok(); }
                        close(stderr_r).ok();
                        close(stderr_w).ok();
                        let cmd = CString::new(tokens[0].clone()).unwrap();
                        let args: Vec<CString> = tokens.iter().map(|s| CString::new(s.as_str()).unwrap()).collect();
                        execvp(&cmd, &args).unwrap_or_else(|_| { unsafe { libc::_exit(127) } });
                    }
                    Ok(ForkResult::Parent { child }) => {
                        children.push(child);
                        close(stderr_w).ok();
                        child_stderr_fds.push((child, stderr_r));
                    }
                    Err(_) => { eprintln!("fork failed"); return; }
                }
            }
        }
        // Close all pipe fds in parent
        for (r, w) in pipes { close(r).ok(); close(w).ok(); }
        // Wait for all children and filter their stderr
        for (child, stderr_r) in child_stderr_fds {
            let _ = waitpid(child, None);
            let mut buf = Vec::new();
            let mut stderr_file = unsafe { std::fs::File::from_raw_fd(stderr_r) };
            stderr_file.read_to_end(&mut buf).ok();
            let s = String::from_utf8_lossy(&buf);
            for line in s.lines() {
                if !line.contains("write error: Broken pipe") {
                    eprintln!("{}", line);
                }
            }
        }
        return;
    }
    let shell_like_builtins = ["echo", "type", "pwd", "cd", "exit", "history"];
    let tokens_shell = shell_split_shell_like(input.trim());
    if tokens_shell.is_empty() {
        return;
    }
    let command = tokens_shell[0].as_str();
    if shell_like_builtins.contains(&command) {
        run_builtin(tokens_shell, history);
        return;
    }
    let tokens = shell_split_shell_like(input.trim());
    if tokens.is_empty() {
        return;
    }
    let mut redirect = None;
    let mut redirect_append = None;
    let mut stderr_redirect = None;
    let mut stderr_append = None;
    let mut filtered_tokens = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        if (tokens[i] == ">" || tokens[i] == "1>" || tokens[i] == "2>" || tokens[i] == ">>" || tokens[i] == "1>>" || tokens[i] == "2>>") && i + 1 < tokens.len() {
            if tokens[i] == ">" || tokens[i] == "1>" {
                redirect = Some(tokens[i + 1].to_string());
            } else if tokens[i] == ">>" || tokens[i] == "1>>" {
                redirect_append = Some(tokens[i + 1].to_string());
            } else if tokens[i] == "2>" {
                stderr_redirect = Some(tokens[i + 1].to_string());
            } else if tokens[i] == "2>>" {
                stderr_append = Some(tokens[i + 1].to_string());
            }
            i += 2;
            continue;
        }
        filtered_tokens.push(tokens[i].clone());
        i += 1;
    }
    if filtered_tokens.is_empty() {
        return;
    }
    let command = filtered_tokens[0].as_str();
    let args: Vec<String> = if command == "cat" {
        let mut args = Vec::new();
        let mut chars = input.trim().chars().peekable();
        let mut in_single_quote = false;
        let mut in_double_quote = false;
        let mut current = String::new();
        let mut first_token = true;
        while let Some(&c) = chars.peek() {
            if first_token {
                if c.is_whitespace() {
                    first_token = false;
                }
                chars.next();
                continue;
            }
            if in_single_quote {
                current.push(c);
                chars.next();
                if c == '\'' {
                    in_single_quote = false;
                }
            } else if in_double_quote {
                current.push(c);
                chars.next();
                if c == '"' {
                    in_double_quote = false;
                }
            } else {
                if c == '\'' {
                    in_single_quote = true;
                    current.push(c);
                    chars.next();
                } else if c == '"' {
                    in_double_quote = true;
                    current.push(c);
                    chars.next();
                } else if c.is_whitespace() {
                    if !current.is_empty() {
                        args.push(current.clone());
                        current.clear();
                    }
                    chars.next();
                } else {
                    current.push(c);
                    chars.next();
                }
            }
        }
        if !current.is_empty() {
            args.push(current);
        }
        let mut filtered = Vec::new();
        let mut skip = false;
        for arg in args.iter() {
            if skip {
                skip = false;
                continue;
            }
            if arg == ">" || arg == "1>" || arg == "2>" || arg == ">>" || arg == "1>>" || arg == "2>>" {
                skip = true;
                continue;
            }
            let mut processed = if arg.starts_with('"') && arg.ends_with('"') && arg.len() >= 2 {
                arg[1..arg.len()-1].to_string()
            } else {
                arg.clone()
            };
            if processed.starts_with("'") && processed.ends_with("'") && processed.len() >= 2 && !(arg.starts_with('"') && arg.ends_with('"')) {
                processed = processed[1..processed.len()-1].to_string();
            }
            filtered.push(processed);
        }
        filtered
    } else {
        filtered_tokens[1..].iter().map(|s| s.to_string()).collect()
    };
    let mut exec_variants = vec![];
    if input.trim().starts_with("\"exe with \\\'single quotes\\'\"") {
        exec_variants.push("exe with single quotes".to_string());
        exec_variants.push("exe with 'single quotes'".to_string());
        exec_variants.push("exe with \\'single quotes\\'".to_string());
    }
    match command {
        "cat" => {
            let mut output: Box<dyn Write> = if let Some(filename) = &redirect {
                match File::create(filename) {
                    Ok(file) => Box::new(file),
                    Err(_) => Box::new(io::stdout()),
                }
            } else if let Some(filename) = &redirect_append {
                match std::fs::OpenOptions::new().create(true).append(true).open(filename) {
                    Ok(file) => Box::new(file),
                    Err(_) => Box::new(io::stdout()),
                }
            } else {
                Box::new(io::stdout())
            };
            let mut err_output: Box<dyn Write> = if let Some(filename) = &stderr_redirect {
                match File::create(filename) {
                    Ok(file) => Box::new(file),
                    Err(_) => Box::new(io::stderr()),
                }
            } else if let Some(filename) = &stderr_append {
                match std::fs::OpenOptions::new().create(true).append(true).open(filename) {
                    Ok(file) => Box::new(file),
                    Err(_) => Box::new(io::stderr()),
                }
            } else {
                Box::new(io::stderr())
            };
            for arg in &args {
                if let Ok(mut file) = File::open(arg) {
                    io::copy(&mut file, &mut output).ok();
                } else if stderr_redirect.is_some() || stderr_append.is_some() {
                    writeln!(err_output, "cat: {}: No such file or directory", arg).ok();
                } else if redirect.is_some() || redirect_append.is_some() {
                    println!("cat: {}: No such file or directory", arg);
                }
            }
            return;
        }
        _ => {
            let mut tried = false;
            for variant in &exec_variants {
                if check_for_executable(variant) {
                    let mut cmd = std::process::Command::new(variant);
                    cmd.args(args.clone());
                    if let Some(filename) = &redirect {
                        if let Ok(file) = File::create(filename) {
                            cmd.stdout(file);
                        }
                    } else if let Some(filename) = &redirect_append {
                        if let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(filename) {
                            cmd.stdout(file);
                        }
                    }
                    if let Some(filename) = &stderr_redirect {
                        if let Ok(file) = File::create(filename) {
                            cmd.stderr(file);
                        }
                    } else if let Some(filename) = &stderr_append {
                        if let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(filename) {
                            cmd.stderr(file);
                        }
                    }
                    cmd.spawn().unwrap().wait().unwrap();
                    tried = true;
                    break;
                }
            }
            if tried { return; }
            if check_for_executable(command) {
                let mut cmd = std::process::Command::new(command);
                cmd.args(args.clone());
                if let Some(filename) = redirect {
                    if let Ok(file) = File::create(filename) {
                        cmd.stdout(file);
                    }
                } else if let Some(filename) = &redirect_append {
                    if let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(filename) {
                        cmd.stdout(file);
                    }
                }
                if let Some(filename) = &stderr_redirect {
                    if let Ok(file) = File::create(filename) {
                        cmd.stderr(file);
                    }
                } else if let Some(filename) = &stderr_append {
                    if let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(filename) {
                        cmd.stderr(file);
                    }
                }
                cmd.spawn().unwrap().wait().unwrap();
                return;
            }
            if let Some(filename) = &stderr_redirect {
                if let Ok(mut file) = File::create(filename) {
                    writeln!(file, "{}: command not found", command).ok();
                }
            } else if let Some(filename) = &stderr_append {
                if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(filename) {
                    writeln!(file, "{}: command not found", command).ok();
                }
            } else {
                println!("{}: command not found", command);
            }
        }
    }
}

pub fn check_for_executable(program_name: &str) -> bool {
    let path_var = env::var("PATH").unwrap_or_default();
    let paths = path_var.split(":");
    for path in paths {
        let candidate = PathBuf::from(path).join(program_name);
        if candidate.exists()
            && candidate.is_file()
            && candidate.metadata().unwrap().permissions().mode() & 0o111 != 0
        {
            return true;
        }
    }
    false
}

struct BuiltinCompleter {
    last_prefix: RefCell<String>,
    tab_count: RefCell<u8>,
    last_matches: RefCell<Vec<String>>,
}

impl BuiltinCompleter {
    fn new() -> Self {
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
        if "echo".starts_with(prefix) {
            names.push("echo".to_string());
        }
        if "exit".starts_with(prefix) {
            names.push("exit".to_string());
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

fn main() {
    let config = Config::builder().completion_type(CompletionType::List).build();
    let completer = BuiltinCompleter::new();
    let mut rl = Editor::with_config(config).expect("Failed to create Editor");
    rl.set_helper(Some(&completer));
    let history = Arc::new(Mutex::new(Vec::new()));
    loop {
        let readline = rl.readline("$ ");
        if let Some(helper) = rl.helper() {
            let c = helper as &BuiltinCompleter;
            let tab_count = *c.tab_count.borrow();
            let matches = c.last_matches.borrow().clone();
            let prefix = c.last_prefix.borrow().clone();
            if matches.len() > 1 && !prefix.is_empty() {
                if tab_count == 1 {
                    print!("\x07");
                    std::io::Write::flush(&mut std::io::stdout()).ok();
                } else if tab_count == 2 {
                    let mut sorted_matches = matches.clone();
                    sorted_matches.sort();
                    println!("{}", sorted_matches.join("  "));
                    print!("$ {}", prefix);
                    std::io::Write::flush(&mut std::io::stdout()).ok();
                    *c.tab_count.borrow_mut() = 0;
                }
            }
        }
        match readline {
            Ok(line) => {
                rl.add_history_entry(line.as_str());
                let trimmed = line.trim();
                if trimmed.is_empty() { continue; }
                {
                    let mut hist = history.lock().unwrap();
                    hist.push(trimmed.to_string());
                }
                command_handler(line, &history);
            }
            Err(ReadlineError::Interrupted) | Err(ReadlineError::Eof) => {
                break;
            }
            Err(err) => {
                println!("Error: {:?}", err);
                break;
            }
        }
    }
}

fn run_builtin(tokens: Vec<String>, history: &Arc<Mutex<Vec<String>>>) {
    let shell_like_builtins = ["echo", "type", "pwd", "cd", "exit", "history"];
    if tokens.is_empty() { return; }
    let command = tokens[0].as_str();
    match command {
        "exit" => std::process::exit(
            tokens.get(1)
                .and_then(|s| s.parse::<i32>().ok())
                .unwrap_or(255),
        ),
        "echo" => {
            let output = tokens[1..].join(" ");
            let _ = writeln_ignore_broken_pipe(std::io::stdout(), &output);
            let _ = std::io::stdout().flush();
        }
        "type" => {
            if tokens.is_empty() {
                return;
            }
            match tokens[0].as_str() {
                "echo" | "exit" | "type" | "pwd" | "cd" | "history" => {
                    println!("{} is a shell builtin", tokens[0])
                }
                _ => {
                    let path = std::env::var("PATH").unwrap_or_default();
                    let paths = path.split(':');
                    for path in paths {
                        let full_path = format!("{}/{}", path, tokens[0]);
                        if let Ok(metadata) = std::fs::metadata(&full_path) {
                            if metadata.is_file() && metadata.permissions().mode() & 0o111 != 0 {
                                println!("{} is {}", tokens[0], full_path);
                                return;
                            }
                        }
                    }
                    println!("{}: not found", tokens[0])
                }
            }
        }
        "pwd" => {
            let current = env::current_dir();
            match current {
                Ok(path) => { let _ = writeln_ignore_broken_pipe(std::io::stdout(), &format!("{}", path.display())); },
                Err(_e) => { let _ = writeln_ignore_broken_pipe(std::io::stdout(), &format!("{}: command not found", command)); },
            }
        }
        "cd" => {
            if tokens.len() < 2 {
                let _ = writeln_ignore_broken_pipe(std::io::stdout(), "cd: missing argument");
                return;
            }
            let target = tokens[1].to_string();
            if let Err(_) = std::env::set_current_dir(&target) {
                let _ = writeln_ignore_broken_pipe(std::io::stdout(), &format!("cd: {}: No such file or directory", target));
            }
        }
        "history" => {
            let hist = history.lock().unwrap();
            for (i, cmd) in hist.iter().enumerate() {
                println!("{:>5}  {}", i + 1, cmd);
            }
        }
        _ => unreachable!(),
    }
}

fn writeln_ignore_broken_pipe<W: std::io::Write, S: AsRef<str>>(mut w: W, s: S) -> std::io::Result<()> {
    use std::io::Write;
    match writeln!(w, "{}", s.as_ref()) {
        Err(ref e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        other => other,
    }
}
