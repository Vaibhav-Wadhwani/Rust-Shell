#[allow(unused_imports)]
use std::io::{self, Write};
use std::process::exit;
use std::env;
use std::fs;
use std::path::PathBuf;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
#[cfg(windows)]
use std::os::windows::process::CommandExt;

fn main() -> ! {
    loop {
        print!("$ ");
        io::stdout().flush().unwrap();

        let stdin = io::stdin();
        let mut input = String::new();
        stdin.read_line(&mut input).unwrap();

        let input = input.trim();

        let command = process_line(input);

        match command.as_slice() {
            [] => continue,
            [cmd, args @ ..] if *cmd == "echo" => {
                cmd_echo(args);
            }
            [cmd, args @ ..] if *cmd == "type" => {
                cmd_type(args);
            }
            [cmd, args @ ..] if *cmd == "pwd" => {
                cmd_pwd();
            }
            [cmd, args @ ..] if *cmd == "cd" && args.len() == 1 => {
                cmd_cd(&args[0]);
            }
            [cmd, args @ ..] if *cmd == "exit" && args == ["0"] => {
                exit(0);
            }
            [cmd, args @ ..] => {
                if let Some(exec_path) = find_in_path(cmd) {
                    let child = std::process::Command::new(exec_path)
                        .arg0(cmd)
                        .args(args)
                        .spawn();
                    match child {
                        Ok(mut child) => {
                            let _ = child.wait();
                        },
                        Err(_) => {
                            println!("{}: command not found", input);
                        }
                    }
                } else {
                    println!("{}: command not found", input);
                }
            }
        }
    }
}

fn process_line(line: &str) -> Vec<String> {
    let mut single = false;
    let mut double = false;
    let mut groups = Vec::new();
    let mut cur = String::new();

    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if single {
            match ch {
                '\'' => single = false,
                _ => cur.push(ch),
            };
        } else if double {
            match ch {
                '"' => double = false,
                '\\' => {
                    if let Some(&ch_next) = chars.peek() {
                        if ch_next == '\\' || ch_next == '"' || ch_next == '$' {
                            chars.next();
                            cur.push(ch_next);
                        } else {
                            cur.push('\\');
                            cur.push(ch_next);
                            chars.next();
                        }
                    }
                }
                _ => cur.push(ch),
            };
        } else {
            match ch {
                '\'' => single = true,
                '"' => double = true,
                '\\' => {
                    if let Some(&ch_next) = chars.peek() {
                        match ch_next {
                            ' ' | '\t' | '\n' | '\\' | '\'' => {
                                chars.next();
                                cur.push(ch_next);
                            }
                            _ => {
                                cur.push('\\');
                                cur.push(ch_next);
                                chars.next();
                            }
                        }
                    }
                }
                ch if ch.is_whitespace() => {
                    if !cur.is_empty() {
                        groups.push(cur);
                        cur = String::new();
                    }
                }
                _ => cur.push(ch),
            };
        }
    }

    if !cur.is_empty() {
        groups.push(cur);
    }

    groups
}

fn cmd_echo(args: &[String]) {
    println!("{}", args.join(" "));
}

fn cmd_type(args: &[String]) {
    use std::env;
    use std::fs;
    use std::path::PathBuf;

    let args_len = args.len();

    if args_len == 0 {
        return;
    }

    if args_len > 1 {
        println!("type: too many arguments");
        return;
    }

    let cmd = args[0].clone();

    // Check for builtins
    match cmd.as_str() {
        "type" | "echo" | "exit" | "pwd" => {
            println!("{} is a shell builtin", cmd);
            return;
        },
        _ => {}
    }

    // Search PATH for executable
    if let Ok(path_var) = env::var("PATH") {
        for dir in env::split_paths(&path_var) {
            let mut candidate = PathBuf::from(&dir);
            candidate.push(&cmd);
            if candidate.exists() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Ok(metadata) = fs::metadata(&candidate) {
                        let perm = metadata.permissions().mode();
                        if (perm & 0o111) != 0 {
                            println!("{} is {}", cmd, candidate.display());
                            return;
                        }
                    }
                }
                #[cfg(windows)]
                {
                    let exts = ["", ".exe", ".bat", ".cmd"];
                    for ext in &exts {
                        let mut candidate_with_ext = candidate.clone();
                        if !ext.is_empty() {
                            candidate_with_ext.set_extension(ext.trim_start_matches('.'));
                        }
                        if candidate_with_ext.exists() {
                            println!("{} is {}", cmd, candidate_with_ext.display());
                            return;
                        }
                    }
                }
            }
        }
    }

    println!("{}: not found", cmd);
}

fn find_in_path(cmd: &str) -> Option<std::path::PathBuf> {
    use std::env;
    use std::fs;
    use std::path::PathBuf;

    if let Ok(path_var) = env::var("PATH") {
        for dir in env::split_paths(&path_var) {
            let mut candidate = PathBuf::from(&dir);
            candidate.push(cmd);
            if candidate.exists() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Ok(metadata) = fs::metadata(&candidate) {
                        let perm = metadata.permissions().mode();
                        if (perm & 0o111) != 0 {
                            return Some(candidate);
                        }
                    }
                }
                #[cfg(windows)]
                {
                    let exts = ["", ".exe", ".bat", ".cmd"];
                    for ext in &exts {
                        let mut candidate_with_ext = candidate.clone();
                        if !ext.is_empty() {
                            candidate_with_ext.set_extension(ext.trim_start_matches('.'));
                        }
                        if candidate_with_ext.exists() {
                            return Some(candidate_with_ext);
                        }
                    }
                }
            }
        }
    }
    None
}

fn cmd_pwd() {
    match std::env::current_dir() {
        Ok(path) => println!("{}", path.display()),
        Err(_) => println!("pwd: failed to get current directory"),
    }
}

fn cmd_cd(path: &str) {
    let target = if path == "~" {
        match std::env::var("HOME") {
            Ok(home) => home,
            Err(_) => {
                println!("cd: HOME not set");
                return;
            }
        }
    } else {
        path.to_string()
    };
    if let Err(_) = std::env::set_current_dir(&target) {
        println!("cd: {}: No such file or directory", path);
    }
}