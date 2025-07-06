use std::env;
use std::fs::File;
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

fn command_handler(input: String) {
    // Tokenize input for robust redirection parsing
    let tokens: Vec<&str> = input.trim().split_whitespace().collect();
    if tokens.is_empty() {
        return;
    }
    let mut redirect = None;
    let mut cmd_tokens = tokens.as_slice();
    let mut i = 0;
    while i < tokens.len() {
        if tokens[i] == ">" || tokens[i] == "1>" {
            if i + 1 < tokens.len() {
                redirect = Some(tokens[i + 1].to_string());
                cmd_tokens = &tokens[..i];
            }
            break;
        }
        i += 1;
    }
    // If no command tokens before redirection, do nothing
    if cmd_tokens.is_empty() {
        return;
    }
    let command = cmd_tokens[0];
    let args = &cmd_tokens[1..];
    // match the cmd and execute the corresponding fn
    match command {
        "exit" => std::process::exit(
            args.get(0)
                .and_then(|s| s.parse::<i32>().ok())
                .unwrap_or(255),
        ),
        "echo" => {
            let output = args
                .iter()
                .map(|s| s.trim_matches(&['\'', '"'][..]))
                .collect::<Vec<_>>()
                .join(" ");
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
            match args[0] {
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
                Err(_e) => println!("{input}: command not found"),
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
        _ => {
            if check_for_executable(command) {
                let mut cmd = std::process::Command::new(command);
                cmd.args(args);
                if let Some(filename) = redirect {
                    if let Ok(file) = File::create(filename) {
                        cmd.stdout(file);
                    }
                }
                cmd.spawn().unwrap().wait().unwrap();
                return;
            }
            println!("{}: command not found", command);
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
