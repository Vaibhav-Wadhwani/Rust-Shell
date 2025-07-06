#[allow(unused_imports)]
use std::io::{self, Write};
use std::process::exit;
use std::env;
use std::fs;
use std::path::PathBuf;

fn main() -> ! {
    loop {
        print!("$ ");
        io::stdout().flush().unwrap();

        let stdin = io::stdin();
        let mut input = String::new();
        stdin.read_line(&mut input).unwrap();

        let input = input.trim();

        let command: Vec<&str> = input.split(" ").collect();

        match command.as_slice() {
            [""] => continue,
            ["echo", args @ ..] => cmd_echo(args),
            ["type", args @ ..] => cmd_type(args),
            ["exit", "0"] => exit(0),
            _ => println!("{}: command not found", input),
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
        "type" | "echo" | "exit" => {
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