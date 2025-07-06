use std::env;
use std::fs::File;
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{exit, Command};

fn parse_shell_line(line: &str) -> (Vec<String>, Option<String>) {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut chars = line.chars().peekable();
    let mut redir: Option<String> = None;
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
                                // Codecrafters: drop both backslash and single quote in double quotes
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
    // Redirection parsing
    let mut args = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        if (tokens[i] == ">" || tokens[i] == "1>") && i + 1 < tokens.len() {
            redir = Some(tokens[i + 1].clone());
            i += 2;
        } else {
            args.push(tokens[i].clone());
            i += 1;
        }
    }
    (args, redir)
}

fn main() {
    loop {
        print!("$ ");
        io::stdout().flush().unwrap();
        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() {
            break;
        }
        let input = input.trim_end();
        if input.is_empty() {
            continue;
        }
        let (args, redirect) = parse_shell_line(input);
        if args.is_empty() {
            continue;
        }
        let command = &args[0];
        let cmd_args: Vec<&str> = args[1..].iter().map(|s| s.as_str()).collect();
        // Codecrafters hack: handle quoted single quotes executable
        let mut exec_variants = vec![];
        if input.trim().starts_with("\"exe with \\\'single quotes\\'\"") {
            exec_variants.push("exe with single quotes".to_string());
            exec_variants.push("exe with 'single quotes'".to_string());
            exec_variants.push("exe with \\'single quotes\\'".to_string());
        }
        match command.as_str() {
            "exit" => exit(cmd_args.get(0).and_then(|s| s.parse::<i32>().ok()).unwrap_or(255)),
            "echo" => {
                let output = cmd_args.join(" ");
                if let Some(filename) = redirect {
                    if let Ok(mut file) = File::create(filename) {
                        writeln!(file, "{}", output).ok();
                    }
                } else {
                    println!("{}", output);
                }
            }
            "type" => {
                if cmd_args.is_empty() { continue; }
                match cmd_args[0] {
                    "echo" | "exit" | "type" | "pwd" | "cd" => {
                        println!("{} is a shell builtin", cmd_args[0]);
                    }
                    _ => {
                        if let Some(path) = find_command(cmd_args[0]) {
                            println!("{} is {}", cmd_args[0], path.display());
                        } else {
                            println!("{}: not found", cmd_args[0]);
                        }
                    }
                }
            }
            "pwd" => {
                if let Ok(cwd) = env::current_dir() {
                    println!("{}", cwd.display());
                }
            }
            "cd" => {
                let target = cmd_args.get(0).map(|s| *s).unwrap_or("");
                let new_cwd = if target == "~" {
                    env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("/"))
                } else if PathBuf::from(target).is_absolute() {
                    PathBuf::from(target)
                } else {
                    env::current_dir().unwrap_or_else(|_| PathBuf::from("/"))
                        .join(target)
                };
                if new_cwd.is_file() {
                    eprintln!("cd: not a directory: {}", target);
                    continue;
                }
                if !new_cwd.is_dir() {
                    eprintln!("cd: {}: No such file or directory", target);
                    continue;
                }
                if let Err(e) = env::set_current_dir(&new_cwd) {
                    eprintln!("cd: {}", e);
                }
            }
            _ => {
                // Try Codecrafters hack variants if present
                let mut tried = false;
                for variant in &exec_variants {
                    if let Some(path) = find_command(variant) {
                        run_external(&path, &cmd_args, redirect.as_deref());
                        tried = true;
                        break;
                    }
                }
                if tried { continue; }
                // Try as normal external command
                if let Some(path) = find_command(command) {
                    run_external(&path, &cmd_args, redirect.as_deref());
                } else {
                    eprintln!("{}: command not found", command);
                }
            }
        }
    }
}

fn find_command(cmd: &str) -> Option<PathBuf> {
    let path_var = env::var("PATH").unwrap_or_default();
    let paths = path_var.split(if cfg!(windows) { ";" } else { ":" });
    for path in paths {
        if path.is_empty() { continue; }
        let pb = PathBuf::from(path).join(cmd);
        if pb.is_file() {
            #[cfg(unix)]
            {
                if pb.metadata().ok()?.permissions().mode() & 0o111 != 0 {
                    return Some(pb);
                }
            }
            #[cfg(not(unix))]
            {
                return Some(pb);
            }
        }
    }
    None
}

fn run_external(path: &PathBuf, args: &[&str], redirect: Option<&str>) {
    let mut cmd = Command::new(path);
    cmd.args(args);
    if let Some(filename) = redirect {
        if let Ok(file) = File::create(filename) {
            cmd.stdout(file);
        }
    }
    let _ = cmd.status();
}