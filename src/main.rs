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

        let command: Vec<&str> = input.split_whitespace().collect();

        match command.as_slice() {
            &[] => continue,
            ["echo", args @ ..] => cmd_echo(args),
            ["type", args @ ..] => cmd_type(args),
            ["pwd"] => cmd_pwd(),
            ["exit", "0"] => exit(0),
            [cmd, args @ ..] => {
                // Try to run as external command
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

fn cmd_echo(args: &[&str]) {
    println!("{}", args.join(" "));
}

fn cmd_type(args: &[&str]) {
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

    let cmd = args[0];

    // Check for builtins
    match cmd {
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
            candidate.push(cmd);
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