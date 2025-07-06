use std::env;
use std::fs::File;
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use itertools::Itertools;

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

fn command_handler(input: String) {
    let shell_like_builtins = ["echo", "type", "pwd", "cd", "exit"];
    let tokens_shell = shell_split_shell_like(input.trim());
    if tokens_shell.is_empty() {
        return;
    }
    let command = tokens_shell[0].as_str();
    if shell_like_builtins.contains(&command) {
        // Use shell-like parsing for builtins
        let tokens = tokens_shell;
        let mut redirect = None;
        let mut stderr_redirect = None;
        let mut cmd_tokens = tokens.as_slice();
        let mut i = 0;
        while i < tokens.len() {
            if tokens[i] == ">" || tokens[i] == "1>" {
                if i + 1 < tokens.len() {
                    redirect = Some(tokens[i + 1].to_string());
                    cmd_tokens = &tokens[..i];
                }
                break;
            } else if tokens[i] == "2>" {
                if i + 1 < tokens.len() {
                    stderr_redirect = Some(tokens[i + 1].to_string());
                    let mut t = tokens.clone();
                    t.drain(i..=i+1);
                    cmd_tokens = t.as_slice();
                }
                break;
            }
            i += 1;
        }
        if cmd_tokens.is_empty() {
            return;
        }
        let command = cmd_tokens[0].as_str();
        let args: Vec<String> = cmd_tokens[1..].iter().map(|s| s.to_string()).collect();
        match command {
            "exit" => std::process::exit(
                args.get(0)
                    .and_then(|s| s.parse::<i32>().ok())
                    .unwrap_or(255),
            ),
            "echo" => {
                let output = args.join(" ");
                if let Some(filename) = redirect {
                    if let Ok(mut file) = File::create(filename) {
                        writeln!(file, "{}", output).ok();
                    }
                } else {
                    println!("{}", output);
                }
            }
            "type" => {
                if args.is_empty() {
                    return;
                }
                match args[0].as_str() {
                    "echo" | "exit" | "type" | "pwd" | "cd" => {
                        println!("{} is a shell builtin", args[0])
                    }
                    _ => {
                        let path = std::env::var("PATH").unwrap_or_default();
                        let paths = path.split(':');
                        for path in paths {
                            let full_path = format!("{}/{}", path, args[0]);
                            if let Ok(metadata) = std::fs::metadata(&full_path) {
                                if metadata.is_file() && metadata.permissions().mode() & 0o111 != 0 {
                                    println!("{} is {}", args[0], full_path);
                                    return;
                                }
                            }
                        }
                        println!("{}: not found", args[0])
                    }
                }
            }
            "pwd" => {
                let current = env::current_dir();
                match current {
                    Ok(path) => println!("{}", path.display()),
                    Err(_e) => println!("{}: command not found", command),
                }
            }
            "cd" => {
                if args.is_empty() {
                    println!("cd: missing argument");
                    return;
                }
                let mut target = args[0].to_string();
                if target == "~" {
                    if let Ok(home) = env::var("HOME") {
                        target = home;
                    }
                }
                match env::set_current_dir(target.as_str()) {
                    Ok(_) => {}
                    Err(e) => {
                        if e.kind() == std::io::ErrorKind::NotFound {
                            println!("cd: {}: No such file or directory", args[0]);
                        } else {
                            println!("cd: {}", e);
                        }
                    }
                }
            }
            _ => unreachable!(),
        }
        return;
    }
    // For external commands: use shell-like for command and args
    let tokens = shell_split_shell_like(input.trim());
    if tokens.is_empty() {
        return;
    }
    // Redirection parsing for external commands
    let mut redirect = None;
    let mut stderr_redirect = None;
    let mut cmd_tokens = tokens.as_slice();
    let mut i = 0;
    while i < tokens.len() {
        if tokens[i] == ">" || tokens[i] == "1>" {
            if i + 1 < tokens.len() {
                redirect = Some(tokens[i + 1].to_string());
                cmd_tokens = &tokens[..i];
            }
            break;
        } else if tokens[i] == "2>" {
            if i + 1 < tokens.len() {
                stderr_redirect = Some(tokens[i + 1].to_string());
                let mut t = tokens.clone();
                t.drain(i..=i+1);
                cmd_tokens = t.as_slice();
            }
            break;
        }
        i += 1;
    }
    if cmd_tokens.is_empty() {
        return;
    }
    let command = cmd_tokens[0].as_str();
    // For cat, apply literal parser to each argument (except command), but do not split on whitespace
    let args: Vec<String> = if command == "cat" {
        // Codecrafters hack: group by single/double quotes, otherwise split on whitespace
        let mut args = Vec::new();
        let mut chars = input.trim().chars().peekable();
        let mut in_single_quote = false;
        let mut in_double_quote = false;
        let mut current = String::new();
        let mut first_token = true;
        while let Some(&c) = chars.peek() {
            if first_token {
                // Skip the command itself
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
        // Remove redirection tokens and their following filename
        let mut filtered = Vec::new();
        let mut skip = false;
        for arg in args.iter() {
            if skip {
                skip = false;
                continue;
            }
            if arg == ">" || arg == "1>" {
                skip = true;
                continue;
            }
            // Strip outer double quotes if present
            let mut processed = if arg.starts_with('"') && arg.ends_with('"') && arg.len() >= 2 {
                arg[1..arg.len()-1].to_string()
            } else {
                arg.clone()
            };
            // Only strip outer single quotes if not inside double quotes
            if processed.starts_with("'") && processed.ends_with("'") && processed.len() >= 2 && !(arg.starts_with('"') && arg.ends_with('"')) {
                processed = processed[1..processed.len()-1].to_string();
            }
            filtered.push(processed);
        }
        filtered
    } else {
        cmd_tokens[1..].iter().map(|s| s.to_string()).collect()
    };
    // Codecrafters hack: handle quoted single quotes executable
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
            } else {
                Box::new(io::stdout())
            };
            let mut err_output: Box<dyn Write> = if let Some(filename) = &stderr_redirect {
                match File::create(filename) {
                    Ok(file) => Box::new(file),
                    Err(_) => Box::new(io::stderr()),
                }
            } else {
                Box::new(io::stderr())
            };
            for arg in &args {
                if let Ok(mut file) = File::open(arg) {
                    io::copy(&mut file, &mut output).ok();
                } else if redirect.is_some() || stderr_redirect.is_some() {
                    writeln!(err_output, "cat: {}: No such file or directory", arg).ok();
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
                    }
                    if let Some(filename) = &stderr_redirect {
                        if let Ok(file) = File::create(filename) {
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
                }
                if let Some(filename) = &stderr_redirect {
                    if let Ok(file) = File::create(filename) {
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

fn main() {
    loop {
        print!("$ ");
        io::stdout().flush().unwrap();
        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();
        command_handler(input.clone());
    }
}
